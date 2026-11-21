import { useEffect, useState } from 'react';
import ReactMarkdown, { defaultUrlTransform, type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { native } from '../services/Desktop';
import { TAG, WIKI, wikiToMarkdown } from '../services/Wiki';

/** Imagem anexada a uma nota (![[foto.png]]), lida da pasta da nota pelo núcleo. */
function Attachment({ target, alt, load }: { target: string; alt: string; load: (target: string) => Promise<string | null> }) {
  const [src, setSrc] = useState<string | null | undefined>(undefined);
  useEffect(() => { let alive = true; void load(target).then(url => { if (alive) setSrc(url); }).catch(() => { if (alive) setSrc(null); }); return () => { alive = false; }; }, [target, load]);
  if (src === undefined) return <span className="md-image-alt">Carregando {alt}…</span>;
  return src ? <img className="note-image" src={src} alt={alt} /> : <span className="md-image-alt">[anexo não encontrado: {alt}]</span>;
}

type Props = {
  text: string;
  /** Links entre notas ([[nota]] ou para .md): só nas notas da Athena. */
  onNoteLink?: (target: string) => void;
  onTag?: (tag: string) => void;
  loadAttachment?: (target: string) => Promise<string | null>;
};

/** Links da web abrem no navegador do sistema (só http/https). Imagens remotas nunca são carregadas
 * (evita rastreamento), só o texto alternativo; imagens anexadas às notas vêm da pasta da nota. */
export default function Markdown({ text, onNoteLink, onTag, loadAttachment }: Props) {
  const components: Components = {
    a: ({ href, children }) => {
      const internal = href?.startsWith(WIKI) || href?.startsWith(TAG);
      return <a href={href} title={href?.startsWith(WIKI) ? decodeURIComponent(href.slice(WIKI.length)) : href} className={href?.startsWith(TAG) ? 'note-tag' : internal ? 'note-link' : undefined} onClick={event => {
        event.preventDefault();
        if (!href) return;
        if (href.startsWith(WIKI)) onNoteLink?.(decodeURIComponent(href.slice(WIKI.length)));
        else if (href.startsWith(TAG)) onTag?.(decodeURIComponent(href.slice(TAG.length)));
        else if (/^https?:\/\//i.test(href)) void native('open_web', { url: href }).catch(() => {});
        else if (onNoteLink && /\.md$/i.test(href.split('#')[0])) onNoteLink(decodeURIComponent(href.split('#')[0]));
      }}>{children}</a>;
    },
    img: ({ src, alt }) => typeof src === 'string' && src.startsWith(WIKI) && loadAttachment
      ? <Attachment target={decodeURIComponent(src.slice(WIKI.length))} alt={alt || ''} load={loadAttachment} />
      : alt ? <span className="md-image-alt">[imagem: {alt}]</span> : null,
  };
  const source = onNoteLink ? wikiToMarkdown(text) : text;
  return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={components} skipHtml urlTransform={url => url.startsWith(WIKI) || url.startsWith(TAG) ? url : defaultUrlTransform(url)}>{source}</ReactMarkdown></div>;
}
