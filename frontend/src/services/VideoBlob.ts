import { useEffect, useState } from 'react';
import { download } from './Download';

/** Maior vídeo tocado dentro da Ayla: ele fica inteiro na memória enquanto toca. */
const MAX_BYTES = 300 * 1024 * 1024;

/** O vídeo inteiro como blob: o WebKitGTK não toca vídeo direto do protocolo interno da Ayla. */
export function useVideoBlob(src: string) {
  const [state, setState] = useState<{ src: string; url?: string; error?: 'grande' | 'indisponível' }>({ src });
  // Outro vídeo: volta a carregar (ajuste durante a renderização, sem efeito).
  if (state.src !== src) setState({ src });
  useEffect(() => {
    if (!src) return;
    const abort = new AbortController();
    let made: string | null = null;
    download(src, abort.signal, MAX_BYTES, 'video/mp4').then(blob => { made = URL.createObjectURL(blob); setState({ src, url: made }); }, (e: Error) => {
      if (!abort.signal.aborted) setState({ src, error: e.message === 'grande' ? 'grande' : 'indisponível' });
    });
    return () => { abort.abort(); if (made) URL.revokeObjectURL(made); };
  }, [src]);
  return state.src === src ? state : { src };
}

/** Solta o decodificador de um vídeo que sai da tela. No WebKitGTK o elemento removido continua
 * com o player aberto até a coleta de lixo; com o OpenH264, vários abertos fazem o próximo falhar. */
export function releaseVideo(video: HTMLVideoElement | null) {
  if (!video) return;
  video.pause();
  video.removeAttribute('src');
  video.load();
}
