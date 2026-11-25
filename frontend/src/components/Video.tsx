import { useRef } from 'react';
import { FullscreenIcon } from './Icons';
import { releaseVideo, useVideoBlob } from '../services/VideoBlob';

/** Vídeo de uma nota ou da Iris, com controles. `onSystem` abre no player do sistema. */
export default function Video({ src, className, autoPlay, onSystem }: { src: string; className?: string; autoPlay?: boolean; onSystem?: () => void }) {
  const state = useVideoBlob(src);
  const video = useRef<HTMLVideoElement>(null);
  if (state.error) return <span className="video-fallback">
    {state.error === 'grande' ? 'Vídeo grande demais para tocar dentro da Ayla (mais de 300 MB).' : 'Não foi possível tocar este vídeo aqui.'}
    {onSystem && <button className="link-button" onClick={onSystem}>Abrir no player do sistema</button>}
  </span>;
  if (!state.url) return <span className="video-fallback" role="status">Carregando o vídeo…</span>;
  return <span className="video-frame">
    <video ref={el => { video.current = el; return () => releaseVideo(el); }} className={className} src={state.url} controls autoPlay={autoPlay} preload="metadata" />
    <button type="button" className="icon-button video-fullscreen tip-end" data-tip="Tela cheia" aria-label="Tela cheia"
      onClick={() => void video.current?.requestFullscreen().catch(() => undefined)}><FullscreenIcon size={18} /></button>
  </span>;
}
