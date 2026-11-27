import { useState } from 'react';
import ReactMarkdown, { defaultUrlTransform, type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Parent } from 'unist';
import { errorText, native } from '../services/Desktop';
import MediaView from './MediaView';
import ZoomImage from './ZoomImage';
import Diagram from './Diagram';
import { TAG, WIKI, imageWidth, isAttachment, wikiToMarkdown } from '../services/Wiki';
import { mediaKind, mediaUrl, type MediaRef } from '../services/Media';

/** As quebras simples da nota também são visíveis no editor da Athena. */
function noteLineBreaks() {
  function splitLines(parent: Parent) {
    parent.children = parent.children.flatMap(child => {
      if ('children' in child && Array.isArray(child.children)) splitLines(child as Parent);
      if (child.type !== 'text' || !('value' in child) || typeof child.value !== 'string') return [child];
      const lines = child.value.split(/\r\n|\r|\n/);
      if (lines.length === 1) return [child];
      return lines.flatMap((value, index) => [
        ...(index ? [{ type: 'break' }] : []),
        ...(value ? [{ type: 'text', value }] : []),
      ]);
    });
  }
  return splitLines;
}

/** Anexo de uma nota (imagem, vídeo ou PDF), pelo serviço de mídia comum à Iris. Imagens ficam
 * no texto (com o tamanho de `![[x|300]]`); vídeo e PDF usam o leitor de mídia. */
function Attachment({ target, alt, size, media, onOpen }: { target: string; alt: string; size?: string; media: MediaRef; onOpen?: (target: string) => void }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="md-image-alt">[anexo não encontrado: {alt || target}]</span>;
  const kind = mediaKind(target); const name = target.slice(target.lastIndexOf('/') + 1);
  const open = onOpen && <button type="button" className="ghost-button" onClick={() => onOpen(target)}>Abrir numa aba</button>;
  if (kind === 'video') return <span className="note-video"><MediaView media={media} name={name} kind="video" /></span>;
  if (kind === 'pdf') return <span className="note-pdf"><span className="note-pdf-frame" aria-label={alt || target}><MediaView media={media} name={name} kind="pdf" actions={open} /></span></span>;
  const [width, height] = (size ?? '').split('x').map(Number);
  return <ZoomImage className="note-image" src={mediaUrl(media)} alt={alt} width={width || undefined} height={height || undefined} onError={() => setFailed(true)} onClick={() => onOpen?.(target)} />;
}

type Props = {
  text: string;
  /** Links entre notas ([[nota]] ou para .md): só nas notas da Athena. */
  onNoteLink?: (target: string) => void;
  onTag?: (tag: string) => void;
  /** Endereço de um anexo (imagem ou PDF) da nota: só nas notas da Athena. */
  media?: (target: string) => MediaRef;
  /** Abre um anexo numa aba (clique na imagem ou em [[arquivo.pdf]]). */
  onAttachment?: (target: string) => void;
  onWebError?: (message: string) => void;
  /** `![[nome]]` sem extensão: o anexo com esse nome, se houver. */
  embed?: (name: string) => string | undefined;
};

/** Links da web abrem no navegador do sistema (só http/https). Imagens remotas nunca são carregadas
 * (evita rastreamento), só o texto alternativo; imagens anexadas às notas vêm da pasta da nota. */
export default function Markdown({ text, onNoteLink, onTag, media, onAttachment, onWebError, embed }: Props) {
  const source = onNoteLink ? wikiToMarkdown(text, embed) : text;
  const relative = (src: string) => !/^[a-z][a-z0-9+.-]*:/i.test(src) && !src.startsWith('//') && !src.startsWith('#');
  const components: Components = {
    a: ({ href, children }) => {
      const internal = href?.startsWith(WIKI) || href?.startsWith(TAG);
      // Endereços "www." soltos viram http:// no GFM; abrem como https://, igual ao editor.
      const web = href && (/^http:\/\/www\./i.test(href) ? `https://${href.slice(7)}` : /^https?:\/\//i.test(href) ? href : href.startsWith('//') ? `https:${href}` : undefined);
      const openWeb = () => { if (web) void native('open_web', { url: web }).catch(e => onWebError?.(errorText(e))); };
      return <a href={web || href} title={href?.startsWith(WIKI) ? decodeURIComponent(href.slice(WIKI.length)) : web || href} className={href?.startsWith(TAG) ? 'note-tag' : internal ? 'note-link' : undefined} onAuxClick={web ? event => { if (event.button === 1) { event.preventDefault(); openWeb(); } } : undefined} onClick={event => {
        event.preventDefault();
        if (!href) return;
        if (href.startsWith(WIKI)) { const target = decodeURIComponent(href.slice(WIKI.length)); if (onAttachment && isAttachment(target)) onAttachment(target); else onNoteLink?.(target); }
        else if (href.startsWith(TAG)) onTag?.(decodeURIComponent(href.slice(TAG.length)));
        else if (web) openWeb();
        else if (onAttachment && relative(href) && isAttachment(href)) onAttachment(decodeURIComponent(href.split('#')[0]));
        else if (onNoteLink && /\.md$/i.test(href.split('#')[0])) onNoteLink(decodeURIComponent(href.split('#')[0]));
      }}>{children}</a>;
    },
    // ```mermaid vira o diagrama desenhado (como no Obsidian).
    pre: ({ node, children }) => {
      const code = node?.children[0];
      const classes = code && code.type === 'element' ? code.properties.className : undefined;
      if (code && code.type === 'element' && Array.isArray(classes) && classes.includes('language-mermaid')) {
        const text = code.children.map(child => child.type === 'text' ? child.value : '').join('');
        return <Diagram code={text.trimEnd()} />;
      }
      return <pre>{children}</pre>;
    },
    img: ({ src, alt, title }) => {
      if (typeof src !== 'string') return null;
      const target = src.startsWith(WIKI) ? decodeURIComponent(src.slice(WIKI.length)) : media && relative(src) ? decodeURIComponent(src.split('#')[0]) : null;
      if (target && media) return <Attachment target={target} alt={alt || ''} size={imageWidth(title)} media={media(target)} onOpen={onAttachment} />;
      return alt ? <span className="md-image-alt">[imagem: {alt}]</span> : null;
    },
  };
  return <div className="markdown"><ReactMarkdown remarkPlugins={onNoteLink ? [remarkGfm, noteLineBreaks] : [remarkGfm]} components={components} skipHtml urlTransform={url => url.startsWith(WIKI) || url.startsWith(TAG) ? url : defaultUrlTransform(url)}>{source}</ReactMarkdown></div>;
}
