import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import Markdown from './Markdown';
import ZoomImage from './ZoomImage';
import { errorText, native } from '../services/Desktop';
import { PDF, attachmentUrl, frontmatter } from '../services/Wiki';

export type Note = { path: string; title: string; modified: number; bytes: number };
export type Tab = { root: string; path: string; pinned: boolean; editing: boolean };

export const titleOf = (path: string) => path.slice(path.lastIndexOf('/') + 1).replace(/\.md$/i, '');
export const folderOf = (path: string) => path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';

/** Banner da nota (propriedade `banner`, como no make-md), na largura toda, com a altura `banner_y`. */
function Banner({ url, y }: { url: string; y: number }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => { setFailed(false); }, [url]);
  if (failed) return null;
  return <div className="note-banner" role="img" aria-label="Banner da nota"><img src={url} alt="" style={{ objectPosition: `center ${y}%` }} onError={() => setFailed(true)} /></div>;
}

/** Anexo aberto numa aba (imagem ou PDF), com opção de abrir no aplicativo do sistema. */
export function AttachmentView({ root, target, onError }: { root: string; target: string; onError: (text: string) => void }) {
  const url = attachmentUrl(root, target);
  return <div className="note-scroll attachment-view">
    <div className="attachment-bar"><b>{target.slice(target.lastIndexOf('/') + 1)}</b><button className="ghost-button" onClick={() => void native('athena_open_attachment', { root, target }).catch(e => onError(errorText(e)))}>Abrir no visualizador do sistema</button></div>
    {PDF.test(target) ? <iframe className="attachment-pdf" src={url} title={target} /> : <div className="attachment-image"><ZoomImage src={url} alt={target} /></div>}
  </div>;
}

type Props = {
  tab: Tab;
  /** Notas da mesma raiz, para sugerir links ao digitar [[. */
  notes: Note[];
  /** Muda quando a nota pode ter mudado fora daqui (conversa, Git): recarrega se não houver edição pendente. */
  revision: number;
  onEditing: (editing: boolean) => void;
  onFollow: (target: string) => void;
  onOpen: (path: string) => void;
  /** Abre um anexo (imagem ou PDF) numa aba. */
  onAttachment: (target: string) => void;
  onRename: (name: string) => Promise<void>;
  onSaved: () => void;
  onTag: (tag: string) => void;
  onStatus: (text: string) => void;
  onError: (text: string) => void;
  /** Entrega a função que grava o que falta (antes de apagar, mover ou abrir no Obsidian). */
  register: (flush: (() => Promise<void>) | null) => void;
};

/** Uma nota aberta numa aba: leitura (banner, título, propriedades, Markdown) ou edição com
 * gravação automática. Grava o que estiver pendente ao trocar de aba ou fechar. */
