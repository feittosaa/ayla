import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import Markdown from './Markdown';
import type { EditorView } from '@codemirror/view';
import MediaView from './MediaView';
import { errorText, native } from '../services/Desktop';
import { base64 } from '../services/Audio';
import { findAll } from '../services/Find';
import { attachmentUrl, changeBanner, embedResolver, folderOf, frontmatter, isAttachment, removeBanner, setAliases, titleOf, type Banner as BannerSettings } from '../services/Wiki';

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

/** Apelidos da nota (`aliases`, como no Obsidian): criar, editar (clique) e remover (dois passos). */
function NoteAliases({ aliases, onChange }: { aliases: string[]; onChange: (next: string[]) => void }) {
  const [draft, setDraft] = useState<{ index: number; value: string } | null>(null);
  const [removing, setRemoving] = useState<number | null>(null);
  const [problem, setProblem] = useState('');
  function commit() {
    if (!draft) return;
    const value = draft.value.replace(/\s+/g, ' ').trim();
    setDraft(null);
    if (!value || value === aliases[draft.index]) return;
    if (aliases.some((alias, i) => i !== draft.index && alias.toLowerCase() === value.toLowerCase())) { setProblem(`“${value}” já é um apelido desta nota.`); return; }
    setProblem('');
    onChange(draft.index < 0 ? [...aliases, value] : aliases.map((alias, i) => i === draft.index ? value : alias));
  }
  const field = (index: number) => <input className="alias-input" autoFocus value={draft?.value ?? ''} maxLength={120}
    aria-label={index < 0 ? 'Novo apelido' : `Editar o apelido ${aliases[index]}`} placeholder="Apelido"
    onChange={event => setDraft({ index, value: event.target.value })} onBlur={commit}
    onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commit(); } if (event.key === 'Escape') setDraft(null); }} />;
  return <div className="note-aliases" role="group" aria-label="Apelidos">
    <span className="note-aliases-label">Apelidos</span>
    <div className="note-aliases-list">
      {aliases.map((alias, index) => draft?.index === index ? <span key={alias}>{field(index)}</span>
        : removing === index ? <span key={alias} className="property-chip alias-chip removing">
            Remover “{alias}”?
            <button type="button" className="alias-confirm" onClick={() => { setRemoving(null); onChange(aliases.filter((_, i) => i !== index)); }}>Remover</button>
            <button type="button" className="alias-action" aria-label="Cancelar" onClick={() => setRemoving(null)}>×</button>
          </span>
        : <span key={alias} className="property-chip alias-chip">
            <button type="button" className="alias-name" title="Editar apelido" onClick={() => { setRemoving(null); setProblem(''); setDraft({ index, value: alias }); }}>{alias}</button>
            <button type="button" className="alias-action" aria-label={`Remover o apelido ${alias}`} title="Remover" onClick={() => { setDraft(null); setRemoving(index); }}>×</button>
          </span>)}
      {draft?.index === -1 ? field(-1)
        : <button type="button" className="alias-add" onClick={() => { setRemoving(null); setProblem(''); setDraft({ index: -1, value: '' }); }}>+ Apelido</button>}
    </div>
    {problem && <p className="alias-problem" role="alert">{problem}</p>}
  </div>;
}

/** Ctrl+F na nota: caixa no canto superior direito com o total, anterior/próxima e fechar. */
function NoteFind({ query, count, current, input, onQuery, onStep, onClose }: {
  query: string; count: number; current: number; input: { current: HTMLInputElement | null };
  onQuery: (value: string) => void; onStep: (delta: number) => void; onClose: () => void;
}) {
  return <div className="note-find-anchor"><div className="note-find" role="search" aria-label="Buscar nesta nota">
    <input ref={input} type="search" value={query} placeholder="Buscar na nota" aria-label="Buscar na nota" autoFocus
      onChange={event => onQuery(event.target.value)}
      onKeyDown={event => {
        if (event.key === 'Enter') { event.preventDefault(); onStep(event.shiftKey ? -1 : 1); }
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose(); }
      }} />
    <span className="note-find-count" role="status">{query.trim() ? count ? `${current + 1} de ${count}` : 'Nada' : ''}</span>
    <button type="button" className="icon-button" aria-label="Anterior" title="Anterior (Shift+Enter)" disabled={!count} onClick={() => onStep(-1)}>↑</button>
    <button type="button" className="icon-button" aria-label="Próxima" title="Próxima (Enter)" disabled={!count} onClick={() => onStep(1)}>↓</button>
    <button type="button" className="icon-button" aria-label="Fechar a busca" title="Fechar (Esc)" onClick={onClose}>×</button>
  </div></div>;
}

