import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';

type View = { scale: number; x: number; y: number };
const MIN = 1; const MAX = 10;

/** Imagem com zoom por Ctrl + roda do mouse (no ponto do cursor). Com zoom, arrastar move a
 * imagem; duplo clique volta ao tamanho normal. Sem zoom, o clique segue para `onClick`. */
export default function ZoomImage({ src, alt, className, width, height, onClick, onError }: {
  src: string; alt: string; className?: string; width?: number; height?: number; onClick?: () => void; onError?: () => void;
}) {
  const frame = useRef<HTMLSpanElement | null>(null); const image = useRef<HTMLImageElement | null>(null);
  const [view, setView] = useState<View>({ scale: 1, x: 0, y: 0 });
  const moved = useRef(false);
  useEffect(() => {
    const element = frame.current;
    if (!element) return;
    // Não-passivo: só assim a página não rola enquanto a imagem recebe o zoom.
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      const delta = event.deltaY || event.deltaX;
      if (!delta) return;
      event.preventDefault();
      const box = element.getBoundingClientRect();
      // Ponto do cursor em relação à posição da imagem sem zoom (ela pode estar centralizada).
      const px = event.clientX - box.left - (image.current?.offsetLeft ?? 0); const py = event.clientY - box.top - (image.current?.offsetTop ?? 0);
      setView(prev => {
        const scale = Math.min(MAX, Math.max(MIN, prev.scale * (delta < 0 ? 1.15 : 1 / 1.15)));
        if (scale === MIN) return { scale: 1, x: 0, y: 0 };
        const ratio = scale / prev.scale;
        return { scale, x: px - (px - prev.x) * ratio, y: py - (py - prev.y) * ratio };
      });
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, []);

  function pan(event: ReactPointerEvent) {
    if (view.scale === 1 || event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    const start = { x: event.clientX, y: event.clientY, vx: view.x, vy: view.y };
    moved.current = false;
    const move = (ev: PointerEvent) => {
      if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) > 3) moved.current = true;
      setView(prev => ({ ...prev, x: start.vx + ev.clientX - start.x, y: start.vy + ev.clientY - start.y }));
    };
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up);
  }

  const zoomed = view.scale > 1;
  return <span ref={frame} className={zoomed ? 'zoom-frame zoomed' : 'zoom-frame'} title={zoomed ? 'Arraste para mover · duplo clique volta ao normal' : 'Ctrl + roda do mouse: zoom'}
    onPointerDown={pan}
    onDoubleClick={e => { if (zoomed) { e.stopPropagation(); setView({ scale: 1, x: 0, y: 0 }); } }}
    onClick={e => { if (zoomed || moved.current) { e.stopPropagation(); moved.current = false; return; } onClick?.(); }}>
    <img ref={image} className={className} src={src} alt={alt} width={width} height={height} draggable={false} onError={onError}
      style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }} />
    {zoomed && <span className="zoom-level" aria-live="polite">{Math.round(view.scale * 100)}%</span>}
  </span>;
}
