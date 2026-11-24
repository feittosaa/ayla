import { useEffect, useRef, useState } from 'react';
import { errorText, formatBytes, native, nativeTask, taskId } from '../services/Desktop';

export type UpscaleTarget = { file?: string; root?: string | null; path?: string; name: string; src: string };
type Status = { installed: boolean; bytes: number; upscayl?: { id: string; label: string }[] };
export type UpscaleResult = { file?: string; root?: string; path?: string; name: string };

/** Ampliar a imagem aberta (Upscayl já instalado ou Real-ESRGAN, download opcional). O original não muda: sai uma cópia ao lado. */
export default function IrisUpscale({ target, onDone, onClose }: { target: UpscaleTarget; onDone: (result: UpscaleResult) => void; onClose: () => void }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [model, setModel] = useState('foto');
  const [scale, setScale] = useState(4);
  const [progress, setProgress] = useState<string | null>(null);
  const [error, setError] = useState('');
  const job = useRef<string | null>(null);
  // Tamanho original (lido da própria imagem; o WebKit já aplica a orientação da foto).
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  useEffect(() => {
    const image = new Image();
    image.onload = () => setSize({ width: image.naturalWidth, height: image.naturalHeight });
    image.src = target.src;
    return () => { image.onload = null; };
  }, [target.src]);
  // O núcleo amplia sempre em 4x antes de reduzir: o lado maior vai até 4096 px.
  const tooBig = size !== null && Math.max(size.width, size.height) > 4096;
  useEffect(() => {
    void native<Status>('iris_upscaler').then(found => {
      setStatus(found);
      // Com o Upscayl instalado, começa pelo modelo padrão dele.
      const upscayl = found.upscayl ?? [];
      if (upscayl.length) setModel((upscayl.find(m => m.id === 'upscayl:upscayl-standard-4x') ?? upscayl[0]).id);
    }).catch(e => setError(errorText(e)));
  }, []);
  // Fechar no meio: interrompe o download ou a ampliação.
  useEffect(() => () => { if (job.current) void native('cancel_job', { requestId: job.current }); }, []);
  async function install() {
    setError(''); const requestId = taskId(); job.current = requestId; setProgress('Baixando…');
    try {
      await nativeTask('install_runtime', { id: 'upscale', requestId }, event => { if (event.type === 'download') setProgress(`Baixando ${formatBytes(event.received || 0)} de ${formatBytes(event.total || 0)}`); });
      setStatus(s => s && { ...s, installed: true });
    } catch (e) { setError(errorText(e)); } finally { job.current = null; setProgress(null); }
  }
  async function run() {
    setError(''); const requestId = taskId(); job.current = requestId; setProgress('Fazendo upscale… 0%');
    try {
      const result = await nativeTask<UpscaleResult>('iris_upscale', { file: target.file ?? null, root: target.file ? null : target.root ?? null, path: target.file ? null : target.path ?? null, modelo: model, scale, requestId },
        event => { if (event.type === 'progress') setProgress(`Fazendo upscale… ${Math.floor(event.percent ?? 0)}%`); });
      job.current = null; onDone(result);
    } catch (e) { setError(errorText(e)); } finally { job.current = null; setProgress(null); }
  }
  const busy = progress !== null;
  return <div className="iris-upscale" role="dialog" aria-label="Upscale de imagem" onClick={e => e.stopPropagation()}>
    <b>Upscale de “{target.name}”</b>
    {!status ? <span>Verificando o upscale…</span>
      : !status.installed && !status.upscayl?.length ? <>
          <span>O upscale é um download opcional de {formatBytes(status.bytes)} (Real-ESRGAN, roda na placa de vídeo). Baixa uma vez e fica pronto.</span>
          <div className="row-actions">
            {busy ? <button className="ghost-button" onClick={() => { if (job.current) void native('cancel_job', { requestId: job.current }); }}>Interromper</button>
              : <button className="primary-action" onClick={() => void install()}>Baixar o upscale</button>}
            <button className="ghost-button" disabled={busy} onClick={onClose}>Cancelar</button>
          </div>
        </>
      : <>
          <div className="iris-upscale-options">
            <label>Modelo<select value={model} disabled={busy} onChange={e => setModel(e.target.value)}>
              {!!status.upscayl?.length && <optgroup label="Upscayl (instalado no computador)">{status.upscayl.map(m => <option key={m.id} value={m.id}>{m.label}</option>)}</optgroup>}
              {status.installed && <optgroup label="Real-ESRGAN"><option value="foto">Foto</option><option value="anime">Ilustração e anime</option></optgroup>}
            </select></label>
            <div className="iris-upscale-scale" role="group" aria-label="Escala">{[2, 3, 4].map(n => <button key={n} type="button" aria-pressed={scale === n} disabled={busy} onClick={() => setScale(n)}>{n}x</button>)}</div>
          </div>
          {size && <span className="iris-upscale-size">{size.width}×{size.height} px → <b>{size.width * scale}×{size.height * scale} px</b></span>}
          {tooBig && <span className="iris-upscale-error">O upscale aceita imagens de até 4096 px de lado.</span>}
          <span className="iris-upscale-note">Salva uma cópia ao lado ({target.name.replace(/\.[^.]+$/, '')}-ampliada-{scale}x); o original não muda.</span>
          <div className="row-actions">
            {busy ? <button className="ghost-button" onClick={() => { if (job.current) void native('cancel_job', { requestId: job.current }); }}>Interromper</button>
              : <button className="primary-action" disabled={tooBig} onClick={() => void run()}>Fazer upscale</button>}
            <button className="ghost-button" disabled={busy} onClick={onClose}>Fechar</button>
          </div>
        </>}
    {progress && <span role="status">{progress}</span>}
    {error && <span role="alert" className="iris-upscale-error">{error}</span>}
  </div>;
}
