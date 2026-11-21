import ReactMarkdown, { defaultUrlTransform, type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { native } from '../services/Desktop';
import { WIKI, wikiToMarkdown } from '../services/Wiki';

/** Links da web abrem no navegador do sistema (só http/https); links de notas vão para
 * `onNoteLink`. Imagens remotas nunca são carregadas (evita rastreamento), só o texto alternativo. */
export default function Markdown({ text, onNoteLink }: { text: string; onNoteLink?: (target: string) => void }) {
  const components: Components = {
    a: ({ href, children }) => <a href={href} title={href?.startsWith(WIKI) ? decodeURIComponent(href.slice(WIKI.length)) : href} className={href?.startsWith(WIKI) ? 'note-link' : undefined} onClick={event => {
      event.preventDefault();
      if (!href) return;
      if (href.startsWith(WIKI)) onNoteLink?.(decodeURIComponent(href.slice(WIKI.length)));
      else if (/^https?:\/\//i.test(href)) void native('open_web', { url: href }).catch(() => {});
      else if (onNoteLink && /\.md$/i.test(href.split('#')[0])) onNoteLink(decodeURIComponent(href.split('#')[0]));
    }}>{children}</a>,
    img: ({ alt }) => alt ? <span className="md-image-alt">[imagem: {alt}]</span> : null,
  };
  const source = onNoteLink ? wikiToMarkdown(text) : text;
  return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={components} skipHtml urlTransform={url => url.startsWith(WIKI) ? url : defaultUrlTransform(url)}>{source}</ReactMarkdown></div>;
}
