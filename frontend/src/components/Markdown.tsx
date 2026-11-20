import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { native } from '../services/Desktop';

/** Links abrem no navegador do sistema (só http/https); imagens remotas nunca são
 * carregadas (evita rastreamento e vazamento de endereço), só o texto alternativo. */
const components: Components = {
  a: ({ href, children }) => <a href={href} title={href} onClick={event => { event.preventDefault(); if (href && /^https?:\/\//i.test(href)) void native('open_web', { url: href }).catch(() => {}); }}>{children}</a>,
  img: ({ alt }) => alt ? <span className="md-image-alt">[imagem: {alt}]</span> : null,
};

/** Resposta em Markdown (negrito, itálico, listas, títulos, código, tabelas). HTML da IA é ignorado. */
export default function Markdown({ text }: { text: string }) {
  return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={components} skipHtml>{text}</ReactMarkdown></div>;
}