export default function AthenaNote(props: Props) {
  const { tab, notes, revision } = props;
  const [content, setContent] = useState<string | null>(null); const [failed, setFailed] = useState('');
  const [backlinks, setBacklinks] = useState<{ path: string; title: string }[]>([]);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [suggest, setSuggest] = useState<{ query: string; start: number } | null>(null);
  const editor = useRef<HTMLTextAreaElement | null>(null);
  const stamp = useRef<number | null>(null); const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dirty = useRef(false); const latest = useRef('');
  const events = useRef(props);
  useEffect(() => { events.current = props; });

  const load = useCallback(async () => {
    try {
      const note = await native<{ content: string; modified: number }>('athena_read', { root: tab.root, path: tab.path });
      setContent(note.content); latest.current = note.content; stamp.current = note.modified; dirty.current = false; setFailed('');
      if (!note.content.trim() || /^# [^\n]*\n*$/.test(note.content)) events.current.onEditing(true);
      void native<{ path: string; title: string }[]>('athena_backlinks', { root: tab.root, path: tab.path }).then(setBacklinks).catch(() => setBacklinks([]));
    } catch (e) { setFailed(errorText(e)); }
  }, [tab.root, tab.path]);
  useEffect(() => { if (!dirty.current) void load(); }, [load, revision]);

  const save = useCallback(async (text: string) => {
    try {
      stamp.current = await native<number>('athena_write', { root: tab.root, path: tab.path, content: text, expected: stamp.current });
      if (latest.current === text) dirty.current = false;
      events.current.onStatus('Salva'); events.current.onSaved();
    } catch (e) { events.current.onError(errorText(e)); dirty.current = false; await load(); }
  }, [tab.root, tab.path, load]);
  const flush = useCallback(async () => {
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    if (dirty.current) await save(latest.current);
  }, [save]);
  useEffect(() => {
    events.current.register(flush);
    return () => { events.current.register(null); void flush(); };
  }, [flush]);
  useEffect(() => { if (!tab.editing) void flush(); }, [tab.editing, flush]);

  function change(text: string) {
    setContent(text); latest.current = text; dirty.current = true; props.onStatus('Salvando…');
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void save(text), 700);
  }
  /** Barra do editor: envolve a seleção ou insere no começo da linha. */
  function format(before: string, after = '', line = false) {
    const field = editor.current;
    if (!field || content === null) return;
    const { selectionStart: a, selectionEnd: b } = field;
    let next: string;
    if (line) { const start = content.lastIndexOf('\n', a - 1) + 1; next = content.slice(0, start) + before + content.slice(start); }
    else next = content.slice(0, a) + before + content.slice(a, b) + after + content.slice(b);
    const caret = b + before.length;
    change(next);
    requestAnimationFrame(() => { field.focus(); field.setSelectionRange(caret, caret); });
  }
  /** Ao digitar [[, sugere notas da mesma raiz. */
  function typed(text: string, caret: number) {
    change(text);
    const before = text.slice(0, caret);
    const open = before.lastIndexOf('[[');
    if (open >= 0 && !before.slice(open).includes(']]') && !before.slice(open).includes('\n') && caret - open <= 80) setSuggest({ query: before.slice(open + 2), start: open + 2 });
    else setSuggest(null);
  }
  function pick(title: string) {
    const field = editor.current;
    if (!field || !suggest || content === null) return;
    const next = content.slice(0, suggest.start) + title + ']]' + content.slice(field.selectionStart).replace(/^\]\]/, '');
    change(next); setSuggest(null);
    const position = suggest.start + title.length + 2;
    requestAnimationFrame(() => { field.focus(); field.setSelectionRange(position, position); });
  }

  const note = useMemo(() => frontmatter(content ?? ''), [content]);
  const fileUrl = useCallback((target: string) => attachmentUrl(tab.root, target), [tab.root]);
  useLayoutEffect(() => {
    const field = editor.current;
    if (!field || !tab.editing) return;
    field.style.height = 'auto'; field.style.height = `${field.scrollHeight}px`;
  }, [content, tab.editing]);

  const title = titleOf(tab.path);
  if (failed) return <div className="note-scroll"><article className="note-page"><h1 className="note-title">{title}</h1><p role="alert" className="app-error">{failed}</p></article></div>;
  if (content === null) return <div className="note-scroll" aria-busy="true" />;
  return <div className="note-scroll" onDoubleClick={() => { if (!tab.editing) props.onEditing(true); }}>
    {note.banner && <Banner url={fileUrl(note.banner.target)} y={note.banner.y} />}
    <article className="note-page">
      {renaming !== null
        ? <form onSubmit={e => { e.preventDefault(); const name = renaming.trim(); setRenaming(null); if (name && name !== title) void props.onRename(name); }}><input className="athena-rename" autoFocus value={renaming} maxLength={120} aria-label="Novo nome da nota" onChange={e => setRenaming(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') setRenaming(null); }} onBlur={() => setRenaming(null)} /></form>
        : <h1 className="note-title" title="Clique para renomear" onClick={() => setRenaming(title)}>{title}</h1>}
      {tab.editing
        ? <div className="athena-edit">
            <div className="athena-toolbar" role="toolbar" aria-label="Formatação">
              <button type="button" title="Negrito" onClick={() => format('**', '**')}><b>B</b></button>
              <button type="button" title="Itálico" onClick={() => format('*', '*')}><i>I</i></button>
              <button type="button" title="Título" onClick={() => format('## ', '', true)}>H</button>
              <button type="button" title="Lista" onClick={() => format('- ', '', true)}>•</button>
              <button type="button" title="Tarefa" onClick={() => format('- [ ] ', '', true)}>☐</button>
              <button type="button" title="Link para nota" onClick={() => { format('[[', ']]'); setSuggest({ query: '', start: (editor.current?.selectionStart ?? 0) + 2 }); }}>[[ ]]</button>
              <button type="button" title="Tag" onClick={() => format('#')}>#</button>
              {suggest && (() => {
                const q = suggest.query.toLowerCase();
                const options = notes.filter(n => n.title.toLowerCase().includes(q) && n.path !== tab.path).slice(0, 8);
                return options.length ? <ul className="link-suggest" role="listbox" aria-label="Notas para o link">{options.map(n => <li key={n.path}><button type="button" onMouseDown={e => { e.preventDefault(); pick(n.title); }}>{n.title}<small>{folderOf(n.path)}</small></button></li>)}</ul> : null;
              })()}
            </div>
            <textarea ref={editor} className="athena-editor" value={content} spellCheck aria-label="Conteúdo da nota" onChange={e => typed(e.target.value, e.target.selectionStart)} onBlur={() => { setTimeout(() => setSuggest(null), 150); void flush(); }} onKeyDown={e => { if (e.key === 'Escape') setSuggest(null); }} />
          </div>
        : <>
            {note.properties.length > 0 && <dl className="note-properties" aria-label="Propriedades">
              {note.properties.map(p => <div key={p.key} className="note-property"><dt>{p.key}</dt><dd>{p.values.length
                ? p.values.map(v => p.key === 'tags' || p.key === 'tag'
                  ? <button key={v} className="tag-chip" onClick={() => props.onTag(v.replace(/^#/, '').toLowerCase())}>#{v.replace(/^#/, '')}</button>
                  : <span key={v} className="property-chip">{v}</span>)
                : <span className="property-empty">vazio</span>}</dd></div>)}
            </dl>}
            {note.body.trim() ? <Markdown text={note.body} onNoteLink={props.onFollow} onTag={props.onTag} attachmentUrl={fileUrl} onAttachment={props.onAttachment} /> : <p className="athena-empty">Nota vazia. Clique no lápis para escrever.</p>}
            {backlinks.length > 0 && <section className="athena-backlinks"><h2>Mencionada em</h2>{backlinks.map(b => <button key={b.path} className="link-button" onClick={() => props.onOpen(b.path)}>{b.title}</button>)}</section>}
          </>}
    </article>
  </div>;
}
