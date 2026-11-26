import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Markdown from './Markdown';
import type { EditorView } from '@codemirror/view';
import MediaView from './MediaView';
import { errorText, native } from '../services/Desktop';
import { base64 } from '../services/Audio';
import { attachmentUrl, changeBanner, folderOf, frontmatter, isAttachment, titleOf, type Banner as BannerSettings } from '../services/Wiki';

const AthenaEditor = lazy(() => import('./AthenaEditor'));
/** Link sendo digitado antes do cursor ("[[Bo|"): o que já foi escrito e onde começa. */
function linkAt(body: string, caret: number) {
  const prefix = body.slice(0, caret);
  const open = prefix.lastIndexOf('[[');
  if (open < 0 || prefix.slice(open).includes(']]') || prefix.slice(open).includes('\n') || caret - open > 80) return null;
  return { query: prefix.slice(open + 2), start: open + 2 };
}

export type Note = { path: string; title: string; modified: number; bytes: number };
export type Tab = { root: string; path: string; pinned: boolean; editing: boolean };


/** Escolha do banner entre as imagens da mesma raiz da nota, com busca e miniaturas. */
function BannerPicker({ root, rootName, images, onPick, onClose }: {
  root: string; rootName: string; images: Note[]; onPick: (path: string) => void; onClose: () => void;
}) {
  const [query, setQuery] = useState('');
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = images.filter(image => words.every(word => image.path.toLowerCase().includes(word)))
    .sort((a, b) => b.modified - a.modified).slice(0, 120);
  return <div className="dialog-backdrop" onClick={onClose}>
    <div className="dialog banner-picker" role="dialog" aria-modal="true" aria-labelledby="banner-picker-title"
      onClick={event => event.stopPropagation()} onKeyDown={event => { if (event.key === 'Escape') onClose(); }}>
      <h2 id="banner-picker-title">Escolher banner</h2>
      <p>Imagens de <b>{rootName}</b>.</p>
      {images.length > 0 && <input className="organ-search" type="search" autoFocus value={query} placeholder="Buscar imagem pelo nome ou pasta"
        aria-label="Buscar imagem" onChange={event => setQuery(event.target.value)}
        onKeyDown={event => { if (event.key === 'Enter' && shown[0]) onPick(shown[0].path); }} />}
      {!images.length ? <p role="status">Nenhuma imagem em {rootName}. Coloque uma imagem dentro dessa pasta para usá-la como banner.</p>
        : !shown.length ? <p role="status">Nenhuma imagem encontrada.</p>
        : <ul className="banner-choices">{shown.map(image => <li key={image.path}>
          <button type="button" title={image.path} onClick={() => onPick(image.path)}>
            <img src={attachmentUrl(root, image.path, true)} alt="" loading="lazy" decoding="async" />
            <span>{image.path.slice(image.path.lastIndexOf('/') + 1)}</span>
            {folderOf(image.path) && <small>{folderOf(image.path)}</small>}
          </button>
        </li>)}</ul>}
      <div className="row-actions"><button type="button" className="ghost-button" onClick={onClose}>Cancelar</button></div>
    </div>
  </div>;
}

