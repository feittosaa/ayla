import { useEffect, useRef } from 'react';
import ZoomImage from './ZoomImage';
import { CloseIcon } from './Icons';

/** Imagem ampliada por cima da conversa (prévias de ficha, imagens enviadas): Ctrl + roda dá zoom,
 * Esc ou clique fora fecha. */
/** `paginas`: documento (prévia de ficha) — as páginas uma embaixo da outra, na largura da tela, rolando. */
export default function ImageLightbox({ src, alt, caption, paginas, inicio = 0, onClose }: { src?: string; alt: string; caption?: string; paginas?: string[]; inicio?: number; onClose: () => void }) {
  const rolagem = useRef<HTMLDivElement | null>(null);
  // Abre na página clicada (só ao abrir; depois a rolagem é de quem lê).
  useEffect(() => { if (inicio > 0) rolagem.current?.querySelectorAll('img')[inicio]?.scrollIntoView({ block: 'start' }); }, [inicio]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose(); } };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [onClose]);
  return <div className="image-lightbox" role="dialog" aria-modal="true" aria-label={alt} onClick={onClose}>
    <div className="image-lightbox-bar" onClick={e => e.stopPropagation()}>
      <span><b>{alt}</b>{caption && <small>{caption}</small>}</span>
      <button type="button" className="icon-button" aria-label="Fechar" data-tip="Fechar (Esc)" onClick={onClose}><CloseIcon size={18} /></button>
    </div>
    {paginas
      ? <div className="image-lightbox-scroll" ref={rolagem} onClick={e => e.stopPropagation()}>
          {paginas.map((pagina, i) => <img key={i} src={pagina} alt={`${alt}, página ${i + 1}`} />)}
        </div>
      : src && <div className="image-lightbox-stage" onClick={e => e.stopPropagation()}><ZoomImage src={src} alt={alt} /></div>}
  </div>;
}