/** Nota que recebeu o último clique: o Ctrl+F vale para ela quando o foco não está em nenhuma. */
let lastNote: HTMLElement | null = null;

function Banner({ url, settings, onChange, onReplace, onRemove }: {
  url: string; settings: BannerSettings; onChange?: (key: 'banner_height' | 'banner_y', value: number) => void;
  /** Escolher outra imagem / tirar o banner (só ao editar). */
  onReplace?: () => void; onRemove?: () => void;
}) {
  const [failed, setFailed] = useState(false);
  const [adjusting, setAdjusting] = useState(false);
  const [removing, setRemoving] = useState(false);
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
      {onReplace && <button type="button" className="ghost-button" aria-haspopup="dialog" onClick={() => { setRemoving(false); onReplace(); }}>Trocar imagem</button>}
      {onRemove && (removing
        ? <span className="note-banner-remove" role="group" aria-label="Confirmar remoção do banner">Remover o banner?
            <button type="button" className="danger-action" onClick={() => { setRemoving(false); setAdjusting(false); onRemove(); }}>Remover</button>
            <button type="button" className="ghost-button" onClick={() => setRemoving(false)}>Cancelar</button>
          </span>
        : <button type="button" className="ghost-button danger" onClick={() => setRemoving(true)}>Remover banner</button>)}
      <button type="button" className="ghost-button" onClick={() => { setRemoving(false); setAdjusting(false); }}>Fechar</button>
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
  /** Anexos da mesma raiz (imagens, PDFs, vídeos): entram nas sugestões do [[ e resolvem `![[nome]]`. */
  files: Note[];
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

/** Onde a pessoa parou em cada nota, para voltar ali ao reabrir (pedido do usuário, 28/11/2026): rolagem,
 * cursor e a distância do cursor ao topo da área da nota. Fica só neste computador (armazenamento da
 * janela, as 300 notas mais recentes); a cópia em memória evita reler o armazenamento a cada troca. */
type Position = { top: number; caret?: number; offset?: number };
const POSITIONS = 'athena-note-positions';
let positions: Map<string, Position> | null = null;
let persist: ReturnType<typeof setTimeout> | null = null;
function allPositions() {
  if (!positions) {
    positions = new Map();
    try {
      const saved = JSON.parse(localStorage.getItem(POSITIONS) ?? '{}') as Record<string, Position | number>;
      for (const [key, value] of Object.entries(saved ?? {})) {
        const position = typeof value === 'number' ? { top: value } : value;
        if (position && Number.isFinite(position.top)) positions.set(key, position);
      }
    } catch { /* posições ilegíveis: começa do topo */ }
  }
  return positions;
}
function savePosition(key: string, position: Position) {
  const all = allPositions(); all.delete(key); all.set(key, position);
  for (const old of [...all.keys()].slice(0, Math.max(0, all.size - 300))) all.delete(old);
  if (persist) clearTimeout(persist);
  persist = setTimeout(() => {
    persist = null;
    try { localStorage.setItem(POSITIONS, JSON.stringify(Object.fromEntries(all))); } catch { /* armazenamento cheio: só não lembra */ }
  }, 300);
}

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
  const scroller = useRef<HTMLDivElement>(null);
  const positionKey = `${tab.root}|${tab.path}`;
  const [finding, setFinding] = useState<{ query: string; current: number; step: number } | null>(null);
  const findInput = useRef<HTMLInputElement>(null);
  const [viewMatches, setViewMatches] = useState<Range[]>([]);
  const loaded = content !== null;
  /** Guarda a tela e o cursor desta nota (com o cursor, também onde ele aparecia na tela). */
  const remember = useCallback(() => {
    const box = scroller.current; if (!box?.isConnected) return;
    const view = editor.current; const position: Position = { top: Math.round(box.scrollTop) };
    if (view) {
      const caret = view.state.selection.main.head; position.caret = caret;
      const at = view.coordsAtPos(caret);
      if (at) position.offset = Math.round(at.top - box.getBoundingClientRect().top);
    }
    savePosition(positionKey, position);
  }, [positionKey]);
  /** Até a nota assentar (banner, imagens, editor carregado sob demanda; até 3 s), recoloca a tela onde estava: pelo cursor quando
   * há editor (o cursor volta à mesma altura), senão pela rolagem. Para quando a pessoa mexe. */
  const restoring = useRef<{ until: number; caretSet: boolean } | null>(null);
  const settle = useCallback(() => {
    const box = scroller.current; const saved = allPositions().get(positionKey); const state = restoring.current;
    if (!box || !saved || !state || performance.now() > state.until) { restoring.current = null; return; }
    const view = editor.current;
    if (view && saved.caret !== undefined) {
      const caret = Math.min(saved.caret, view.state.doc.length);
      if (!state.caretSet) { state.caretSet = true; view.dispatch({ selection: { anchor: caret } }); view.contentDOM.focus({ preventScroll: true }); box.scrollTop = saved.top; }
      const at = view.coordsAtPos(caret);
      if (at && saved.offset !== undefined) box.scrollTop += at.top - box.getBoundingClientRect().top - saved.offset;
      else box.scrollTop = saved.top;
    } else box.scrollTop = saved.top;
    requestAnimationFrame(settle);
  }, [positionKey]);
  useEffect(() => {
    const box = scroller.current; if (!box || !loaded) return;
    if (allPositions().has(positionKey)) { restoring.current = { until: performance.now() + 3000, caretSet: false }; requestAnimationFrame(settle); }
    let pending: ReturnType<typeof setTimeout> | null = null;
    const userMoved = () => { restoring.current = null; };
    const onScroll = () => { if (restoring.current) return; if (pending) clearTimeout(pending); pending = setTimeout(remember, 200); };
    const events = ['wheel', 'touchmove', 'keydown', 'pointerdown'] as const;
    for (const name of events) box.addEventListener(name, userMoved, { passive: true });
    box.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      if (pending) clearTimeout(pending); restoring.current = null;
      for (const name of events) box.removeEventListener(name, userMoved);
      box.removeEventListener('scroll', onScroll);
    };
  }, [positionKey, loaded, remember, settle]);
  // Ao sair da nota (outra nota, outra aba, fechar), guarda antes de o editor e a página sumirem.
  useLayoutEffect(() => () => { if (!restoring.current) remember(); }, [remember]);
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
          // Outra imagem: a posição da anterior não serve; a altura fica.
          root: tab.root, path: tab.path, content: removeBanner(latest.current, ['banner_y']), expected, image,
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
    if (!restoring.current) remember();
  }
  // Cursor levado para fora do link (clique ou setas, sem Enter): a lista fecha. Só digitar a abre.
  function caretMoved(caret: number) {
    if (!restoring.current) remember();
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
    const embed = view.state.doc.sliceString(suggest.start - 3, suggest.start - 2) === '!';
    const insert = `${title}${alias && !embed ? `|${alias}` : ''}]]`;
    view.dispatch({ changes: { from: suggest.start, to: caret + closing, insert }, selection: { anchor: suggest.start + insert.length } });
    setSuggest(null);
    view.focus();
  }

  function suggestKey(key: 'Enter' | 'Tab' | 'ArrowUp' | 'ArrowDown') {
    if (!suggestions.length) return false;
    if (key === 'ArrowUp' || key === 'ArrowDown') {
      const step = key === 'ArrowDown' ? 1 : -1;
      setHighlight(h => (Math.min(h, suggestions.length - 1) + step + suggestions.length) % suggestions.length);
    } else { const chosen = suggestions[Math.min(highlight, suggestions.length - 1)]; pick(chosen.link ?? chosen.title, chosen.alias); }
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
  // Busca: no editor, no texto da nota (o editor só desenha o que está na tela); no modo Ver, no
  // texto mostrado, destacado sem mexer no HTML (CSS Custom Highlight).
  const findQuery = finding?.query ?? '';
  const editMatches = useMemo(() => tab.editing && findQuery.trim() ? findAll(editor.current?.state.doc.toString() ?? note.body.replace(/\r\n/g, '\n'), findQuery) : [],
    [tab.editing, findQuery, note.body]);
  useEffect(() => {
    if (tab.editing || !findQuery.trim()) { setViewMatches([]); return; }
    const page = scroller.current?.querySelector('article.note-page'); if (!page) return;
    const ranges: Range[] = [];
    const walker = document.createTreeWalker(page, NodeFilter.SHOW_TEXT, { acceptNode: node =>
      node.parentElement?.closest('.note-find, .note-aliases, input, textarea, button, script, style') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT });
    for (let node = walker.nextNode(); node && ranges.length < 2000; node = walker.nextNode()) {
      for (const [from, to] of findAll((node as Text).data, findQuery, 2000 - ranges.length)) {
        const range = document.createRange(); range.setStart(node, from); range.setEnd(node, to); ranges.push(range);
      }
    }
    setViewMatches(ranges);
  }, [tab.editing, findQuery, content]);
  const findCount = tab.editing ? editMatches.length : viewMatches.length;
  const findCurrent = finding && findCount ? Math.min(finding.current, findCount - 1) : 0;
  useEffect(() => {
    const registry = typeof CSS !== 'undefined' && 'highlights' in CSS ? CSS.highlights : null;
    if (!registry || tab.editing || !viewMatches.length) return;
    registry.set('athena-find', new Highlight(...viewMatches));
    registry.set('athena-find-current', new Highlight(viewMatches[findCurrent]));
    return () => { registry.delete('athena-find'); registry.delete('athena-find-current'); };
  }, [tab.editing, viewMatches, findCurrent]);
  // Modo Ver: leva a ocorrência atual a um terço da altura da nota.
  const findStep = finding?.step;
  useEffect(() => {
    const box = scroller.current; const range = viewMatches[findCurrent];
    if (tab.editing || !box || !range) return;
    const at = range.getBoundingClientRect(); const frame = box.getBoundingClientRect();
    if (at.top < frame.top + 60 || at.bottom > frame.bottom - 20) box.scrollTop += at.top - frame.top - box.clientHeight / 3;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [findStep, viewMatches]);
  const editorFind = useMemo(() => tab.editing && finding && editMatches.length ? { matches: editMatches, current: findCurrent, step: finding.step } : null,
    [tab.editing, finding, editMatches, findCurrent]);
  function closeFind() { setFinding(null); if (tab.editing) editor.current?.focus(); }
  function stepFind(delta: number) {
    setFinding(state => state && findCount ? { ...state, current: (Math.min(state.current, findCount - 1) + delta + findCount) % findCount, step: state.step + 1 } : state);
  }
  // Ctrl+F: na nota com o foco ou, sem foco em nenhuma, na última clicada (ou na única visível).
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey || event.key.toLowerCase() !== 'f') return;
      const box = scroller.current; if (!box || !box.offsetParent) return;
      const focused = document.activeElement;
      const owner = focused instanceof HTMLElement ? focused.closest('.note-scroll') : null;
      const visible = [...document.querySelectorAll<HTMLElement>('.note-scroll')].filter(n => n.offsetParent);
      const mine = owner ? owner === box : (lastNote && visible.includes(lastNote) ? lastNote === box : visible[0] === box);
      if (!mine) return;
      event.preventDefault();
      const picked = tab.editing ? (() => { const v = editor.current; if (!v) return ''; const r = v.state.selection.main; return v.state.sliceDoc(r.from, r.to); })()
        : (window.getSelection()?.toString() ?? '');
      const seed = picked && !picked.includes('\n') && picked.length <= 80 ? picked : '';
      setFinding(state => ({ query: seed || state?.query || '', current: 0, step: (state?.step ?? 0) + 1 }));
      requestAnimationFrame(() => { findInput.current?.focus(); findInput.current?.select(); });
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [tab.editing]);
  // Pelo título ou por um apelido (sem diferenciar acentos); o apelido vira o texto do link.
  // Anexos (imagens, PDFs) também entram, com a extensão, como no Obsidian; depois de "![[", vêm primeiro.
  const { files } = props;
  const suggestions = useMemo(() => {
    if (!suggest) return [];
    const plain = (text: string) => text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
    const q = plain(suggest.query);
    const rank = (text: string) => { const t = plain(text); return t === q ? 0 : t.startsWith(q) ? 1 : t.includes(q) ? 2 : -1; };
    const embed = editor.current?.state.doc.sliceString(suggest.start - 3, suggest.start - 2) === '!';
    const out: { path: string; title: string; alias?: string; link?: string; file?: boolean; rank: number }[] = [];
    for (const n of notes) {
      if (n.path === tab.path) continue;
      const own = rank(n.title);
      if (own >= 0) out.push({ path: n.path, title: n.title, rank: own + (embed ? 3 : 0) });
      if (!embed) for (const alias of props.aliases?.[n.path] ?? []) { const r = rank(alias); if (r >= 0) out.push({ path: n.path, title: n.title, alias, rank: r }); }
    }
    const names = new Map<string, number>();
    for (const f of files) { const name = f.path.slice(f.path.lastIndexOf('/') + 1).toLowerCase(); names.set(name, (names.get(name) ?? 0) + 1); }
    for (const f of files) {
      if (!isAttachment(f.path)) continue;
      const name = f.path.slice(f.path.lastIndexOf('/') + 1);
      const r = rank(name);
      // Nome repetido em outra pasta: o link leva o caminho, para achar o arquivo certo.
      if (r >= 0) out.push({ path: f.path, title: name, link: (names.get(name.toLowerCase()) ?? 0) > 1 ? f.path : name, file: true, rank: r + (embed ? 0 : 3) });
    }
    return out.sort((a, b) => a.rank - b.rank).slice(0, 8);
  }, [suggest, notes, files, tab.path, props.aliases]);
  const embed = useMemo(() => embedResolver(files), [files]);
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
  return <div className="note-scroll" ref={scroller} onPointerDown={event => { lastNote = event.currentTarget; }} onDoubleClick={() => { if (!tab.editing) props.onEditing(true); }}>
    {finding && <NoteFind query={finding.query} count={findCount} current={findCurrent} input={findInput}
      onQuery={value => setFinding(state => ({ query: value, current: 0, step: (state?.step ?? 0) + 1 }))} onStep={stepFind} onClose={closeFind} />}
    {note.banner ? <Banner key={note.banner.target} url={attachmentUrl(tab.root, note.banner.target)} settings={note.banner.height || !props.bannerHeight ? note.banner : { ...note.banner, height: props.bannerHeight }}
      onChange={tab.editing && !pasting ? (key, value) => change(changeBanner(latest.current, key, value)) : undefined}
      onReplace={tab.editing && !pasting ? () => setChoosingBanner(true) : undefined}
      onRemove={tab.editing && !pasting ? () => change(removeBanner(latest.current)) : undefined} />
      : <div className="note-banner-margin">
        <button type="button" className="ghost-button" disabled={pasting} aria-haspopup="dialog" onClick={() => setChoosingBanner(true)}>Adicionar banner</button>
      </div>}
    {choosingBanner && <BannerPicker root={tab.root} rootName={props.rootName} images={props.images} onPick={addBanner} onClose={() => setChoosingBanner(false)} />}
    <article className="note-page">
      {renaming !== null
        ? <form onSubmit={e => { e.preventDefault(); rename(renaming); }}><input className="athena-rename" autoFocus value={renaming} maxLength={120} placeholder="Título" aria-label="Título da nota (também o nome do arquivo)" onFocus={e => e.target.select()} onChange={e => setRenaming(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') { renamingNow.current = true; setRenaming(null); props.onTitled?.(); } }} onBlur={() => rename(renaming)} /></form>
        : <h1 className="note-title" title="Clique para renomear" onClick={() => { renamingNow.current = false; setRenaming(title); }}>{title}</h1>}
      <NoteAliases aliases={note.aliases} onChange={next => change(setAliases(latest.current, next))} />
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
              {suggestions.length > 0 && <ul className="link-suggest" role="listbox" aria-label="Notas para o link">{suggestions.map((n, k) => <li key={`${n.path}|${n.alias ?? ''}`} role="option" aria-selected={k === Math.min(highlight, suggestions.length - 1)}><button type="button" tabIndex={-1} onMouseDown={e => { e.preventDefault(); pick(n.link ?? n.title, n.alias); }}>{n.alias ?? n.title}<small>{n.alias ? `↳ ${n.title}` : folderOf(n.path)}</small></button></li>)}</ul>}
            </div>
            <Suspense fallback={<div className="athena-visual-editor" aria-busy="true" />}>
              <AthenaEditor editor={editor} root={tab.root} value={note.body} readOnly={pasting}
              autoFocus={renaming === null && !note.body.trim()} onChange={typed} onPaste={pasteImage}
              onBlur={() => { setTimeout(() => setSuggest(null), 150); void flush(); }} onEscape={() => setSuggest(null)} onSuggestKey={suggestKey} onLink={follow} onCaret={caretMoved} onReady={() => { if (restoring.current) restoring.current.caretSet = false; }} embed={embed} media={files} find={editorFind} />
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
            {note.body.trim() ? <Markdown text={note.body} onNoteLink={props.onFollow} media={fileMedia} onAttachment={props.onAttachment} onWebError={props.onError} embed={embed} /> : <p className="athena-empty">Nota vazia. Clique no lápis para escrever.</p>}
            {backlinks.length > 0 && <section className="athena-backlinks"><h2>Mencionada em</h2>{backlinks.map(b => <button key={b.path} className="link-button" onClick={() => props.onOpen(b.path)}>{b.title}</button>)}</section>}
          </>}
    </article>
  </div>;
}
