import { useState } from 'react';
import ReactMarkdown, { defaultUrlTransform, type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { native } from '../services/Desktop';
import ZoomImage from './ZoomImage';
import { PDF, TAG, WIKI, isAttachment, wikiToMarkdown } from '../services/Wiki';

/** Anexo de uma nota (imagem ou PDF), lido da pasta da nota pelo núcleo. */
function Attachment({ target, alt, size, url, onOpen }: { target: string; alt: string; size?: string; url: string; onOpen?: (target: string) => void }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="md-image-alt">[anexo não encontrado: {alt || target}]</span>;
  if (PDF.test(target)) return <span className="note-pdf">
    <iframe src={url} title={alt || target} onError={() => setFailed(true)} />
    {onOpen && <button type="button" className="link-button" onClick={() => onOpen(target)}>Abrir {target.slice(target.lastIndexOf('/') + 1)} numa aba</button>}
  </span>;
  const [width, height] = (size ?? '').split('x').map(Number);
  return <ZoomImage className="note-image" src={url} alt={alt} width={width || undefined} height={height || undefined} onError={() => setFailed(true)} onClick={() => onOpen?.(target)} />;
}

type Props = {
  text: string;
  /** Links entre notas ([[nota]] ou para .md): só nas notas da Athena. */
  onNoteLink?: (target: string) => void;
  onTag?: (tag: string) => void;
  /** Endereço de um anexo (imagem ou PDF) da nota: só nas notas da Athena. */
  attachmentUrl?: (target: string) => string;
  /** Abre um anexo numa aba (clique na imagem ou em [[arquivo.pdf]]). */
  onAttachment?: (target: string) => void;
};

/** Links da web abrem no navegador do sistema (só http/https). Imagens remotas nunca são carregadas
 * (evita rastreamento), só o texto alternativo; imagens anexadas às notas vêm da pasta da nota. */
export default function Markdown({ text, onNoteLink, onTag, attachmentUrl, onAttachment }: Props) {
  const relative = (src: string) => !/^[a-z][a-z0-9+.-]*:/i.test(src) && !src.startsWith('//') && !src.startsWith('#');
  const components: Components = {
    a: ({ href, children }) => {
      const internal = href?.startsWith(WIKI) || href?.startsWith(TAG);
      return <a href={href} title={href?.startsWith(WIKI) ? decodeURIComponent(href.slice(WIKI.length)) : href} className={href?.startsWith(TAG) ? 'note-tag' : internal ? 'note-link' : undefined} onClick={event => {
        event.preventDefault();
        if (!href) return;
        if (href.startsWith(WIKI)) { const target = decodeURIComponent(href.slice(WIKI.length)); if (onAttachment && isAttachment(target)) onAttachment(target); else onNoteLink?.(target); }
        else if (href.startsWith(TAG)) onTag?.(decodeURIComponent(href.slice(TAG.length)));
        else if (/^https?:\/\//i.test(href)) void native('open_web', { url: href }).catch(() => {});
        else if (onAttachment && relative(href) && isAttachment(href)) onAttachment(decodeURIComponent(href.split('#')[0]));
        else if (onNoteLink && /\.md$/i.test(href.split('#')[0])) onNoteLink(decodeURIComponent(href.split('#')[0]));
      }}>{children}</a>;
    },
    img: ({ src, alt, title }) => {
      if (typeof src !== 'string') return null;
      const target = src.startsWith(WIKI) ? decodeURIComponent(src.slice(WIKI.length)) : attachmentUrl && relative(src) ? decodeURIComponent(src.split('#')[0]) : null;
      if (target && attachmentUrl) return <Attachment target={target} alt={alt || ''} size={title || undefined} url={attachmentUrl(target)} onOpen={onAttachment} />;
      return alt ? <span className="md-image-alt">[imagem: {alt}]</span> : null;
    },
  };
  const source = onNoteLink ? wikiToMarkdown(text) : text;
  return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={components} skipHtml urlTransform={url => url.startsWith(WIKI) || url.startsWith(TAG) ? url : defaultUrlTransform(url)}>{source}</ReactMarkdown></div>;
}
