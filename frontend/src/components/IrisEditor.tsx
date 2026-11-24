import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react';
import { errorText, formatBytes, native, nativeTask, taskId } from '../services/Desktop';
import { aspects, clock, cropPixels, dragCrop, fitCrop, fractionRatio, isFull, rotated, type Aspect, type Crop, type Handle } from '../services/Editor';
import { CloseIcon } from './Icons';
import IrisVideoStrip from './IrisVideoStrip';

export type EditTarget = { file?: string; root?: string | null; path?: string; name: string; kind: 'image' | 'video'; src: string };
export type EditResult = { file?: string; root?: string; path?: string; name: string; replaced?: boolean };
type Tools = { ffmpeg: boolean; ffmpeg_bytes: number; replaceable: boolean };

const RotateIcon = ({ left }: { left?: boolean }) => <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={left ? { transform: 'scaleX(-1)' } : undefined}><path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" /></svg>;
const FlipIcon = ({ vertical }: { vertical?: boolean }) => <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={vertical ? { transform: 'rotate(90deg)' } : undefined}><path d="M12 3v18" strokeDasharray="2 3" /><path d="M9 6 3 18h6zM15 6l6 12h-6z" /></svg>;

/** Mini editor da Iris. Imagem: girar, espelhar, recortar, redimensionar, brilho, contraste e
 * saturação (sem download). Vídeo: cortar início e fim, girar, recortar o quadro e tirar o som, em
 * MP4 (precisa do FFmpeg, download opcional). Salva uma cópia ao lado; substituir o original pede
 * confirmação. A prévia usa as mesmas regras do núcleo, então o arquivo sai como se vê. */
