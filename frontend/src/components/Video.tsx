import { useEffect, useRef, useState } from 'react';
import { FullscreenIcon } from './Icons';
import { download } from '../services/Download';

/** Maior vídeo tocado dentro da Ayla: ele fica inteiro na memória enquanto toca. */
const MAX_BYTES = 300 * 1024 * 1024;

/** Vídeo de uma nota ou da Iris, com controles. `onSystem` abre no player do sistema. */
export default function Video({ src, className, autoPlay, onSystem }: { src: string; className?: string; autoPlay?: boolean; onSystem?: () => void }) {
  const [state, setState] = useState<{ src: string; url?: string; error?: 'grande' | 'indisponível' }>({ src });
  const video = useRef<HTMLVideoElement>(null);
  // Outro vídeo: volta a carregar (ajuste durante a renderização, sem efeito).
  if (state.src !== src) setState({ src });
  useEffect(() => {
    const abort = new AbortController();
    let made: string | null = null;
    download(src, abort.signal, MAX_BYTES, 'video/mp4').then(blob => { made = URL.createObjectURL(blob); setState({ src, url: made }); }, (e: Error) => {
      if (!abort.signal.aborted) setState({ src, error: e.message === 'grande' ? 'grande' : 'indisponível' });
    });
    return () => { abort.abort(); if (made) URL.revokeObjectURL(made); };
  }, [src]);
  if (state.error) return <span className="video-fallback">
    {state.error === 'grande' ? 'Vídeo grande demais para tocar dentro da Ayla (mais de 300 MB).' : 'Não foi possível tocar este vídeo aqui.'}
    {onSystem && <button className="link-button" onClick={onSystem}>Abrir no player do sistema</button>}
  </span>;
  if (!state.url) return <span className="video-fallback" role="status">Carregando o vídeo…</span>;
  return <span className="video-frame">
    <video ref={video} className={className} src={state.url} controls autoPlay={autoPlay} preload="metadata" />
    <button type="button" className="icon-button video-fullscreen tip-end" data-tip="Tela cheia" aria-label="Tela cheia"
      onClick={() => void video.current?.requestFullscreen().catch(() => undefined)}><FullscreenIcon size={18} /></button>
  </span>;
}