function Banner({ url, settings, onChange }: {
  url: string; settings: BannerSettings; onChange?: (key: 'banner_height' | 'banner_y', value: number) => void;
}) {
  const [failed, setFailed] = useState(false);
  const [adjusting, setAdjusting] = useState(false);
  const frame = useRef<HTMLDivElement>(null);
  const drag = useRef<{ y: number; height: number } | null>(null);
  const [dragHeight, setDragHeight] = useState<number>();
  const height = dragHeight ?? settings.height;
  const clamp = (value: number) => Math.max(80, Math.min(600, Math.round(value)));
  const resetDrag = () => { drag.current = null; setDragHeight(undefined); };
  if (failed && !onChange) return null;
  return <section className="note-banner-region">
    <div ref={frame} className="note-banner" style={height ? { height } : undefined}>
      {!failed && <img src={url} alt={onChange ? '' : 'Banner da nota'} style={{ objectPosition: `center ${settings.y}%` }} onError={() => setFailed(true)} />}
      {onChange && <button type="button" className="note-banner-adjust" aria-label="Ajustar banner" aria-expanded={adjusting}
        onClick={() => setAdjusting(value => !value)}>
        {failed ? 'Imagem do banner indisponível' : 'Ajustar banner'}
      </button>}
      {onChange && <button type="button" className="note-banner-resize" aria-label="Redimensionar banner" title="Arraste a borda para ajustar a altura"
        onPointerDown={event => {
          if (event.button !== 0 || !frame.current) return;
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { y: event.clientY, height: frame.current.getBoundingClientRect().height };
        }}
        onPointerMove={event => {
          if (drag.current) setDragHeight(clamp(drag.current.height + event.clientY - drag.current.y));
        }}
        onPointerUp={event => {
          const start = drag.current;
          resetDrag();
          if (start && start.y !== event.clientY) onChange('banner_height', clamp(start.height + event.clientY - start.y));
        }}
        onLostPointerCapture={resetDrag}
        onKeyDown={event => {
          if (!frame.current || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return;
          event.preventDefault();
          onChange('banner_height', clamp(frame.current.getBoundingClientRect().height + (event.key === 'ArrowDown' ? 10 : -10)));
        }} />}
    </div>
    {adjusting && onChange && <div className="note-banner-controls" role="group" aria-label="Ajustes do banner">
      <label>Posição vertical <input type="range" min="0" max="100" value={Math.round(settings.y)} onChange={event => onChange('banner_y', Number(event.currentTarget.value))} /> <output>{Math.round(settings.y)}%</output></label>
      <button type="button" className="ghost-button" onClick={() => setAdjusting(false)}>Fechar</button>
    </div>}
  </section>;
}

/** Anexo aberto numa aba (imagem, vídeo ou PDF), no leitor de mídia comum à Iris. */
export function AttachmentView({ root, target, onError }: { root: string; target: string; onError: (text: string) => void }) {
  return <div className="attachment-view"><MediaView media={{ root, path: target }} name={target.slice(target.lastIndexOf('/') + 1)} bar onError={onError} /></div>;
}

type Props = {
  tab: Tab;
  /** Notas da mesma raiz, para sugerir links ao digitar [[. */
  notes: Note[];
  /** Apelidos das notas da mesma raiz, por caminho (`aliases` nas propriedades, como no Obsidian). */
  aliases?: Record<string, string[]>;
  /** Imagens da mesma raiz (cofre, projeto ou Athena): as únicas que podem virar banner. */
  images: Note[];
  rootName: string;
  /** Altura padrão dos banners no make-md do cofre, para ficar igual ao Obsidian. */
  bannerHeight?: number;
  /** Muda quando a nota pode ter mudado fora daqui (conversa, Git): recarrega se não houver edição pendente. */
  revision: number;
  onEditing: (editing: boolean) => void;
  onFollow: (target: string) => void;
  onOpen: (path: string) => void;
  /** Abre um anexo (imagem ou PDF) numa aba. */
  onAttachment: (target: string) => void;
  onRename: (name: string) => Promise<void>;
  /** Nota recém-criada: abre com o título selecionado para digitar o nome (como no Obsidian). */
  askTitle?: boolean;
  onTitled?: () => void;
  onSaved: () => void;
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
  const [choosingBanner, setChoosingBanner] = useState(false);
  const [backlinks, setBacklinks] = useState<{ path: string; title: string }[]>([]);
  const [renaming, setRenaming] = useState<string | null>(() => props.askTitle ? titleOf(props.tab.path) : null);
  /** Evita gravar o nome duas vezes (Enter seguido do blur do campo). */
  const renamingNow = useRef(false);
  const [suggest, setSuggest] = useState<{ query: string; start: number } | null>(null);
  /** Sugestão destacada: Enter ou Tab a escolhe; as setas mudam o destaque (como no Obsidian). */
  const [highlight, setHighlight] = useState(0);
  const editor = useRef<EditorView | null>(null);
  const stamp = useRef<number | null>(null); const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dirty = useRef(false); const latest = useRef('');
  const saving = useRef<Promise<void> | null>(null);
  const pasteJob = useRef<Promise<void> | null>(null);
  const [pasting, setPasting] = useState(false);
  const events = useRef(props);
  useEffect(() => { events.current = props; });

  /** Conteúdo lido do disco vira o estado da aba (e a nota vazia abre em edição). */
  const apply = useCallback((note: { content: string; modified: number }) => {
    setContent(note.content); latest.current = note.content; stamp.current = note.modified; dirty.current = false; setFailed('');
    if (!note.content.trim() || /^# [^\n]*\n*$/.test(note.content)) events.current.onEditing(true);
    void native<{ path: string; title: string }[]>('athena_backlinks', { root: tab.root, path: tab.path }).then(setBacklinks).catch(() => setBacklinks([]));
  }, [tab.root, tab.path]);
  const read = useCallback(() => native<{ content: string; modified: number }>('athena_read', { root: tab.root, path: tab.path }), [tab.root, tab.path]);
  const load = useCallback(() => read().then(apply, e => setFailed(errorText(e))), [read, apply]);
  useEffect(() => {
    if (dirty.current || pasteJob.current) return;
    let alive = true;
    // Mesma versão que já está na tela: não reaplica (o cursor não pula).
    read().then(note => { if (alive && !dirty.current && !pasteJob.current && !(note.modified === stamp.current && note.content === latest.current)) apply(note); }, e => { if (alive) setFailed(errorText(e)); });
    return () => { alive = false; };
  }, [read, apply, revision]);

  const save = useCallback(async () => {
    while (saving.current) await saving.current;
    if (!dirty.current || pasteJob.current) return;
    const text = latest.current;
    const write = (async () => {
      try {
        stamp.current = await native<number>('athena_write', { root: tab.root, path: tab.path, content: text, expected: stamp.current });
        if (latest.current === text) dirty.current = false;
        events.current.onStatus('Salva'); events.current.onSaved();
      } catch (e) { events.current.onError(errorText(e)); dirty.current = false; await load(); }
    })();
    saving.current = write;
    try { await write; } finally { saving.current = null; }
  }, [tab.root, tab.path, load]);
  const flush = useCallback(async () => {
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    await pasteJob.current;
    await save();
  }, [save]);
  useEffect(() => {
    events.current.register(flush);
    return () => { events.current.register(null); void flush(); };
  }, [flush]);
  useEffect(() => { if (!tab.editing) void flush(); }, [tab.editing, flush]);

  function change(text: string) {
    setContent(text); latest.current = text; dirty.current = true; props.onStatus('Salvando…');
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void save(), 700);
  }
  function pasteImage(event: ClipboardEvent, view: EditorView): boolean {
    const clipboard = event.clipboardData;
    const file = Array.from(clipboard?.files ?? []).find(file => file.type.startsWith('image/'))
      ?? Array.from(clipboard?.items ?? []).find(item => item.kind === 'file' && item.type.startsWith('image/'))?.getAsFile();
    if (!file) return false;
    if (pasteJob.current) return true;
    if (file.size > 15 * 1024 * 1024) { props.onError('A imagem passa de 15 MB'); return true; }
    const value = latest.current;
    const body = frontmatter(value).body;
    const header = value.slice(0, value.length - body.length);
    const newline = body.includes('\r\n') ? '\r\n' : '\n';
    const { from, to } = view.state.selection.main;
    const before = header + view.state.doc.sliceString(0, from, newline);
    const after = view.state.doc.sliceString(to, undefined, newline);
    setSuggest(null); setPasting(true); props.onStatus('Salvando imagem…');
    pasteJob.current = (async () => {
      try {
        await saving.current;
        const expected = stamp.current;
        if (expected === null || latest.current !== value) throw new Error('A nota mudou. Cole a imagem novamente.');
        const data = base64(new Uint8Array(await file.arrayBuffer()));
        const { embed, modified } = await native<{ embed: string; modified: number }>('athena_paste_image', {
          root: tab.root, path: tab.path, before, after, expected, data,
        });
        const text = before + embed + after;
        latest.current = text; stamp.current = modified; dirty.current = false;
        setContent(text); events.current.onStatus('Salva'); events.current.onSaved();
        requestAnimationFrame(() => {
          if (editor.current === view) view.dispatch({ selection: { anchor: Math.min(from + embed.length, view.state.doc.length) }, scrollIntoView: true });
        });
      } catch (e) {
        events.current.onStatus('');
        events.current.onError(errorText(e));
      } finally {
        pasteJob.current = null;
        setPasting(false);
        if (dirty.current) void save();
      }
    })();
    return true;
  }

  /** Usa como banner uma imagem escolhida entre as da mesma raiz da nota. */
  function addBanner(image: string) {
    setChoosingBanner(false);
    if (pasteJob.current || stamp.current === null) return;
    setPasting(true);
    pasteJob.current = (async () => {
      try {
        await saving.current;
        const expected = stamp.current;
        if (expected === null) return;
        const result = await native<{ content: string; modified: number }>('athena_set_banner', {
          root: tab.root, path: tab.path, content: latest.current, expected, image,
        });
        apply(result);
        events.current.onStatus('Salva');
        events.current.onSaved();
      } catch (error) {
        events.current.onError(errorText(error));
      } finally {
        pasteJob.current = null;
        setPasting(false);
        if (dirty.current) void save();
      }
    })();
  }

  function format(before: string, after = '', line = false) {
    const view = editor.current;
    if (!view || pasteJob.current) return;
    const { from, to } = view.state.selection.main;
    const anchor = line ? view.state.doc.lineAt(from).from : from;
    const end = line ? anchor : to;
    const row = view.state.doc.lineAt(anchor);
    const gap = before === '- ' && row.number > 1 && !row.length && view.state.doc.line(row.number - 1).text.trim() ? '\n' : '';
    const insert = gap + before + view.state.doc.sliceString(anchor, end) + after;
    view.dispatch({ changes: { from: anchor, to: end, insert }, selection: { anchor: to + gap.length + before.length } });
    view.focus();
  }

  function typed(body: string, caret: number) {
    const text = latest.current;
    const prior = frontmatter(text).body;
    const newline = prior.includes('\r\n') ? '\r\n' : '\n';
    change(text.slice(0, text.length - prior.length) + body.replace(/\n/g, newline));
    setSuggest(linkAt(body, caret));
    setHighlight(0);
  }
  // Cursor levado para fora do link (clique ou setas, sem Enter): a lista fecha. Só digitar a abre.
  function caretMoved(caret: number) {
    setSuggest(current => {
      if (!current) return current;
      const next = linkAt(editor.current?.state.doc.toString() ?? '', caret);
      return next && next.start === current.start ? next : null;
    });
  }

  function pick(title: string, alias?: string) {
    const view = editor.current;
    if (!view || !suggest || pasteJob.current) return;
    const caret = view.state.selection.main.head;
    const closing = view.state.doc.sliceString(caret, caret + 2) === ']]' ? 2 : 0;
    const insert = `${title}${alias ? `|${alias}` : ''}]]`;
    view.dispatch({ changes: { from: suggest.start, to: caret + closing, insert }, selection: { anchor: suggest.start + insert.length } });
    setSuggest(null);
    view.focus();
  }

  function suggestKey(key: 'Enter' | 'Tab' | 'ArrowUp' | 'ArrowDown') {
    if (!suggestions.length) return false;
    if (key === 'ArrowUp' || key === 'ArrowDown') {
      const step = key === 'ArrowDown' ? 1 : -1;
      setHighlight(h => (Math.min(h, suggestions.length - 1) + step + suggestions.length) % suggestions.length);
    } else { const chosen = suggestions[Math.min(highlight, suggestions.length - 1)]; pick(chosen.title, chosen.alias); }
    return true;
  }

  function follow(target: string) {
    if (/^(https?:\/\/|\/\/)/i.test(target)) {
      void native('open_web', { url: target.startsWith('//') ? `https:${target}` : target }).catch(error => props.onError(errorText(error)));
      return;
    }
    if (/^[a-z][a-z0-9+.-]*:/i.test(target)) return;
    if (isAttachment(target)) props.onAttachment(target.split('#')[0]);
    else props.onFollow(target);
  }

  const note = useMemo(() => frontmatter(content ?? ''), [content]);
  // Pelo título ou por um apelido (sem diferenciar acentos); o apelido vira o texto do link.
  const suggestions = useMemo(() => {
    if (!suggest) return [];
    const plain = (text: string) => text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
    const q = plain(suggest.query);
    const rank = (text: string) => { const t = plain(text); return t === q ? 0 : t.startsWith(q) ? 1 : t.includes(q) ? 2 : -1; };
    const out: { path: string; title: string; alias?: string; rank: number }[] = [];
    for (const n of notes) {
      if (n.path === tab.path) continue;
      const own = rank(n.title);
      if (own >= 0) out.push({ path: n.path, title: n.title, rank: own });
      for (const alias of props.aliases?.[n.path] ?? []) { const r = rank(alias); if (r >= 0) out.push({ path: n.path, title: n.title, alias, rank: r }); }
    }
    return out.sort((a, b) => a.rank - b.rank).slice(0, 8);
  }, [suggest, notes, tab.path, props.aliases]);
  const fileMedia = useCallback((target: string) => ({ root: tab.root, path: target }), [tab.root]);

  const title = titleOf(tab.path);
  /** Título confirmado (Enter ou clique fora): renomeia o arquivo se mudou. */
  function rename(value: string) {
    if (renamingNow.current) { renamingNow.current = false; return; }
    renamingNow.current = true;
    const name = value.trim(); setRenaming(null); props.onTitled?.();
    if (name && name !== title) void props.onRename(name);
    setTimeout(() => { renamingNow.current = false; }, 0);
  }
  if (failed) return <div className="note-scroll"><article className="note-page"><h1 className="note-title">{title}</h1><p role="alert" className="app-error">{failed}</p></article></div>;
  if (content === null) return <div className="note-scroll" aria-busy="true" />;
  return <div className="note-scroll" onDoubleClick={() => { if (!tab.editing) props.onEditing(true); }}>
    {note.banner ? <Banner key={note.banner.target} url={attachmentUrl(tab.root, note.banner.target)} settings={note.banner.height || !props.bannerHeight ? note.banner : { ...note.banner, height: props.bannerHeight }}
      onChange={tab.editing && !pasting ? (key, value) => change(changeBanner(latest.current, key, value)) : undefined} />
      : <div className="note-banner-margin">
        <button type="button" className="ghost-button" disabled={pasting} aria-haspopup="dialog" onClick={() => setChoosingBanner(true)}>Adicionar banner</button>
      </div>}
    {choosingBanner && <BannerPicker root={tab.root} rootName={props.rootName} images={props.images} onPick={addBanner} onClose={() => setChoosingBanner(false)} />}
    <article className="note-page">
      {renaming !== null
        ? <form onSubmit={e => { e.preventDefault(); rename(renaming); }}><input className="athena-rename" autoFocus value={renaming} maxLength={120} placeholder="Título" aria-label="Título da nota (também o nome do arquivo)" onFocus={e => e.target.select()} onChange={e => setRenaming(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') { renamingNow.current = true; setRenaming(null); props.onTitled?.(); } }} onBlur={() => rename(renaming)} /></form>
        : <h1 className="note-title" title="Clique para renomear" onClick={() => { renamingNow.current = false; setRenaming(title); }}>{title}</h1>}
      {tab.editing
        ? <div className="athena-edit">
            <div className="athena-toolbar" role="toolbar" aria-label="Formatação">
              <button type="button" title="Negrito" onClick={() => format('**', '**')}><b>B</b></button>
              <button type="button" title="Itálico" onClick={() => format('*', '*')}><i>I</i></button>
              <button type="button" title="Título" onClick={() => format('## ', '', true)}>H</button>
              <button type="button" title="Lista" onClick={() => format('- ', '', true)}>•</button>
              <button type="button" title="Tarefa" onClick={() => format('- [ ] ', '', true)}>☐</button>
              <button type="button" title="Link para nota" onClick={() => format('[[', ']]')}>[[ ]]</button>
              <button type="button" title="Tag" onClick={() => format('#')}>#</button>
              {suggestions.length > 0 && <ul className="link-suggest" role="listbox" aria-label="Notas para o link">{suggestions.map((n, k) => <li key={`${n.path}|${n.alias ?? ''}`} role="option" aria-selected={k === Math.min(highlight, suggestions.length - 1)}><button type="button" tabIndex={-1} onMouseDown={e => { e.preventDefault(); pick(n.title, n.alias); }}>{n.alias ?? n.title}<small>{n.alias ? `↳ ${n.title}` : folderOf(n.path)}</small></button></li>)}</ul>}
            </div>
            <Suspense fallback={<div className="athena-visual-editor" aria-busy="true" />}>
              <AthenaEditor editor={editor} root={tab.root} value={note.body} readOnly={pasting}
              autoFocus={renaming === null && !note.body.trim()} onChange={typed} onPaste={pasteImage}
              onBlur={() => { setTimeout(() => setSuggest(null), 150); void flush(); }} onEscape={() => setSuggest(null)} onSuggestKey={suggestKey} onLink={follow} onCaret={caretMoved} />
            </Suspense>
          </div>
        : <>
            {note.properties.length > 0 && <dl className="note-properties" aria-label="Propriedades">
              {note.properties.map(p => <div key={p.key} className="note-property"><dt>{p.key}</dt><dd>{p.values.length
                ? p.values.map(v => p.key === 'tags' || p.key === 'tag'
                  ? <span key={v} className="property-chip">#{v.replace(/^#/, '')}</span>
                  : <span key={v} className="property-chip">{v}</span>)
                : <span className="property-empty">vazio</span>}</dd></div>)}
            </dl>}
            {note.body.trim() ? <Markdown text={note.body} onNoteLink={props.onFollow} media={fileMedia} onAttachment={props.onAttachment} onWebError={props.onError} /> : <p className="athena-empty">Nota vazia. Clique no lápis para escrever.</p>}
            {backlinks.length > 0 && <section className="athena-backlinks"><h2>Mencionada em</h2>{backlinks.map(b => <button key={b.path} className="link-button" onClick={() => props.onOpen(b.path)}>{b.title}</button>)}</section>}
          </>}
    </article>
  </div>;
}
