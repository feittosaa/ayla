import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { clock } from '../services/Editor';

const HEIGHT = 56;
const MIN_GAP = 0.1;

/** Faixa de quadros do vídeo embaixo da prévia: as alças das pontas cortam o trecho, clicar ou
 * arrastar no meio leva a reprodução para aquele ponto. Os quadros são desenhados no próprio
 * WebKit por um segundo vídeo escondido (sem FFmpeg). */
export default function IrisVideoStrip({ src, duration, ratio, range, time, disabled, onRange, onSeek }: {
  src: string; duration: number; ratio: number; range: [number, number]; time: number; disabled?: boolean;
  onRange: (range: [number, number]) => void; onSeek: (seconds: number) => void;
}) {
  const strip = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const canvases = useRef<(HTMLCanvasElement | null)[]>([]);
  const drag = useRef<'start' | 'end' | 'seek' | null>(null);

  useEffect(() => {
    const element = strip.current;
    if (!element) return;
    setWidth(element.clientWidth);
    const observer = new ResizeObserver(([entry]) => setWidth(Math.round(entry.contentRect.width)));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const frameWidth = Math.max(24, HEIGHT * (ratio || 16 / 9));
  const count = width ? Math.max(1, Math.ceil(width / frameWidth)) : 0;

  // Desenha um quadro por vez, do começo ao fim (cada um no meio do seu pedaço do vídeo).
  useEffect(() => {
    if (!count || !duration) return;
    let stopped = false;
    const probe = document.createElement('video');
    probe.muted = true; probe.preload = 'auto'; probe.src = src;
    // No WebKitGTK, um vídeo fora da página não entrega quadros ao canvas: fica nela, invisível.
    probe.setAttribute('aria-hidden', 'true');
    probe.style.cssText = 'position:fixed;left:0;top:0;width:2px;height:2px;opacity:0;pointer-events:none';
    document.body.appendChild(probe);
    const at = (seconds: number) => new Promise<void>(resolve => {
      const done = () => { probe.removeEventListener('seeked', done); resolve(); };
      probe.addEventListener('seeked', done);
      probe.currentTime = seconds;
    });
    const run = async () => {
      for (let i = 0; i < count && !stopped; i++) {
        await at(Math.min(duration - 0.05, (i + 0.5) * duration / count));
        const canvas = canvases.current[i];
        if (stopped || !canvas || !probe.videoWidth) continue;
        const context = canvas.getContext('2d');
        if (!context) continue;
        // Preenche o quadro sem distorcer (corta as sobras, como um "cover").
        const scale = Math.max(canvas.width / probe.videoWidth, canvas.height / probe.videoHeight);
        const w = probe.videoWidth * scale, h = probe.videoHeight * scale;
        context.drawImage(probe, (canvas.width - w) / 2, (canvas.height - h) / 2, w, h);
      }
    };
    probe.addEventListener('loadeddata', () => void run(), { once: true });
    // O decodificador às vezes recusa abrir enquanto o vídeo principal abre: tenta de novo.
    let retries = 0;
    probe.addEventListener('error', () => {
      if (stopped || retries++ >= 3) return;
      setTimeout(() => { if (!stopped) { probe.src = src; probe.load(); } }, 500 * retries);
    });
    return () => { stopped = true; probe.removeAttribute('src'); probe.load(); probe.remove(); };
  }, [src, count, duration]);

  const secondsAt = (clientX: number) => {
    const box = strip.current?.getBoundingClientRect();
    if (!box || !box.width) return 0;
    return Math.min(duration, Math.max(0, (clientX - box.left) / box.width * duration));
  };
  function begin(kind: 'start' | 'end' | 'seek', event: ReactPointerEvent) {
    if (disabled) return;
    event.preventDefault(); event.stopPropagation();
    try { (event.currentTarget as Element).setPointerCapture(event.pointerId); } catch { /* ponteiro já solto */ }
    drag.current = kind;
    move(event);
  }
  function move(event: ReactPointerEvent) {
    const kind = drag.current;
    if (!kind) return;
    const t = secondsAt(event.clientX);
    if (kind === 'start') { const start = Math.min(t, range[1] - MIN_GAP); onRange([start, range[1]]); onSeek(start); }
    else if (kind === 'end') { const end = Math.max(t, range[0] + MIN_GAP); onRange([range[0], end]); onSeek(end); }
    else onSeek(Math.min(range[1], Math.max(range[0], t)));
  }
  const end = () => { drag.current = null; };
  const pct = (seconds: number) => `${duration ? seconds / duration * 100 : 0}%`;

  return <div className="iris-strip-wrap">
    <div className="iris-strip-labels">
      <span>Início <b>{clock(range[0])}</b></span>
      <span>Trecho <b>{clock(range[1] - range[0])}</b> de {clock(duration)}</span>
      <span>Fim <b>{clock(range[1])}</b></span>
    </div>
    <div className={disabled ? 'iris-strip disabled' : 'iris-strip'} ref={strip} style={{ height: HEIGHT }}
      onPointerDown={e => begin('seek', e)} onPointerMove={move} onPointerUp={end} onPointerCancel={end}>
      <div className="iris-strip-frames" aria-hidden="true">
        {Array.from({ length: count }, (_, i) => <canvas key={`${count}-${i}`} ref={el => { canvases.current[i] = el; }} width={Math.round(frameWidth)} height={HEIGHT} style={{ width: frameWidth, height: HEIGHT }} />)}
      </div>
      <div className="iris-strip-shade" style={{ left: 0, width: pct(range[0]) }} />
      <div className="iris-strip-shade" style={{ left: pct(range[1]), right: 0 }} />
      <div className="iris-strip-range" style={{ left: pct(range[0]), width: pct(range[1] - range[0]) }}>
        <span className="iris-strip-handle start" role="slider" aria-label="Início do trecho" aria-valuemin={0} aria-valuemax={duration} aria-valuenow={range[0]} aria-valuetext={clock(range[0])}
          onPointerDown={e => begin('start', e)} onPointerMove={move} onPointerUp={end} onPointerCancel={end} />
        <span className="iris-strip-handle end" role="slider" aria-label="Fim do trecho" aria-valuemin={0} aria-valuemax={duration} aria-valuenow={range[1]} aria-valuetext={clock(range[1])}
          onPointerDown={e => begin('end', e)} onPointerMove={move} onPointerUp={end} onPointerCancel={end} />
      </div>
      <div className="iris-strip-playhead" style={{ left: pct(time) }} />
    </div>
  </div>;
}