export default function IrisEditor({ target, onDone, onClose }: { target: EditTarget; onDone: (result: EditResult) => void; onClose: () => void }) {
  const video = target.kind === 'video';
  const [tools, setTools] = useState<Tools | null>(null);
  const [natural, setNatural] = useState<{ width: number; height: number } | null>(null);
  const [rotate, setRotate] = useState(0);
  const [flipH, setFlipH] = useState(false);
  const [flipV, setFlipV] = useState(false);
  const [aspect, setAspect] = useState<Aspect>('livre');
  const [crop, setCrop] = useState<Crop | null>(null);
  const [resizeW, setResizeW] = useState<number | null>(null);
  const [resizeH, setResizeH] = useState<number | null>(null);
  const [lock, setLock] = useState(true);
  const [tone, setTone] = useState({ brightness: 1, contrast: 1, saturation: 1 });
  const [duration, setDuration] = useState(0);
  const [range, setRange] = useState<[number, number]>([0, 0]);
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [mute, setMute] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [confirmReplace, setConfirmReplace] = useState(false);
  const [box, setBox] = useState({ width: 0, height: 0 });
  const stage = useRef<HTMLDivElement>(null);
  const player = useRef<HTMLVideoElement>(null);
  const job = useRef<string | null>(null);
  const drag = useRef<{ handle: Handle; x: number; y: number; start: Crop } | null>(null);
  const media = { file: target.file ?? null, root: target.file ? null : target.root ?? null, path: target.file ? null : target.path ?? null };

  useEffect(() => { void native<Tools>('iris_editor', media).then(setTools).catch(e => setError(errorText(e))); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  // Fechar no meio: interrompe o download ou a exportação.
  useEffect(() => () => { if (job.current) void native('cancel_job', { requestId: job.current }); }, []);
  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    // Medida já na abertura (o ResizeObserver só avisa no próximo quadro pintado).
    setBox({ width: element.clientWidth, height: element.clientHeight });
    const observer = new ResizeObserver(([entry]) => setBox({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const frame = natural ? rotated(natural.width, natural.height, rotate) : null;
  const k = frame && box.width && box.height ? Math.min(box.width / frame.width, box.height / frame.height) : 0;
  const shown = frame ? { width: frame.width * k, height: frame.height * k } : { width: 0, height: 0 };
  // Sem recorte é o quadro inteiro: as bordas já podem ser arrastadas.
  const area = crop ?? { x: 0, y: 0, width: 1, height: 1 };
  const ratio = frame ? fractionRatio(aspect, frame.width, frame.height) : null;
  const cut = frame ? cropPixels(crop, frame.width, frame.height) : { width: 0, height: 0 };
  const finalSize = resizeW === null ? cut : { width: resizeW, height: lock ? Math.max(1, Math.round(resizeW * cut.height / cut.width)) : resizeH ?? cut.height };
  const toned = tone.brightness !== 1 || tone.contrast !== 1 || tone.saturation !== 1;
  const trimmed = video && duration > 0 && (range[0] > 0.05 || range[1] < duration - 0.05);
  const changed = rotate !== 0 || flipH || flipV || !isFull(crop) || (resizeW !== null && (finalSize.width !== cut.width || finalSize.height !== cut.height)) || toned || trimmed || mute;
  const busy = progress !== null;

  function chooseAspect(next: Aspect) {
    setAspect(next); setResizeW(null); setResizeH(null);
    setCrop(frame && next !== 'livre' ? fitCrop(next, frame.width, frame.height) : null);
  }
  function turn(delta: number) {
    const next = (rotate + delta + 360) % 360;
    setRotate(next); setResizeW(null); setResizeH(null);
    // O quadro muda de forma: o recorte recomeça na mesma proporção.
    if (aspect !== 'livre' && natural) { const f = rotated(natural.width, natural.height, next); setCrop(fitCrop(aspect, f.width, f.height)); } else setCrop(null);
  }
  function startDrag(handle: Handle, event: ReactPointerEvent) {
    if (!frame || busy) return;
    event.preventDefault(); event.stopPropagation();
    try { (event.currentTarget as Element).setPointerCapture(event.pointerId); } catch { /* ponteiro já solto */ }
    drag.current = { handle, x: event.clientX, y: event.clientY, start: area };
  }
  function moveDrag(event: ReactPointerEvent) {
    const current = drag.current;
    if (!current || !shown.width) return;
    setCrop(dragCrop(current.start, current.handle, (event.clientX - current.x) / shown.width, (event.clientY - current.y) / shown.height, ratio));
  }

  // Vídeo: toca só o trecho escolhido.
  function seek(seconds: number) { if (player.current) { player.current.currentTime = seconds; setTime(seconds); } }
  function toggle() {
    const v = player.current;
    if (!v) return;
    if (v.paused) { if (v.currentTime >= range[1] - 0.05 || v.currentTime < range[0]) v.currentTime = range[0]; void v.play(); } else v.pause();
  }

  async function save(replace: boolean) {
    setError(''); setConfirmReplace(false);
    try {
      let result: EditResult;
      if (video) {
        const requestId = taskId(); job.current = requestId; setProgress('Exportando… 0%');
        const edit = { start: range[0] > 0.05 ? range[0] : 0, end: range[1] < duration - 0.05 ? range[1] : null, duration, rotate, crop: isFull(crop) ? null : crop, mute };
        result = await nativeTask<EditResult>('iris_edit_video', { ...media, edit, replace, requestId },
          event => { if (event.type === 'progress') setProgress(`Exportando… ${Math.floor(event.percent ?? 0)}%`); });
      } else {
        setProgress('Salvando…');
        const resize = resizeW !== null && (finalSize.width !== cut.width || finalSize.height !== cut.height);
        const edit = { rotate, flip_h: flipH, flip_v: flipV, crop: isFull(crop) ? null : crop, width: resize ? finalSize.width : null, height: resize ? finalSize.height : null, ...tone };
        result = await native<EditResult>('iris_edit_image', { ...media, edit, replace });
      }
      job.current = null; setProgress(null); onDone(result);
    } catch (e) { setError(errorText(e)); setProgress(null); } finally { job.current = null; }
  }
  async function installFfmpeg() {
    setError(''); const requestId = taskId(); job.current = requestId; setProgress('Baixando…');
    try {
      await nativeTask('install_runtime', { id: 'ffmpeg', requestId }, event => { if (event.type === 'download') setProgress(`Baixando ${formatBytes(event.received || 0)} de ${formatBytes(event.total || 0)}`); });
      setTools(t => t && { ...t, ffmpeg: true });
    } catch (e) { setError(errorText(e)); } finally { job.current = null; setProgress(null); }
  }

  // Esc fecha o editor (a confirmação de substituir primeiro).
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || busy) return;
      event.stopImmediatePropagation();
      if (confirmReplace) setConfirmReplace(false); else onClose();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [busy, confirmReplace, onClose]);

  const transform = `translate(-50%, -50%) scale(${flipH ? -1 : 1}, ${flipV ? -1 : 1}) rotate(${rotate}deg)`;
  const mediaStyle: CSSProperties = natural ? { width: natural.width * k, height: natural.height * k, transform, filter: toned ? `brightness(${tone.brightness}) contrast(${tone.contrast}) saturate(${tone.saturation})` : undefined } : { opacity: 0 };
  const percent = (v: number) => `${Math.round(v * 100)}%`;
  const needsFfmpeg = video && tools && !tools.ffmpeg;

  return <div className="iris-editor" role="dialog" aria-modal="true" aria-label={`Editar ${target.name}`} onClick={e => e.stopPropagation()}>
    <header className="iris-viewer-bar">
      <span className="iris-viewer-title"><b>Editar “{target.name}”</b><small>{frame ? `${finalSize.width}×${finalSize.height} px` : 'Carregando…'}{video && duration ? ` · ${clock(range[1] - range[0])}` : ''}</small></span>
      {needsFfmpeg ? null : <>
        {tools?.replaceable && <button className="ghost-button iris-outline-button" disabled={!changed || busy || confirmReplace} onClick={() => setConfirmReplace(true)}>Substituir original</button>}
        <button className="primary-action" disabled={!changed || busy || !frame} onClick={() => void save(false)}>Salvar cópia</button>
      </>}
      <button className="icon-button tip-end" data-tip="Fechar o editor (Esc)" aria-label="Fechar o editor" disabled={busy} onClick={onClose}><CloseIcon size={18} /></button>
    </header>
    {confirmReplace && <div className="iris-trash-confirm" role="group" aria-label="Confirmar substituição">
      <span>Substituir “{target.name}” pela versão editada? O original não volta.</span>
      <button className="ghost-button" autoFocus onClick={() => setConfirmReplace(false)}>Cancelar</button>
      <button className="ghost-button" onClick={() => void save(true)}>Substituir</button>
    </div>}
    <div className="iris-editor-body">
      <div className="iris-editor-main">
      <div className="iris-editor-stage" ref={stage}>
        <div className="iris-editor-frame" style={{ width: shown.width, height: shown.height }}>
          {video
            ? <video ref={player} src={target.src} style={mediaStyle} muted={mute} preload="auto" playsInline
                onLoadedMetadata={e => { const v = e.currentTarget; setNatural({ width: v.videoWidth, height: v.videoHeight }); setDuration(v.duration); setRange([0, v.duration]); }}
                onTimeUpdate={e => { const t = e.currentTarget.currentTime; setTime(t); if (t >= range[1] && !e.currentTarget.paused) { e.currentTarget.pause(); } }}
                onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onError={() => setError('O vídeo não abriu na prévia.')} />
            : <img src={target.src} alt="" style={mediaStyle} draggable={false} onLoad={e => setNatural({ width: e.currentTarget.naturalWidth, height: e.currentTarget.naturalHeight })} onError={() => setError('A imagem não abriu na prévia.')} />}
          {frame && <div className={isFull(crop) ? 'iris-crop full' : 'iris-crop'} style={{ left: area.x * shown.width, top: area.y * shown.height, width: area.width * shown.width, height: area.height * shown.height }}
            onPointerDown={e => startDrag('move', e)} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null; }}>
            {(['n', 's', 'e', 'w', 'nw', 'ne', 'sw', 'se'] as const).map(h => <span key={h} className={`iris-crop-handle ${h}`} onPointerDown={e => startDrag(h, e)} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null; }} />)}
          </div>}
        </div>
      </div>
      {video && duration > 0 && <div className="iris-editor-timeline">
        <div className="iris-editor-row">
          <button className="ghost-button iris-outline-button" disabled={busy} onClick={toggle}>{playing ? 'Pausar' : 'Tocar'}</button>
          <span className="iris-editor-time">{clock(time)}</span>
        </div>
        <IrisVideoStrip src={target.src} duration={duration} ratio={frame ? frame.width / frame.height : 16 / 9} range={range} time={time} disabled={busy}
          onRange={setRange} onSeek={seconds => { player.current?.pause(); seek(seconds); }} />
      </div>}
      </div>
      <aside className="iris-editor-panel">
        <section>
          <h3>Girar{video ? '' : ' e espelhar'}</h3>
          <div className="iris-editor-row">
            <button className="icon-button" data-tip="Girar para a esquerda" aria-label="Girar para a esquerda" disabled={busy} onClick={() => turn(-90)}><RotateIcon left /></button>
            <button className="icon-button" data-tip="Girar para a direita" aria-label="Girar para a direita" disabled={busy} onClick={() => turn(90)}><RotateIcon /></button>
            {!video && <>
              <button className={flipH ? 'icon-button active' : 'icon-button'} data-tip="Espelhar na horizontal" aria-label="Espelhar na horizontal" aria-pressed={flipH} disabled={busy} onClick={() => setFlipH(v => !v)}><FlipIcon /></button>
              <button className={flipV ? 'icon-button active' : 'icon-button'} data-tip="Espelhar na vertical" aria-label="Espelhar na vertical" aria-pressed={flipV} disabled={busy} onClick={() => setFlipV(v => !v)}><FlipIcon vertical /></button>
            </>}
          </div>
        </section>
        <section>
          <h3>Recortar</h3>
          <div className="iris-editor-choices" role="group" aria-label="Proporção do recorte">
            {aspects.map(a => <button key={a.id} type="button" aria-pressed={aspect === a.id} disabled={busy || !frame} onClick={() => chooseAspect(a.id)}>{a.label}</button>)}
          </div>
          <span className="iris-upscale-note">Arraste as bordas ou os cantos para recortar; arraste o meio para mover.</span>
          {!isFull(crop) && <button className="ghost-button" disabled={busy} onClick={() => chooseAspect('livre')}>Desfazer recorte</button>}
        </section>
        {!video && <section>
          <h3>Tamanho</h3>
          <div className="iris-editor-row">
            <label>Largura<input type="number" min={1} max={16384} value={finalSize.width || ''} disabled={busy || !frame}
              onChange={e => { const v = Math.round(Number(e.target.value)); if (v > 0) { setResizeW(Math.min(16384, v)); if (!lock && resizeH === null) setResizeH(finalSize.height); } }} /></label>
            <label>Altura<input type="number" min={1} max={16384} value={finalSize.height || ''} disabled={busy || !frame}
              onChange={e => { const v = Math.round(Number(e.target.value)); if (v <= 0) return; if (lock) setResizeW(Math.max(1, Math.round(v * cut.width / cut.height))); else { setResizeW(finalSize.width); setResizeH(Math.min(16384, v)); } }} /></label>
          </div>
          <label className="iris-editor-check"><input type="checkbox" checked={lock} disabled={busy} onChange={e => { setLock(e.target.checked); setResizeH(null); }} />Manter a proporção</label>
          {resizeW !== null && <button className="ghost-button" disabled={busy} onClick={() => { setResizeW(null); setResizeH(null); }}>Tamanho do recorte</button>}
        </section>}
        {!video && <section>
          <h3>Ajustes</h3>
          {([['brightness', 'Brilho'], ['contrast', 'Contraste'], ['saturation', 'Saturação']] as const).map(([id, label]) =>
            <label key={id} className="iris-editor-slider">{label}<span>{percent(tone[id])}</span>
              <input type="range" min={0} max={2} step={0.01} value={tone[id]} disabled={busy} onChange={e => setTone(t => ({ ...t, [id]: Number(e.target.value) }))} onDoubleClick={() => setTone(t => ({ ...t, [id]: 1 }))} /></label>)}
          {toned && <button className="ghost-button" disabled={busy} onClick={() => setTone({ brightness: 1, contrast: 1, saturation: 1 })}>Restaurar ajustes</button>}
        </section>}
        {video && duration > 0 && <section>
          <h3>Trecho</h3>
          <span className="iris-upscale-note">Arraste as pontas da faixa embaixo do vídeo para cortar o começo e o fim.</span>
          <div className="iris-editor-row">
            <button className="ghost-button" disabled={busy || time >= range[1] - 0.1} onClick={() => setRange([time, range[1]])}>Início aqui</button>
            <button className="ghost-button" disabled={busy || time <= range[0] + 0.1} onClick={() => setRange([range[0], time])}>Fim aqui</button>
          </div>
          <label className="iris-editor-check"><input type="checkbox" checked={mute} disabled={busy} onChange={e => setMute(e.target.checked)} />Tirar o som</label>
        </section>}
        {needsFfmpeg && <section>
          <span className="iris-upscale-note">Para exportar vídeos, o editor usa o FFmpeg, um download opcional de {formatBytes(tools.ffmpeg_bytes)} (o mesmo que lê vídeos na conversa).</span>
          {busy ? <button className="ghost-button" onClick={() => { if (job.current) void native('cancel_job', { requestId: job.current }); }}>Interromper</button>
            : <button className="primary-action" onClick={() => void installFfmpeg()}>Baixar o FFmpeg</button>}
        </section>}
        <span className="iris-upscale-note">Salva uma cópia ao lado ({target.name.replace(/\.[^.]+$/, '')}-editada{video ? '.mp4' : ''}); o original não muda.{tools && !tools.replaceable ? video ? ' Só vídeos MP4 podem ser substituídos.' : ' Este formato não pode ser substituído.' : ''}</span>
        {progress && <span role="status">{progress}{video && !needsFfmpeg && <button className="ghost-button" onClick={() => { if (job.current) void native('cancel_job', { requestId: job.current }); }}>Interromper</button>}</span>}
        {error && <span role="alert" className="iris-upscale-error">{error}</span>}
      </aside>
    </div>
  </div>;
}
