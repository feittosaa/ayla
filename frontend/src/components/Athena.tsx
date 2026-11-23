import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import Confirm from './Confirm';
import AthenaNote, { AttachmentView, type Note, type Tab } from './AthenaNote';
import { CalendarIcon, ChevronRightIcon, FileIcon, ImageIcon, VideoIcon, CloseIcon, ColumnsIcon, ExpandIcon, EyeIcon, GitIcon, ImportIcon, LockIcon, PencilIcon, UnlockIcon, PinIcon, PlusIcon, RefreshIcon, SidebarIcon, TrashIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import { preloadPdf } from '../services/PdfJs';
import { IMAGE, VIDEO, folderOf, isAttachment, titleOf } from '../services/Wiki';
import BrandMark from './BrandMark';
import AthenaText from './AthenaText';
import { ImportDialog } from './AthenaImport';
import { LibraryPanel } from './AthenaLibrary';
import AthenaReader from './AthenaReader';
import { bookRoot, isBookRoot, readingKey, seriesOf, sourceOf, useLibrary } from '../services/Library';
import { LockDialog, LockedView, UNLOCK_GRACE_MS, type LockMode, type LockResult } from './AthenaLock';
import AthenaGraph, { type NoteGraph } from './AthenaGraph';
import AthenaCalendar from './AthenaCalendar';

type Root = { id: string; kind: 'ayla' | 'vault' | 'project'; name: string; path: string; available: boolean; obsidian: boolean; git: boolean; folders: string[]; notes: Note[]; files?: Note[]; texts?: Note[]; bannerHeight?: number | null; protected?: Lock[]; lockedFiles?: FileLock[] };
/** Pasta protegida da raiz: aberta nesta execução (desde `since`, em segundos) ou bloqueada. */
type Lock = { folder: string; unlocked: boolean; since: number | null; recovery: boolean };
/** Arquivo protegido sozinho (nota, texto ou livro), pelo caminho original. */
type FileLock = { path: string; unlocked: boolean; since: number | null; recovery: boolean };
/** Item protegido: uma pasta (com o que tem dentro) ou um arquivo sozinho. */
type Guard = { path: string; file: boolean; unlocked: boolean; since: number | null; recovery: boolean };
type Tree = { roots: Root[]; suggested: string[]; git_enabled: boolean };
type Hit = { root: string; rootName: string; path: string; title: string; snippet: string };
type Outcome = { committed: number; pulled: boolean; pushed: boolean; message: string };
type Change = { path: string; kind: 'novo' | 'modificado' | 'apagado' | 'renomeado' | 'conflito' };
type GitStatus = { repo: boolean; branch: string; changes: number; files: Change[]; remote: boolean; upstream: boolean; ahead: number; behind: number; last_commit: { message: string; at: number } | null };
type GitInfo = { enabled: boolean; obsidian: boolean; status: GitStatus; last: { at: number; outcome: Outcome } | null };
type Adding = { root: string; folder: string; kind: 'note' | 'folder' };
type Folder = { name: string; path: string; folders: Folder[]; notes: Note[]; files: Note[]; locked: FileLock[] };
type Overview = { graph: NoteGraph };
type Pane = { tabs: Tab[]; active: number };
type NoteLocation = Pick<Tab, 'root' | 'path'>;
const noteKey = ({ root, path }: NoteLocation) => JSON.stringify([root, path]);
/** Chave de seleção de uma pasta (não se confunde com a de uma nota). */
const folderKey = ({ root, path }: NoteLocation) => JSON.stringify([root, path, 'pasta']);
/** O que está sendo arrastado: nota ou pasta da árvore, ou uma aba. */
type DragItem = { kind: 'note' | 'file' | 'folder'; root: string; path: string; label: string; tab?: { pane: number; at: number }; notes?: NoteLocation[]; folders?: NoteLocation[] };
/** Onde soltar: numa pasta (move o arquivo) ou numa tela (abre a nota; nas laterais, divide a tela). */
type Drop = { kind: 'folder'; root: string; path: string } | { kind: 'pane'; pane: number; side: 'center' | 'left' | 'right' };
/** Abas abertas (até duas telas lado a lado), a tela em foco e o painel do Git: ficam salvos neste computador. */
type Workspace = { panes: Pane[]; focused: number; panel: boolean };

/** Pastas e notas de uma raiz em árvore, em ordem alfabética (como o Obsidian). */
function build(root: Root): Folder {
  const top: Folder = { name: root.name, path: '', folders: [], notes: [], files: [], locked: [] };
  const find = (path: string): Folder => {
    if (!path) return top;
    const parent = find(folderOf(path));
    const name = path.slice(path.lastIndexOf('/') + 1);
    let folder = parent.folders.find(f => f.name === name);
    if (!folder) { folder = { name, path, folders: [], notes: [], files: [], locked: [] }; parent.folders.push(folder); }
    return folder;
  };
  root.folders.forEach(find);
  root.notes.forEach(note => find(folderOf(note.path)).notes.push(note));
  (root.files ?? []).forEach(file => find(folderOf(file.path)).files.push(file));
  (root.texts ?? []).forEach(text => find(folderOf(text.path)).files.push(text));
  (root.lockedFiles ?? []).forEach(file => find(folderOf(file.path)).locked.push(file));
  const sort = (folder: Folder) => { folder.folders.sort((a, b) => a.name.localeCompare(b.name, 'pt-BR')); folder.notes.sort((a, b) => a.title.localeCompare(b.title, 'pt-BR')); folder.folders.forEach(sort); };
  sort(top);
  return top;
}

/** Link [[alvo]] → nota da mesma raiz: pelo caminho ou pelo nome (o caminho mais curto, como o Obsidian). */
function resolve(root: Root, target: string): string | null {
  const wanted = target.trim().replace(/\.md$/i, '').toLowerCase();
  const matches = root.notes.filter(n => {
    const path = n.path.replace(/\.md$/i, '').toLowerCase();
    return path === wanted || path.endsWith(`/${wanted}`) || n.title.toLowerCase() === wanted;
  });
  matches.sort((a, b) => a.path.length - b.path.length);
  return matches[0]?.path ?? null;
}

const sectionLabel: Record<Root['kind'], string> = { ayla: '', vault: 'Cofres', project: 'Projetos' };
/** Avisa as Configurações que a lista de cofres mudou. */
const vaultsChanged = () => window.dispatchEvent(new Event('athena-vaults'));
const WORKSPACE = 'ayla-athena-workspace';
const empty: Workspace = { panes: [{ tabs: [], active: 0 }], focused: 0, panel: false };
/** Texto aberto pelo "Abrir com Athena" fora das raízes: vale só enquanto a Ayla estiver aberta. */
const isExternal = (root: string) => root.startsWith('aberto:');
/** Notas e Biblioteca têm abas e telas separadas: uma nota não aparece na Biblioteca e vice-versa. */
type View = 'notas' | 'biblioteca';
type Spaces = Record<View, Workspace>;
const SPACES = 'ayla-athena-workspaces';
/** Só as abas de um lado (livros ou notas), sem telas vazias além da primeira. */
function only(ws: Workspace, books: boolean): Workspace {
  let panes = ws.panes.map(p => {
    const current = p.tabs[p.active];
    const tabs = p.tabs.filter(t => isBookRoot(t.root) === books);
    const kept = current ? tabs.indexOf(current) : -1;
    return { tabs, active: kept >= 0 ? kept : Math.max(0, Math.min(p.active, tabs.length - 1)) };
  });
  if (panes.length > 1) panes = panes.filter(p => p.tabs.length);
  if (!panes.length) panes = [{ tabs: [], active: 0 }];
  return { ...ws, panes, focused: Math.min(ws.focused, panes.length - 1) };
}
function restoreSpaces(): Spaces {
  try {
    const saved = JSON.parse(localStorage.getItem(SPACES) ?? 'null') as Partial<Spaces> | null;
    if (saved) return { notas: only(parse(saved.notas), false), biblioteca: only(parse(saved.biblioteca), true) };
  } catch { /* volta ao formato antigo */ }
  // Antes, notas e livros dividiam as mesmas abas: separa.
  const old = restore();
  return { notas: only(old, false), biblioteca: { ...only(old, true), panel: false } };
}
function restore(): Workspace {
  try { return parse(JSON.parse(localStorage.getItem(WORKSPACE) ?? 'null')); } catch { return empty; }
}
function parse(saved: Workspace | null | undefined): Workspace {
  try {
    if (!saved || !Array.isArray(saved.panes) || !saved.panes.length) return empty;
    const panes = saved.panes.slice(0, 2).map(p => {
      const tabs = (Array.isArray(p.tabs) ? p.tabs : []).filter(t => typeof t?.root === 'string' && typeof t.path === 'string' && !isExternal(t.root)).map(t => ({ root: t.root, path: t.path, pinned: !!t.pinned, editing: !isAttachment(t.path) }));
      return { tabs, active: Math.min(Math.max(0, Number(p.active) || 0), Math.max(0, tabs.length - 1)) };
    });
    return { panes, focused: Math.min(Number(saved.focused) || 0, panes.length - 1), panel: !!saved.panel };
  } catch { return empty; }
}
/** Largura das barras laterais (esquerda: árvore; direita: painel do Git), salva neste computador. */
const LAYOUT = 'ayla-athena-layout';
const widths = { left: { initial: 288, min: 200, max: 560 }, right: { initial: 290, min: 220, max: 600 } };
type Side = keyof typeof widths;
const clampWidth = (side: Side, value: number) => Math.round(Math.min(widths[side].max, Math.max(widths[side].min, value)));
function restoreLayout(): Record<Side, number> {
  try {
    const saved = JSON.parse(localStorage.getItem(LAYOUT) ?? '{}') as Partial<Record<Side, number>>;
    return { left: clampWidth('left', Number(saved.left) || widths.left.initial), right: clampWidth('right', Number(saved.right) || widths.right.initial) };
  } catch { return { left: widths.left.initial, right: widths.right.initial }; }
}
const same = (tab: { root: string; path: string }, root: string, path: string) => tab.root === root && tab.path === path;
/** Título de um livro na aba: sem a extensão e com espaços no lugar de "_". */
const bookTitle = (path: string) => path.slice(path.lastIndexOf('/') + 1).replace(/\.(epub|cbz)$/i, '').replace(/_/g, ' ');
/** Aba visível na tela em foco. */
const shownTab = (ws: Workspace): Tab | undefined => { const pane = ws.panes[Math.min(ws.focused, ws.panes.length - 1)]; return pane?.tabs[pane.active]; };
/** Chaves da árvore que precisam estar abertas para mostrar a nota (raiz e pastas acima dela). */
function ancestors(root: string, path: string) {
  const parts = path.split('/').slice(0, -1);
  return [`${root}|`, ...parts.map((_, i) => `${root}|${parts.slice(0, i + 1).join('/')}`)];
}
/** Só as notas da raiz em foco, com os links renumerados. */
function graphOf(graph: NoteGraph, root: string): NoteGraph {
  const index = new Map<number, number>();
  const nodes = graph.nodes.filter((node, i) => node.root === root && index.set(i, index.size));
  const edges: [number, number][] = [];
  for (const [a, b] of graph.edges) {
    const from = index.get(a); const to = index.get(b);
    if (from !== undefined && to !== undefined) edges.push([from, to]);
  }
  return { nodes, edges };
}
function sameGraph(previous: NoteGraph, next: NoteGraph) {
  return previous.nodes.length === next.nodes.length && previous.edges.length === next.edges.length
    && previous.nodes.every((node, index) => {
      const other = next.nodes[index];
      return same(node, other.root, other.path) && node.title === other.title && node.rootName === other.rootName;
    })
    && previous.edges.every(([from, to], index) => from === next.edges[index][0] && to === next.edges[index][1]);
}
const changeLabel: Record<Change['kind'], string> = { novo: 'N', modificado: 'M', apagado: 'A', renomeado: 'R', conflito: '!' };
function ago(seconds: number) {
  const diff = Math.max(0, Date.now() / 1000 - seconds);
  if (diff < 60) return 'agora';
  if (diff < 3600) return `há ${Math.floor(diff / 60)} min`;
  if (diff < 86400) return `há ${Math.floor(diff / 3600)} h`;
  return new Date(seconds * 1000).toLocaleDateString('pt-BR');
}

/** Athena: notas em Markdown em árvore — notas da Athena, cofres (ex.: do Obsidian) e projetos —, com
 * abas (fixáveis), duas notas lado a lado, links entre notas, busca, nota do dia e painel do Git. */
export default function Athena({ active, onOpenSettings, external }: { active: boolean; onOpenSettings: () => void; external?: { id: number; paths: string[] } }) {
  /** Barra lateral: notas ou Biblioteca (livros e mangás), lembrada neste computador. */
  const [view, setView] = useState<View>(() => localStorage.getItem('ayla-athena-view') === 'biblioteca' ? 'biblioteca' : 'notas');
  useEffect(() => { localStorage.setItem('ayla-athena-view', view); }, [view]);
  const viewRef = useRef(view); viewRef.current = view;
  const [spaces, setSpaces] = useState<Spaces>(restoreSpaces);
  const spacesRef = useRef(spaces); spacesRef.current = spaces;
  const workspace = spaces[view];
  /** Abas e telas do lado à vista (ou do lado pedido). */
  const setWorkspace = useCallback((update: (ws: Workspace) => Workspace, target?: View) => setSpaces(prev => {
    const key = target ?? viewRef.current;
    return { ...prev, [key]: update(prev[key]) };
  }), []);
  /** Renomear, mover e apagar valem para os dois lados. */
  const setEveryWorkspace = useCallback((update: (ws: Workspace) => Workspace) => setSpaces(prev => ({ notas: update(prev.notas), biblioteca: update(prev.biblioteca) })), []);
  const opened = shownTab(spaces.notas);
  const [tree, setTree] = useState<Tree | null>(null);
  const [expanded, setExpanded] = useState<string[]>(() => ['ayla|', ...(opened ? ancestors(opened.root, opened.path) : [])]);
  const [query, setQuery] = useState(''); const [hits, setHits] = useState<Hit[] | null>(null);
  const [error, setError] = useState(''); const [adding, setAdding] = useState<Adding | null>(null); const [name, setName] = useState('');
  useEffect(() => { if (active) preloadPdf(); }, [active]);
  /** Nota recém-criada que abre pedindo o título. */
  const [fresh, setFresh] = useState<string | null>(null);
  const [folderEdit, setFolderEdit] = useState<{ root: string; path: string; name: string } | null>(null);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [graphError, setGraphError] = useState('');
  const overviewRequest = useRef(0);
  /** Raiz em foco: a da nota aberta ao entrar na Athena (não sempre a Athena). */
  const [focus, setFocus] = useState(() => opened?.root ?? 'ayla'); const [sidebar, setSidebar] = useState(true);
  const [statuses, setStatuses] = useState(['', '']); const [confirming, setConfirming] = useState<number | null>(null);
  /** Nota a apagar pela lixeira da barra lateral, à espera da confirmação no modal. */
  const [trashing, setTrashing] = useState<{ root: string; path: string; title: string } | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [batchBusy, setBatchBusy] = useState(false);
  const [batchNotice, setBatchNotice] = useState('');
  const selectionAnchor = useRef<string | null>(null);
  const [revision, setRevision] = useState(0); const [saved, setSaved] = useState(0);
  /** Diálogo de senha de uma pasta protegida. */
  const [lockDialog, setLockDialog] = useState<{ mode: LockMode; root: string; path: string; file: boolean } | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const { library, load: loadLibrary, saveProgress, saveMode, markBooks } = useLibrary();
  /** Importar notas (.txt/.md) para uma pasta. */
  const [importing, setImporting] = useState<{ root: string; folder: string } | null>(null);
  const [git, setGit] = useState<GitInfo | null>(null); const [syncing, setSyncing] = useState(false);
  const flushers = useRef<((() => Promise<void>) | null)[]>([null, null]);
  const [drag, setDrag] = useState<{ item: DragItem; x: number; y: number; drop: Drop | null } | null>(null);
  const swallow = useRef(false);
  const [layout, setLayout] = useState(restoreLayout); const [resizing, setResizing] = useState(false);
  useEffect(() => { localStorage.setItem(LAYOUT, JSON.stringify(layout)); }, [layout]);
  /** Alça entre as colunas: arrastar ajusta a largura; setas também; duplo clique volta ao padrão. */
  function handle(side: Side, label: string) {
    const grow = side === 'left' ? 1 : -1;
    const start = (e: ReactPointerEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      const origin = e.clientX; const base = layout[side];
      setResizing(true);
      const move = (ev: PointerEvent) => setLayout(prev => ({ ...prev, [side]: clampWidth(side, base + (ev.clientX - origin) * grow) }));
      const up = () => { setResizing(false); window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
      window.addEventListener('pointermove', move); window.addEventListener('pointerup', up);
    };
    return <div className="resize-handle" role="separator" aria-orientation="vertical" aria-label={label} title={`${label} (duplo clique: tamanho padrão)`} tabIndex={0}
      aria-valuemin={widths[side].min} aria-valuemax={widths[side].max} aria-valuenow={layout[side]}
      onPointerDown={start} onDoubleClick={() => setLayout(prev => ({ ...prev, [side]: widths[side].initial }))}
      onKeyDown={e => { const step = e.key === 'ArrowRight' ? 16 : e.key === 'ArrowLeft' ? -16 : 0; if (step) { e.preventDefault(); setLayout(prev => ({ ...prev, [side]: clampWidth(side, prev[side] + step * grow) })); } }} />;
  }

  const load = useCallback(() => { void native<Tree>('athena_tree').then(setTree).catch(e => setError(errorText(e))); }, []);
  const loadOverview = useCallback(() => {
    const request = ++overviewRequest.current;
    void native<Overview>('athena_overview').then(result => {
      if (request !== overviewRequest.current) return;
      setOverview(current => current && sameGraph(current.graph, result.graph) ? { ...result, graph: current.graph } : result);
      setGraphError('');
    }, error => { if (request === overviewRequest.current) setGraphError(errorText(error)); });
  }, []);
  const rootOf = useCallback((id: string) => tree?.roots.find(r => r.id === id) ?? null, [tree]);
  const selectableNotes = useMemo(() => {
    const notes = new Map<string, NoteLocation>();
    for (const root of tree?.roots ?? []) {
      if (!root.available) continue;
      const paths = [...root.notes, ...(root.texts ?? []), ...(root.lockedFiles ?? []).filter(file => file.unlocked && /\.(md|txt)$/i.test(file.path))];
      for (const { path } of paths) {
        const note = { root: root.id, path };
        notes.set(noteKey(note), note);
      }
    }
    return notes;
  }, [tree]);
  const selectedNotes = selected.flatMap(key => {
    const note = selectableNotes.get(key);
    return note ? [note] : [];
  });
  // Pastas selecionáveis: as visíveis de cada raiz disponível, menos as protegidas e bloqueadas.
  const selectableFolders = useMemo(() => {
    const folders = new Map<string, NoteLocation>();
    for (const root of tree?.roots ?? []) {
      if (!root.available) continue;
      for (const path of root.folders) {
        if (root.protected?.some(l => !l.unlocked && (path === l.folder || path.startsWith(`${l.folder}/`)))) continue;
        folders.set(folderKey({ root: root.id, path }), { root: root.id, path });
      }
    }
    return folders;
  }, [tree]);
  const selectedFolders = selected.flatMap(key => {
    const folder = selectableFolders.get(key);
    return folder ? [folder] : [];
  });
  const selectionCount = selectedNotes.length + selectedFolders.length;
  const selectionLabel = [selectedNotes.length && `${selectedNotes.length} ${selectedNotes.length === 1 ? 'nota' : 'notas'}`, selectedFolders.length && `${selectedFolders.length} ${selectedFolders.length === 1 ? 'pasta' : 'pastas'}`].filter(Boolean).join(' e ');
  /** Item protegido que contém o caminho (pasta) ou é ele (arquivo), nas Notas ou na Biblioteca. */
  const lockOf = useCallback((root: string, path: string): Guard | undefined => {
    if (root === 'biblioteca' || isBookRoot(root)) {
      const found = library?.sources.find(s => s.id === (root === 'biblioteca' ? 'ayla' : sourceOf(root)))?.locks
        ?.find(l => l.kind === 'file' ? l.path === path : path === l.path || path.startsWith(`${l.path}/`));
      return found && { path: found.path, file: found.kind === 'file', unlocked: found.unlocked, since: found.since, recovery: found.recovery };
    }
    const folder = rootOf(root)?.protected?.find(l => path === l.folder || path.startsWith(`${l.folder}/`));
    if (folder) return { path: folder.folder, file: false, unlocked: folder.unlocked, since: folder.since, recovery: folder.recovery };
    const file = rootOf(root)?.lockedFiles?.find(l => l.path === path);
    return file && { ...file, file: true };
  }, [rootOf, library]);
  const expand = useCallback((root: string, path: string) => {
    setExpanded(prev => [...new Set([...prev, ...ancestors(root, path)])]);
  }, []);
  useEffect(() => { localStorage.setItem(SPACES, JSON.stringify(spaces)); }, [spaces]);
  useEffect(() => { if (active) { load(); loadOverview(); } }, [active, load, loadOverview]);

  const panes = workspace.panes;
  const focused = Math.min(workspace.focused, panes.length - 1);
  const activeTab = panes[focused].tabs[panes[focused].active] as Tab | undefined;
  // A raiz em foco acompanha a nota que passa a ficar visível (trocar de aba, fechar, abrir pela conversa).
  const activeRoot = activeTab?.root;
  // Calendário no lugar das notas; abrir uma nota (pela lateral, conversa ou calendário) volta às notas.
  const [calendar, setCalendar] = useState(false);
  const activeKey = activeTab ? `${activeTab.root}|${activeTab.path}` : '';
  useEffect(() => { if (activeKey) setCalendar(false); }, [activeKey]);
  useEffect(() => { if (activeRoot && !isBookRoot(activeRoot) && !isExternal(activeRoot)) setFocus(activeRoot); }, [activeRoot]);
  // A Biblioteca carrega quando aparece na lateral ou quando há um livro aberto.
  const readingBook = spaces.biblioteca.panes.some(p => p.tabs.length > 0);
  /** Modo Biblioteca: a lateral na Biblioteca (só os livros ficam nas abas). */
  const libraryMode = view === 'biblioteca';
  useEffect(() => {
    if (active && !library && (view === 'biblioteca' || readingBook)) void loadLibrary().catch(e => setError(errorText(e)));
  }, [active, library, view, readingBook, loadLibrary]);
  // Cofre fechado ou indisponível fora da Ayla: volta para a Athena.
  useEffect(() => { if (tree && !tree.roots.some(r => r.id === focus && r.available)) setFocus('ayla'); }, [tree, focus]);
  const graph = overview?.graph;
  // Pastas protegidas: livres por 10 min depois da senha; depois disso, bloqueiam assim que nenhuma
  // nota ou arquivo delas estiver à vista (outra nota, aba fechada, outro espaço da Ayla).
  useEffect(() => { const id = setInterval(() => setClock(Date.now()), 15_000); return () => clearInterval(id); }, []);
  const shownTabs = useMemo(() => active ? workspace.panes.map(p => p.tabs[p.active]).filter((t): t is Tab => Boolean(t)) : [], [active, workspace.panes]);
  useEffect(() => {
    const guards: { root: string; tabRoot: string; guard: Guard }[] = [];
    for (const root of tree?.roots ?? []) {
      for (const l of root.protected ?? []) guards.push({ root: root.id, tabRoot: root.id, guard: { path: l.folder, file: false, unlocked: l.unlocked, since: l.since, recovery: l.recovery } });
      for (const l of root.lockedFiles ?? []) guards.push({ root: root.id, tabRoot: root.id, guard: { ...l, file: true } });
    }
    for (const l of library?.sources.find(s => s.id === 'ayla')?.locks ?? []) {
      guards.push({ root: 'biblioteca', tabRoot: bookRoot('ayla'), guard: { path: l.path, file: l.kind === 'file', unlocked: l.unlocked, since: l.since, recovery: l.recovery } });
    }
    for (const { root, tabRoot, guard } of guards) {
      if (!guard.unlocked || clock - (guard.since ?? 0) * 1000 < UNLOCK_GRACE_MS) continue;
      const inside = shownTabs.some(t => t.root === tabRoot && (t.path === guard.path || (!guard.file && t.path.startsWith(`${guard.path}/`))));
      if (!inside) void lockItem(root, guard.path);
    }
  }, [tree, library, shownTabs, clock]); // eslint-disable-line react-hooks/exhaustive-deps
  const focusedGraph = useMemo(() => graph ? graphOf(graph, focus) : null, [graph, focus]);

  /** Abre a nota na tela em foco: na aba atual (se não estiver fixada) ou numa aba nova. */
  const open = useCallback((root: string, path: string, options: { newTab?: boolean; edit?: boolean } = {}) => {
    setError('');
    // Livros abrem em abas como as notas, mas não mudam a raiz de notas em foco.
    if (!isBookRoot(root) && !isExternal(root)) { setFocus(root); expand(root, path); }
    // Livro abre na Biblioteca; nota, em Notas (trocando o lado à vista, se preciso).
    const target: View = isBookRoot(root) ? 'biblioteca' : 'notas';
    if (target !== viewRef.current) { viewRef.current = target; setView(target); }
    setWorkspace(ws => {
      const index = Math.min(ws.focused, ws.panes.length - 1);
      const pane = ws.panes[index];
      const found = pane.tabs.findIndex(t => same(t, root, path));
      let tabs = pane.tabs; let at: number;
      if (found >= 0) { at = found; if (options.edit) tabs = tabs.map((t, k) => k === found ? { ...t, editing: true } : t); }
      else {
        const tab: Tab = { root, path, pinned: false, editing: !isAttachment(path) };
        const current = pane.tabs[pane.active];
        if (!current || options.newTab || current.pinned) { at = current ? pane.active + 1 : 0; tabs = [...tabs.slice(0, at), tab, ...tabs.slice(at)]; }
        else { at = pane.active; tabs = tabs.map((t, k) => k === at ? tab : t); }
      }
      return { ...ws, focused: index, panes: ws.panes.map((p, k) => k === index ? { tabs, active: at } : p) };
    }, target);
  }, [expand, setWorkspace]);
  const updatePane = (index: number, change: (pane: Pane) => Pane) => setWorkspace(ws => {
    const next = ws.panes.map((p, k) => k === index ? change(p) : p);
    if (next.length > 1 && !next[index].tabs.length) { const rest = next.filter((_, k) => k !== index); return { ...ws, panes: rest, focused: 0 }; }
    return { ...ws, panes: next };
  });
  const closeTab = (index: number, at: number) => updatePane(index, pane => {
    const tabs = pane.tabs.filter((_, k) => k !== at);
    return { tabs, active: Math.max(0, Math.min(at < pane.active ? pane.active - 1 : pane.active, tabs.length - 1)) };
  });
  // "Abrir com Athena": nota de uma raiz abre nela; texto de fora abre numa aba que grava o próprio arquivo.
  const [outside, setOutside] = useState<Record<string, string>>({});
  const handledOpen = useRef(0);
  useEffect(() => {
    if (!external || external.id === handledOpen.current) return;
    handledOpen.current = external.id;
    void (async () => {
      for (const path of external.paths) {
        try {
          const found = await native<{ root: string; path: string; name: string }>('athena_open_external', { path });
          if (isExternal(found.root)) setOutside(prev => ({ ...prev, [found.root]: found.name }));
          open(found.root, found.path, { newTab: true });
        } catch (e) { setError(errorText(e)); }
      }
    })();
  }, [external, open]);
  const setTab = (index: number, at: number, change: Partial<Tab>) => updatePane(index, pane => ({ ...pane, tabs: pane.tabs.map((t, k) => k === at ? { ...t, ...change } : t) }));
  const activate = (index: number, at: number) => {
    const tab = panes[index]?.tabs[at];
    if (tab) setFocus(tab.root);
    setWorkspace(ws => ({ ...ws, focused: index, panes: ws.panes.map((p, k) => k === index ? { ...p, active: at } : p) }));
  };
  /** Troca o caminho de notas abertas (renomear ou mover nota ou pasta). */
  // Notas fixadas no topo da lateral: só nas notas da própria Athena (cofres e projetos não).
  const [pinnedNotes, setPinnedNotes] = useState<string[]>(() => { try { const v = JSON.parse(localStorage.getItem('ayla-athena-pinned') ?? '[]'); return Array.isArray(v) ? v.filter((p): p is string => typeof p === 'string') : []; } catch { return []; } });
  useEffect(() => { try { localStorage.setItem('ayla-athena-pinned', JSON.stringify(pinnedNotes)); } catch { /* sem armazenamento */ } }, [pinnedNotes]);
  const togglePinned = (path: string) => setPinnedNotes(prev => prev.includes(path) ? prev.filter(p => p !== path) : [...prev, path]);
  // Pastas fixadas no topo, também só nas notas da Athena.
  const [pinnedFolders, setPinnedFolders] = useState<string[]>(() => { try { const v = JSON.parse(localStorage.getItem('ayla-athena-pinned-folders') ?? '[]'); return Array.isArray(v) ? v.filter((p): p is string => typeof p === 'string') : []; } catch { return []; } });
  useEffect(() => { try { localStorage.setItem('ayla-athena-pinned-folders', JSON.stringify(pinnedFolders)); } catch { /* sem armazenamento */ } }, [pinnedFolders]);
  const togglePinnedFolder = (path: string) => setPinnedFolders(prev => prev.includes(path) ? prev.filter(p => p !== path) : [...prev, path]);
  const moved = (from: string, to: string, folder: boolean) => (p: string) => folder && (p === from || p.startsWith(`${from}/`)) ? `${to}${p.slice(from.length)}` : !folder && p === from ? to : p;
  const retarget = (root: string, from: string, to: string, folder = false) => {
    if (root === 'ayla') { setPinnedNotes(prev => prev.map(moved(from, to, folder))); if (folder) setPinnedFolders(prev => prev.map(moved(from, to, true))); }
    retargetTabs(root, from, to, folder);
  };
  const retargetTabs = (root: string, from: string, to: string, folder = false) => setEveryWorkspace(ws => ({ ...ws, panes: ws.panes.map(p => ({ ...p, tabs: p.tabs.map(t => t.root !== root ? t
    : folder && t.path.startsWith(`${from}/`) ? { ...t, path: `${to}${t.path.slice(from.length)}` }
    : !folder && t.path === from ? { ...t, path: to } : t) })) }));
  /** Fecha as abas que não existem mais (nota apagada, cofre fechado). */
  const drop = (gone: (tab: Tab) => boolean) => setEveryWorkspace(ws => {
    let next = ws.panes.map(p => {
      const current = p.tabs[p.active];
      const tabs = p.tabs.filter(t => !gone(t));
      const kept = current ? tabs.indexOf(current) : -1;
      return { tabs, active: kept >= 0 ? kept : Math.max(0, Math.min(p.active, tabs.length - 1)) };
    });
    if (next.length > 1) next = next.filter(p => p.tabs.length) as Pane[];
    if (!next.length) next = [{ tabs: [], active: 0 }];
    return { ...ws, panes: next, focused: Math.min(ws.focused, next.length - 1) };
  });
  /** Abre (ou leva a aba arrastada) numa tela; nas laterais, com uma tela só, cria a segunda. */
  const openAt = (root: string, path: string, target: { pane: number; side: 'center' | 'left' | 'right' }, from?: { pane: number; at: number }) => {
    setFocus(root); expand(root, path);
    setWorkspace(ws => {
      let list = ws.panes.map(p => ({ ...p, tabs: [...p.tabs] }));
      let tab: Tab = { root, path, pinned: false, editing: !isAttachment(path) };
      if (from && list[from.pane]?.tabs[from.at]) {
        const source = list[from.pane];
        tab = source.tabs[from.at]; source.tabs.splice(from.at, 1);
        source.active = Math.max(0, Math.min(from.at < source.active ? source.active - 1 : source.active, source.tabs.length - 1));
      }
      let destination: Pane;
      if (target.side !== 'center' && list.length === 1) {
        destination = { tabs: [tab], active: 0 };
        list = target.side === 'left' ? [destination, ...list] : [...list, destination];
      } else {
        destination = list[Math.min(target.pane, list.length - 1)];
        const found = destination.tabs.findIndex(t => same(t, root, path));
        if (found >= 0) destination.active = found;
        else { const at = destination.tabs.length ? destination.active + 1 : 0; destination.tabs.splice(at, 0, tab); destination.active = at; }
      }
      if (list.length > 1) list = list.filter(p => p.tabs.length || p === destination);
      return { ...ws, panes: list, focused: Math.max(0, list.indexOf(destination)) };
    });
  };
  const split = () => setWorkspace(ws => ws.panes.length > 1 ? { ...ws, panes: [ws.panes[0]], focused: 0 } : { ...ws, panes: [...ws.panes, { tabs: [], active: 0 }], focused: 1 });

  // Nota salva na Athena separada (outra janela): as abas com ela releem (sem alterações pendentes).
  useEffect(() => {
    let me = 'main';
    try { me = getCurrentWebviewWindow().label; } catch { /* fora do Tauri */ }
    const stop = listen<{ root: string; path: string; from: string }>('note-saved', event => {
      const { root, path, from } = event.payload;
      if (from !== me && Object.values(spacesRef.current).some(ws => ws.panes.some(p => p.tabs.some(t => same(t, root, path))))) setRevision(r => r + 1);
    });
    return () => { void stop.then(off => off()); };
  }, []);
  useEffect(() => {
    const stop = listen<{ root?: string; path?: string }>('notes-updated', event => {
      load(); loadOverview(); setRevision(r => r + 1);
      if (event.payload?.root && event.payload.path) open(event.payload.root, event.payload.path, { newTab: true });
    });
    return () => { void stop.then(off => off()); };
  }, [load, loadOverview, open]);
  useEffect(() => {
    if (!query.trim()) return;
    let alive = true;
    const id = setTimeout(() => void native<Hit[]>('athena_search', { query }).then(
      results => { if (alive) setHits(results); }, e => { if (alive) setError(errorText(e)); },
    ), 250);
    return () => { alive = false; clearTimeout(id); };
  }, [query, revision]);

  // Painel do Git: segue a raiz em foco; atualiza ao abrir, ao trocar de raiz e pouco depois de salvar.
  const gitRoot = rootOf(focus);
  const refreshGit = useCallback(() => {
    if (!workspace.panel || !gitRoot) return;
    void native<GitInfo>('athena_git_status', { root: gitRoot.id }).then(setGit).catch(() => setGit(null));
  }, [workspace.panel, gitRoot]);
  useEffect(() => { setGit(null); }, [focus]);
  useEffect(() => { const id = setTimeout(refreshGit, saved ? 1200 : 0); return () => clearTimeout(id); }, [refreshGit, saved, revision]);
  useEffect(() => { window.addEventListener('focus', refreshGit); return () => window.removeEventListener('focus', refreshGit); }, [refreshGit]);

  const onSaved = useCallback(() => { load(); loadOverview(); setSaved(n => n + 1); }, [load, loadOverview]);
  async function flushAll() { await Promise.all(flushers.current.map(f => f?.())); }

  /** Nova pasta: cria com o nome digitado; ao clicar fora sem nome, desiste. */
  async function submit(event?: FormEvent) {
    event?.preventDefault();
    if (!adding) return;
    const { root, folder } = adding; setError('');
    setAdding(null); setName('');
    if (!event && !name.trim()) return;
    try {
      const made = await native<string>('athena_mkdir', { root, parent: folder, name: name.trim() || 'Nova pasta' });
      setExpanded(prev => [...prev, `${root}|${made}`, `${root}|${folder}`]); load();
    } catch (e) { setError(errorText(e)); }
  }
  /** Nova nota como no Obsidian: criada na hora como "Nova nota" e aberta pedindo o título,
   * que vira o nome do arquivo. */
  async function newNote(root: string, folder: string) {
    setError('');
    try {
      const made = await native<string>('athena_create', { root, folder, title: 'Nova nota', empty: true });
      load(); loadOverview(); setFresh(`${root}|${made}`); open(root, made, { edit: true });
    } catch (e) { setError(errorText(e)); }
  }
  async function today() {
    const root = rootOf(focus)?.available ? focus : 'ayla';
    try { const daily = await native<string>('athena_daily', { root }); load(); loadOverview(); open(root, daily); } catch (e) { setError(errorText(e)); }
  }
  /** Nota de um dia do calendário (só nas notas da Athena), criada se ainda não existe. */
  async function openDay(date: string) {
    try {
      const daily = await native<string>('athena_daily', { root: 'ayla', date });
      load(); loadOverview(); setCalendar(false); open('ayla', daily);
    } catch (e) { setError(errorText(e)); }
  }
  async function remove(index: number) {
    const tab = panes[index]?.tabs[panes[index].active];
    if (tab) await trashNote(tab.root, tab.path, index);
  }
  /** Manda a nota para a lixeira (.trash) da raiz, gravando antes o que estiver pendente. */
  async function trashNote(root: string, path: string, pane?: number) {
    try {
      if (pane === undefined) await flushAll(); else await flushers.current[pane]?.();
      await native('athena_trash', { root, path });
      setConfirming(null); setTrashing(null); drop(t => same(t, root, path)); load(); loadOverview();
    } catch (e) { setTrashing(null); setError(errorText(e)); }
  }
  async function addVault(path?: string) {
    setError('');
    try {
      const folder = path ?? await native<string | null>('athena_pick_folder');
      if (!folder) return;
      await native('athena_add_vault', { path: folder });
      setExpanded(prev => [...prev, `vault:${folder}|`]); setFocus(`vault:${folder}`); load(); loadOverview(); vaultsChanged();
    } catch (e) { setError(errorText(e)); }
  }
  async function closeVault(root: Root) {
    try {
      await native('athena_remove_vault', { path: root.path });
      drop(t => t.root === root.id); if (focus === root.id) setFocus('ayla'); load(); loadOverview(); vaultsChanged();
    } catch (e) { setError(errorText(e)); }
  }
  async function sync() {
    if (!gitRoot) return;
    setSyncing(true); setError('');
    try {
      await flushAll();
      const outcome = await native<Outcome>('athena_git_sync', { root: gitRoot.id });
      setStatuses([outcome.message, outcome.message]); load(); loadOverview(); setRevision(r => r + 1);
    } catch (e) { setError(errorText(e)); } finally { setSyncing(false); }
  }
  /** Renomeia ou move a nota da aba (o núcleo atualiza os links nas outras notas). */
  async function moveNote(index: number, tab: { root: string; path: string }, to: string) {
    setError('');
    try {
      await flushAll();
      const result = await native<{ path: string; relinked: number }>('athena_rename', { root: tab.root, path: tab.path, to });
      retarget(tab.root, tab.path, result.path); expand(tab.root, result.path); load(); loadOverview(); setRevision(r => r + 1);
      const text = result.relinked ? `Links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}` : folderOf(result.path) !== folderOf(tab.path) ? 'Nota movida' : 'Nota renomeada';
      setStatuses(prev => prev.map((s, k) => k === index ? text : s));
    } catch (e) { setError(errorText(e)); }
  }
  async function changeSelected(notes: NoteLocation[], destination?: Extract<Drop, { kind: 'folder' }>, folders: NoteLocation[] = []) {
    if (batchBusy) return;
    setBatchBusy(true); setBatchNotice(''); setError('');
    const handled = new Set<string>();
    const moved = new Map<string, string>();
    const failed: string[] = [];
    let count = 0;
    let folderCount = 0;
    try {
      await flushAll();
      // Pastas primeiro (só as de cima: a de dentro vai junto). As notas dentro delas também vão junto.
      const top = folders.filter(f => !folders.some(o => o.root === f.root && f.path.startsWith(`${o.path}/`)));
      const inside = (item: NoteLocation) => top.some(f => f.root === item.root && item.path.startsWith(`${f.path}/`));
      for (const folder of top) {
        const key = folderKey(folder);
        try {
          if (destination) {
            if (folderOf(folder.path) === destination.path) { handled.add(key); continue; }
            if (destination.path === folder.path || destination.path.startsWith(`${folder.path}/`)) { failed.push(`${folder.path} — não dá para mover uma pasta para dentro dela mesma`); continue; }
            const name = folder.path.slice(folder.path.lastIndexOf('/') + 1);
            const result = await native<{ path: string }>('athena_rename_folder', { root: folder.root, path: folder.path, to: destination.path ? `${destination.path}/${name}` : name });
            retarget(folder.root, folder.path, result.path, true);
          } else {
            await native('athena_trash_folder', { root: folder.root, path: folder.path });
            drop(t => t.root === folder.root && t.path.startsWith(`${folder.path}/`));
          }
          handled.add(key); folderCount++;
        } catch (e) { failed.push(`${folder.path} — ${errorText(e)}`); }
      }
      for (const folder of folders.filter(f => !top.includes(f))) handled.add(folderKey(folder));
      for (const note of notes.filter(n => { if (inside(n)) { handled.add(noteKey(n)); return false; } return true; })) {
        const { root, path } = note;
        const key = noteKey(note);
        if (destination?.path === folderOf(path)) { handled.add(key); continue; }
        try {
          if (destination) {
            const name = path.slice(path.lastIndexOf('/') + 1);
            const to = destination.path ? `${destination.path}/${name}` : name;
            const next = /\.txt$/i.test(path)
              ? await native<string>('athena_move_file', { root, path, folder: destination.path })
              : (await native<{ path: string }>('athena_rename', { root, path, to })).path;
            moved.set(key, next);
          } else {
            await native('athena_trash', { root, path });
          }
          handled.add(key);
          count++;
        } catch (e) { failed.push(`${path} — ${errorText(e)}`); }
      }
      if (destination) {
        setEveryWorkspace(ws => ({ ...ws, panes: ws.panes.map(pane => ({ ...pane, tabs: pane.tabs.map(tab => {
          const path = moved.get(noteKey(tab));
          return path ? { ...tab, path } : tab;
        }) })) }));
        const first = moved.values().next().value;
        if (first) expand(destination.root, first);
      } else {
        drop(tab => handled.has(noteKey(tab)));
      }
      setSelected(previous => previous.filter(key => !handled.has(key)));
      selectionAnchor.current = null;
      if (count || folderCount) {
        setRevision(r => r + 1);
        const total = count + folderCount;
        const what = [count && (count === 1 ? '1 nota' : `${count} notas`), folderCount && (folderCount === 1 ? '1 pasta' : `${folderCount} pastas`)].filter(Boolean).join(' e ');
        const action = destination ? (total === 1 ? 'movida' : 'movidas') : (total === 1 ? 'enviada à lixeira' : 'enviadas à lixeira');
        setBatchNotice(`${what} ${action}.`);
      }
      if (failed.length) setError(`Falha em ${failed.length} ${failed.length === 1 ? 'item' : 'itens'}. ${failed[0]}`);
      load(); loadOverview();
    } catch (e) { setError(errorText(e)); } finally { setBatchBusy(false); }
  }
  async function renameFolder(event: FormEvent) {
    event.preventDefault();
    if (!folderEdit) return;
    const { root, path, name: next } = folderEdit;
    const parent = folderOf(path);
    const to = next.includes('/') ? next : (parent ? `${parent}/${next}` : next);
    try {
      await flushAll();
      const result = await native<{ path: string; relinked: number }>('athena_rename_folder', { root, path, to });
      setFolderEdit(null); setExpanded(prev => [...prev, `${root}|${result.path}`]); retarget(root, path, result.path, true); load(); loadOverview(); setRevision(r => r + 1);
      if (result.relinked) setStatuses(prev => prev.map(() => `Pasta renomeada · links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}`));
    } catch (e) { setError(errorText(e)); }
  }
  /** Pasta ou tela sob o ponteiro, se aceitar o item (mesma raiz, não para dentro de si). */
  function findDrop(item: DragItem, x: number, y: number): Drop | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-drop]');
    if (!el) return null;
    if (el.dataset.drop === 'folder') {
      const root = el.dataset.root ?? ''; const path = el.dataset.path ?? '';
      if (item.notes || item.folders) {
        const items = [...(item.notes ?? []), ...(item.folders ?? [])];
        const intoItself = (item.folders ?? []).some(f => path === f.path || path.startsWith(`${f.path}/`));
        return !intoItself && items.every(i => i.root === root) && items.some(i => folderOf(i.path) !== path) ? { kind: 'folder', root, path } : null;
      }
      if (root !== item.root || folderOf(item.path) === path || (item.kind === 'note' && isAttachment(item.path))) return null;
      if (item.kind === 'folder' && (path === item.path || path.startsWith(`${item.path}/`))) return null;
      return { kind: 'folder', root, path };
    }
    if (el.dataset.drop === 'pane' && item.kind !== 'folder' && !item.notes && !item.folders) {
      const pane = Number(el.dataset.pane); const box = el.getBoundingClientRect();
      let side: 'center' | 'left' | 'right' = 'center';
      if (panes.length === 1 && panes[0].tabs.length && !(item.tab && panes[0].tabs.length === 1)) {
        if (x < box.left + box.width * 0.25) side = 'left'; else if (x > box.right - box.width * 0.25) side = 'right';
      }
      if (side === 'center' && item.tab?.pane === pane) return null;
      return { kind: 'pane', pane, side };
    }
    return null;
  }
  async function dropItem(item: DragItem, drop: Drop) {
    if (drop.kind === 'pane') { openAt(item.root, item.path, drop, item.tab); return; }
    if (item.notes || item.folders) { await changeSelected(item.notes ?? [], drop, item.folders ?? []); return; }
    const name = item.path.slice(item.path.lastIndexOf('/') + 1);
    const to = drop.path ? `${drop.path}/${name}` : name;
    if (item.kind === 'file') {
      setError('');
      try {
        await flushAll();
        const moved = await native<string>('athena_move_file', { root: item.root, path: item.path, folder: drop.path });
        retarget(item.root, item.path, moved); setExpanded(prev => [...new Set([...prev, `${drop.root}|${drop.path}`])]); load();
      } catch (e) { setError(errorText(e)); }
      return;
    }
    if (item.kind === 'note') { await moveNote(focused, item, to.replace(/\.md$/i, '')); setExpanded(prev => [...new Set([...prev, `${drop.root}|${drop.path}`])]); return; }
    setError('');
    try {
      await flushAll();
      const result = await native<{ path: string; relinked: number }>('athena_rename_folder', { root: item.root, path: item.path, to });
      retarget(item.root, item.path, result.path, true); setExpanded(prev => [...new Set([...prev, `${drop.root}|${drop.path}`, `${item.root}|${result.path}`])]); load(); setRevision(r => r + 1);
    } catch (e) { setError(errorText(e)); }
  }
  /** Arrastar com o ponteiro (funciona igual no Linux e no Windows): começa depois de 6 px. */
  const startDrag = (item: DragItem) => (e: ReactPointerEvent) => {
    if (batchBusy || e.button !== 0 || (e.target as HTMLElement).closest('input, form')) return;
    const sx = e.clientX; const sy = e.clientY; let moving = false; let last: Drop | null = null;
    const move = (ev: PointerEvent) => {
      if (!moving && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
      if (!moving) window.getSelection()?.removeAllRanges();
      moving = true; last = findDrop(item, ev.clientX, ev.clientY);
      setDrag({ item, x: ev.clientX, y: ev.clientY, drop: last });
    };
    const finish = (cancelled: boolean) => {
      window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); window.removeEventListener('pointercancel', cancel); window.removeEventListener('keydown', key);
      if (!moving) return;
      swallow.current = true; setTimeout(() => { swallow.current = false; }, 0);
      setDrag(null);
      if (!cancelled && last) void dropItem(item, last);
    };
    const up = () => finish(false);
    const cancel = () => finish(true);
    const key = (ev: KeyboardEvent) => { if (ev.key === 'Escape') cancel(); };
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up); window.addEventListener('pointercancel', cancel); window.addEventListener('keydown', key);
  };
  const dropping = (root: string, path: string) => drag?.drop?.kind === 'folder' && drag.drop.root === root && drag.drop.path === path;
  const isSelected = (root: string, path: string) => selected.includes(noteKey({ root, path }));
  function startNoteDrag(item: DragItem) {
    const chosen = item.kind === 'folder' ? selected.includes(folderKey(item)) : isSelected(item.root, item.path);
    if (!chosen) return startDrag(item);
    return startDrag({ ...item, notes: selectedNotes, folders: selectedFolders, label: selectionCount === 1 ? item.label : selectionLabel });
  }
  function selectDot(root: string, path: string, title: string, folder = false) {
    const key = folder ? folderKey({ root, path }) : noteKey({ root, path });
    if (!(folder ? selectableFolders.has(key) : selectableNotes.has(key))) return null;
    const checked = selected.includes(key);
    const label = checked ? `Tirar ${title} da seleção` : `Selecionar ${title}`;
    return <button type="button" className="select-dot" role="checkbox" aria-checked={checked} aria-label={label} title={label}
      data-note-selection={key} disabled={batchBusy} onPointerDown={e => e.stopPropagation()} onClick={e => {
        e.stopPropagation();
        const visible = e.shiftKey ? Array.from(e.currentTarget.closest('nav')?.querySelectorAll<HTMLElement>('[data-note-selection]') ?? [], row => row.dataset.noteSelection!) : [];
        const from = visible.indexOf(selectionAnchor.current ?? '');
        const to = visible.indexOf(key);
        setSelected(previous => from >= 0 && to >= 0
          ? [...new Set([...previous, ...visible.slice(Math.min(from, to), Math.max(from, to) + 1)])]
          : checked ? previous.filter(value => value !== key) : [...previous, key]);
        selectionAnchor.current = key;
      }} />;
  }
  function noteHit({ root, path, title, rootName, snippet }: Omit<Hit, 'snippet'> & { snippet?: string }) {
    const checked = isSelected(root, path);
    return <div key={noteKey({ root, path })} className={`tree-row note hit${checked ? ' checked' : ''}`} role="treeitem" aria-selected={checked}
      onPointerDown={startNoteDrag({ kind: 'note', root, path, label: title })} onClick={openFromTree(root, path)} onAuxClick={openFromTree(root, path)} title={path}>
      <span className="tree-name">{title}</span><small>{rootName}{snippet ? ` · ${snippet}` : ''}</small>{selectDot(root, path, title)}
    </div>;
  }

  /** Clique num link [[…]]: abre a nota da mesma raiz ou, como no Obsidian, cria se ainda não existe. */
  async function follow(tab: Tab, target: string) {
    const root = rootOf(tab.root);
    if (isAttachment(target)) { open(tab.root, target.split('#')[0].trim()); return; }
    const found = root && resolve(root, target);
    if (found) { open(tab.root, found); return; }
    try {
      const made = await native<string>('athena_create', { root: tab.root, folder: folderOf(target), title: target.slice(target.lastIndexOf('/') + 1) });
      load(); loadOverview(); open(tab.root, made, { edit: true });
    } catch (e) { setError(errorText(e)); }
  }
  const openFromTree = (root: string, path: string) => (e: ReactMouseEvent) => { if (e.button === 1 || e.ctrlKey || e.metaKey) e.preventDefault(); open(root, path, { newTab: e.button === 1 || e.ctrlKey || e.metaKey }); };

  const toggle = (key: string) => setExpanded(prev => prev.includes(key) ? prev.filter(k => k !== key) : [...prev, key]);
  const roots = useMemo(() => tree?.roots ?? [], [tree]);

  function addForm(root: string, folder: string, depth: number) {
    if (!adding || adding.root !== root || adding.folder !== folder) return null;
    return <form className="tree-add" style={{ paddingLeft: 12 + depth * 14 }} onSubmit={e => void submit(e)}>
      <input autoFocus value={name} maxLength={120} placeholder="Nome da pasta" onChange={e => setName(e.target.value)} onBlur={() => void submit()} onKeyDown={e => { if (e.key === 'Escape') { setAdding(null); setName(''); } }} />
    </form>;
  }
  async function removeFolder(root: string, folder: string) {
    setError('');
    try {
      await flushAll();
      await native('athena_trash_folder', { root, path: folder });
      drop(t => t.root === root && t.path.startsWith(`${folder}/`)); load(); loadOverview();
    } catch (e) { setError(errorText(e)); }
  }
  /** Recarrega a árvore das notas ou a Biblioteca, conforme a raiz do item. */
  const reloadFor = (root: string) => { if (root === 'biblioteca') void loadLibrary().catch(e => setError(errorText(e))); else { load(); loadOverview(); } };
  /** Bloqueia de novo: salva o que estiver pendente, tira a chave da memória e fecha a pasta na árvore. */
  async function lockItem(root: string, path: string) {
    await flushAll();
    try { await native('athena_lock', { root, path }); } catch (e) { setError(errorText(e)); }
    setExpanded(prev => prev.filter(key => key !== `${root}|${path}` && !key.startsWith(`${root}|${path}/`)));
    reloadFor(root); setRevision(r => r + 1);
  }
  /** Envia o diálogo de senha; devolve a chave de recuperação global quando ela acaba de ser criada. */
  async function submitLock(mode: LockMode, secret: string, next: string): Promise<LockResult> {
    if (!lockDialog) return;
    const { root, path, file } = lockDialog;
    let recovery: string | undefined;
    if (mode === 'protect') {
      await flushAll();
      recovery = (await native<{ count: number; recovery: string | null }>('athena_protect', { root, path, password: secret, file })).recovery ?? undefined;
      // Abas do item continuam abertas, agora pedindo a senha.
      setExpanded(prev => prev.filter(key => key !== `${root}|${path}` && !key.startsWith(`${root}|${path}/`)));
    } else if (mode === 'unlock') {
      await unlockItem(root, path, secret);
      // Arquivo protegido: abre logo depois da senha certa (na Biblioteca, no leitor).
      if (file) open(root === 'biblioteca' ? bookRoot('ayla') : root, path);
    } else if (mode === 'recover') {
      await native('athena_recover', { root, path, code: secret, password: next });
      showOpened(root, path);
      if (file) open(root === 'biblioteca' ? bookRoot('ayla') : root, path);
    } else if (mode === 'password') {
      await native('athena_change_password', { root, path, current: secret, password: next });
    } else {
      await flushAll();
      await native('athena_unprotect', { root, path, password: secret });
    }
    reloadFor(root); setRevision(r => r + 1);
    return recovery;
  }
  async function unlockItem(root: string, path: string, password: string) {
    await native('athena_unlock', { root, path, password });
    showOpened(root, path);
  }
  /** Item aberto (pela senha ou pela chave de recuperação): foco nele e árvore aberta. */
  function showOpened(root: string, path: string) {
    setClock(Date.now());
    if (root !== 'biblioteca') { setFocus(root); setExpanded(prev => [...new Set([...prev, `${root}|`, `${root}|${path}`])]); }
    reloadFor(root); setRevision(r => r + 1);
  }
  /** Ferramentas de senha de uma pasta ou arquivo: proteger; aberto, bloquear, trocar a senha e tirar a proteção. */
  function lockTools(root: string, path: string, guard: Guard | undefined, file = false): ReactNode {
    const stop = (action: () => void) => (e: { stopPropagation: () => void }) => { e.stopPropagation(); action(); };
    const label = path.slice(path.lastIndexOf('/') + 1);
    if (!guard) return <button className="icon-button tiny" title="Proteger com senha" aria-label={`Proteger ${label} com senha`} onClick={stop(() => setLockDialog({ mode: 'protect', root, path, file }))}><LockIcon size={13} /></button>;
    if (guard.path !== path || !guard.unlocked) return null;
    return <>
      <button className="icon-button tiny" title="Bloquear agora" aria-label={`Bloquear ${label}`} onClick={stop(() => void lockItem(root, path))}><LockIcon size={13} /></button>
      <button className="icon-button tiny" title="Trocar a senha" aria-label={`Trocar a senha de ${label}`} onClick={stop(() => setLockDialog({ mode: 'password', root, path, file: guard.file }))}>✱</button>
      <button className="icon-button tiny" title="Tirar a proteção" aria-label={`Tirar a proteção de ${label}`} onClick={stop(() => setLockDialog({ mode: 'unprotect', root, path, file: guard.file }))}><UnlockIcon size={13} /></button>
    </>;
  }
  function tools(root: string, folder: string): ReactNode {
    const start = (kind: Adding['kind']) => (e: { stopPropagation: () => void }) => { e.stopPropagation(); setFocus(root); setAdding({ root, folder, kind }); setName(''); setExpanded(prev => [...new Set([...prev, `${root}|${folder}`])]); };
    return <span className="tree-tools">
      <button className="icon-button tiny" title="Nova nota aqui" aria-label="Nova nota aqui" onClick={e => { e.stopPropagation(); setFocus(root); setExpanded(prev => [...new Set([...prev, `${root}|${folder}`])]); void newNote(root, folder); }}><PlusIcon size={14} /></button>
      <button className="icon-button tiny" title="Nova pasta aqui" aria-label="Nova pasta aqui" onClick={start('folder')}>⊞</button>
      <button className="icon-button tiny" title="Importar .txt e .md para cá" aria-label="Importar notas para cá" onClick={e => { e.stopPropagation(); setFocus(root); setImporting({ root, folder }); }}><ImportIcon size={13} /></button>
      {folder && <Confirm className="icon-button tiny" title="Apagar a pasta (vai para a lixeira do cofre)" ariaLabel={`Apagar a pasta ${folder}`} question={`Apagar a pasta "${folder.slice(folder.lastIndexOf('/') + 1)}" e tudo o que tem dentro? Ela vai para a lixeira (.trash) da raiz.`} confirm="Apagar" onConfirm={() => void removeFolder(root, folder)}><TrashIcon size={13} /></Confirm>}
      {folder && <button className="icon-button tiny" title="Renomear ou mover a pasta" aria-label="Renomear ou mover a pasta" onClick={e => { e.stopPropagation(); setFolderEdit({ root, path: folder, name: folder.slice(folder.lastIndexOf('/') + 1) }); }}>✎</button>}
      {folder && lockTools(root, folder, lockOf(root, folder))}
    </span>;
  }
  function folderNode(root: Root, folder: Folder, depth: number): ReactNode {
    const key = `${root.id}|${folder.path}`;
    const lock = folder.path ? root.protected?.find(l => l.folder === folder.path) : undefined;
    // Bloqueada: sempre fechada; o clique pede a senha.
    if (lock && !lock.unlocked) {
      return <div key={key} role="group" className="tree-drop" data-drop="locked">
        <div className="tree-row folder locked" style={{ paddingLeft: 4 + depth * 14 }} role="treeitem" aria-expanded={false} title="Pasta protegida: clique para digitar a senha"
          onClick={() => { setFocus(root.id); setLockDialog({ mode: 'unlock', root: root.id, path: folder.path, file: false }); }}>
          <span className="tree-caret"><LockIcon size={13} /></span><span className="tree-name">{folder.name}</span>
        </div>
      </div>;
    }
    const shown = !folder.path || expanded.includes(key);
    const inner = folder.path ? depth + 1 : depth;
    return <div key={key} role="group" className={folder.path && dropping(root.id, folder.path) ? 'tree-drop drop-target' : 'tree-drop'} data-drop={folder.path ? 'folder' : undefined} data-root={root.id} data-path={folder.path}>
      {folder.path && (folderEdit?.root === root.id && folderEdit.path === folder.path
        ? <form className="tree-add" style={{ paddingLeft: 4 + depth * 14 }} onSubmit={e => void renameFolder(e)}>
            <input autoFocus value={folderEdit.name} maxLength={200} title="Novo nome, ou caminho (ex.: Jogos/RPG) para mover" onChange={e => setFolderEdit({ ...folderEdit, name: e.target.value })} onKeyDown={e => { if (e.key === 'Escape') setFolderEdit(null); }} />
          </form>
        : <div className={`tree-row folder${selected.includes(folderKey({ root: root.id, path: folder.path })) ? ' checked' : ''}${root.kind === 'ayla' && pinnedFolders.includes(folder.path) ? ' pinned' : ''}`} style={{ paddingLeft: 4 + depth * 14 }} onPointerDown={startNoteDrag({ kind: 'folder', root: root.id, path: folder.path, label: folder.name })} onClick={() => { setFocus(root.id); toggle(key); }} role="treeitem" aria-expanded={shown} aria-selected={selected.includes(folderKey({ root: root.id, path: folder.path }))}>
            <span className={shown ? 'tree-caret open' : 'tree-caret'}><ChevronRightIcon size={13} /></span><span className="tree-name">{folder.name}</span>{lock && <span className="tree-lock" title="Pasta protegida, aberta"><UnlockIcon size={12} /></span>}{tools(root.id, folder.path)}{root.kind === 'ayla' && pinButton(folder.name, pinnedFolders.includes(folder.path), () => togglePinnedFolder(folder.path))}{selectDot(root.id, folder.path, folder.name, true)}
          </div>)}
      {shown && <>
        {addForm(root.id, folder.path, inner)}
        {folder.folders.filter(child => root.kind !== 'ayla' || !pinnedFolders.includes(child.path)).map(child => folderNode(root, child, inner))}
        {[...folder.notes.filter(n => root.kind !== 'ayla' || !pinnedNotes.includes(n.path)).map(n => ({ ...n, file: false })), ...folder.files.map(f => ({ ...f, file: true }))].sort((a, b) => a.title.localeCompare(b.title, 'pt-BR')).map(item => item.file
          ? <div key={item.path} role="treeitem" className={`tree-row note file${isSelected(root.id, item.path) ? ' checked' : ''}`} aria-selected={/\.txt$/i.test(item.path) ? isSelected(root.id, item.path) : undefined} aria-current={activeTab && same(activeTab, root.id, item.path) ? 'page' : undefined} style={{ paddingLeft: 20 + inner * 14 }} title={`${item.path}\nClique para abrir; arraste para uma pasta para mover`} onPointerDown={startNoteDrag({ kind: 'file', root: root.id, path: item.path, label: item.title })} onClick={openFromTree(root.id, item.path)} onAuxClick={openFromTree(root.id, item.path)}>
              <span className="tree-file-icon">{/\.(pdf|txt)$/i.test(item.path) ? <FileIcon size={13} /> : VIDEO.test(item.path) ? <VideoIcon size={13} /> : <ImageIcon size={13} />}</span><span className="tree-name">{item.title.replace(/\.[^.]+$/, '')}</span><small>{item.title.slice(item.title.lastIndexOf('.') + 1).toUpperCase()}</small>
              {/\.txt$/i.test(item.path) && !lockOf(root.id, item.path) && <button className="icon-button tiny tree-trash tree-protect" title="Proteger com senha" aria-label={`Proteger ${item.title} com senha`} onPointerDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); setLockDialog({ mode: 'protect', root: root.id, path: item.path, file: true }); }}><LockIcon size={13} /></button>}
              {selectDot(root.id, item.path, item.title)}
            </div>
          : noteRow(root, item, inner))}
        {/* Arquivos protegidos sozinhos: bloqueados, o clique pede a senha; abertos, abrem como nota. */}
        {folder.locked.map(file => {
          const name = file.path.slice(file.path.lastIndexOf('/') + 1);
          const title = name.replace(/\.md$/i, '');
          const selectable = selectableNotes.has(noteKey({ root: root.id, path: file.path }));
          return <div key={`trancado|${file.path}`} role="treeitem" className={`tree-row note${file.unlocked ? '' : ' locked'}${selectable && isSelected(root.id, file.path) ? ' checked' : ''}`}
            aria-selected={selectable ? isSelected(root.id, file.path) : undefined} aria-current={file.unlocked && activeTab && same(activeTab, root.id, file.path) ? 'page' : undefined}
            style={{ paddingLeft: 20 + inner * 14 }} title={file.unlocked ? `${file.path}\nProtegido com senha, aberto` : `${file.path}\nProtegido com senha: clique para digitar a senha`}
            onPointerDown={selectable && isSelected(root.id, file.path) ? startNoteDrag({ kind: 'note', root: root.id, path: file.path, label: title }) : undefined}
            onClick={file.unlocked ? openFromTree(root.id, file.path) : () => setLockDialog({ mode: 'unlock', root: root.id, path: file.path, file: true })}>
            <span className="tree-file-icon"><LockIcon size={12} /></span><span className="tree-name">{title}</span>
            {file.unlocked && <span className="tree-tools">{lockTools(root.id, file.path, lockOf(root.id, file.path), true)}</span>}
            {selectDot(root.id, file.path, title)}
          </div>;
        })}
      </>}
    </div>;
  }
  /** Pasta pelo caminho, dentro da árvore montada. */
  function findFolder(folder: Folder, path: string): Folder | undefined {
    if (folder.path === path) return folder;
    for (const child of folder.folders) { if (path === child.path || path.startsWith(`${child.path}/`)) return findFolder(child, path); }
    return undefined;
  }
  /** Pino de fixar no topo: sempre no mesmo lugar (antes da bolinha de seleção), em notas e pastas. */
  function pinButton(title: string, pinned: boolean, toggle: () => void): ReactNode {
    return <button className="icon-button tiny tree-trash tree-pin" title={pinned ? 'Desafixar do topo' : 'Fixar no topo'} aria-label={pinned ? `Desafixar ${title}` : `Fixar ${title} no topo`} aria-pressed={pinned}
      onPointerDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); toggle(); }}><PinIcon size={12} /></button>;
  }
  /** Linha de uma nota na árvore (também usada pelas fixadas no topo da Athena). */
  function noteRow(root: Root, item: { path: string; title: string }, inner: number, pinned = false): ReactNode {
    return <div key={item.path} role="treeitem" className={`tree-row note${isSelected(root.id, item.path) ? ' checked' : ''}${pinned ? ' pinned' : ''}`} aria-selected={isSelected(root.id, item.path)} aria-current={activeTab && same(activeTab, root.id, item.path) ? 'page' : undefined} style={{ paddingLeft: 20 + inner * 14 }} title={`${item.path}\nCtrl+clique ou botão do meio: abrir em nova aba. Arraste para uma pasta (mover) ou para a tela (abrir ao lado)`} onPointerDown={startNoteDrag({ kind: 'note', root: root.id, path: item.path, label: item.title })} onClick={openFromTree(root.id, item.path)} onAuxClick={openFromTree(root.id, item.path)}>
          <span className="tree-name">{item.title}</span>
          {!lockOf(root.id, item.path) && <button className="icon-button tiny tree-trash tree-protect" title="Proteger com senha" aria-label={`Proteger ${item.title} com senha`} onPointerDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); setLockDialog({ mode: 'protect', root: root.id, path: item.path, file: true }); }}><LockIcon size={13} /></button>}
          <button className="icon-button tiny tree-trash" title="Apagar nota" aria-label={`Apagar a nota ${item.title}`} onPointerDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); setTrashing({ root: root.id, path: item.path, title: item.title }); }}><TrashIcon size={13} /></button>
          {root.kind === 'ayla' && pinButton(item.title, pinned, () => togglePinned(item.path))}
          {selectDot(root.id, item.path, item.title)}
        </div>;
  }
  function rootNode(root: Root): ReactNode {
    const key = `${root.id}|`;
    const shown = expanded.includes(key);
    return <div key={root.id} className={dropping(root.id, '') ? 'tree-root tree-drop drop-target' : 'tree-root tree-drop'} data-drop={root.available ? 'folder' : undefined} data-root={root.id} data-path="">
      <div className={focus === root.id ? 'tree-row root focused' : 'tree-row root'} onClick={() => { if (!root.available) return; if (focus === root.id) toggle(key); else { setFocus(root.id); setExpanded(prev => prev.includes(key) ? prev : [...prev, key]); } }} role="treeitem" aria-expanded={shown} title={focus === root.id ? `${root.path}\nEm foco: novas notas e a nota de hoje vão para cá` : root.path}>
        <span className={shown ? 'tree-caret open' : 'tree-caret'}><ChevronRightIcon size={13} /></span>
        <span className="tree-name">{root.name}</span>
        {!root.available && <small>indisponível</small>}
        {root.git && <small title="Repositório Git">git</small>}
        {root.available && tools(root.id, '')}
        {/* Fechar o cofre: só com o mouse em cima, como as outras ferramentas da linha. */}
        {root.kind === 'vault' && <span className="tree-tools"><Confirm className="icon-button tiny" title="Fechar cofre (os arquivos ficam onde estão)" ariaLabel={`Fechar o cofre ${root.name}`} question="Fechar este cofre na Athena? Os arquivos ficam onde estão." confirm="Fechar" onConfirm={() => void closeVault(root)}>×</Confirm></span>}
      </div>
      {shown && root.available && root.kind === 'ayla' && pinnedFolders.map(path => findFolder(build(root), path)).filter((f): f is Folder => Boolean(f)).map(f => folderNode(root, f, 1))}
      {shown && root.available && root.kind === 'ayla' && root.notes.filter(n => pinnedNotes.includes(n.path)).sort((a, b) => pinnedNotes.indexOf(a.path) - pinnedNotes.indexOf(b.path)).map(n => noteRow(root, { path: n.path, title: n.title ?? titleOf(n.path) }, 1, true))}
      {shown && root.available && folderNode(root, build(root), 1)}
    </div>;
  }

  /** Livro aberto numa aba: o leitor, com o progresso e o modo da série. */
  function bookView(tab: Tab): ReactNode {
    const source = sourceOf(tab.root);
    if (!library) return <div className="note-scroll" aria-busy="true" />;
    const guard = lockOf(tab.root, tab.path);
    if (guard && !guard.unlocked) {
      return <LockedView key={`${tab.root}|${guard.path}`} folder={guard.path} file={guard.file} recovery={guard.recovery}
        onUnlock={password => unlockItem('biblioteca', guard.path, password)} onForgot={() => setLockDialog({ mode: 'recover', root: 'biblioteca', path: guard.path, file: guard.file })} />;
    }
    const book = library.sources.find(s => s.id === source)?.books.find(b => b.path === tab.path);
    if (!book) return <div className="athena-welcome"><p>Este livro não está mais na Biblioteca (a pasta pode ter sido desconectada ou o arquivo, movido).</p></div>;
    return <AthenaReader key={`${tab.root}|${tab.path}`} source={source} book={book} series={seriesOf(library, source, book.series)}
      progress={library.reading.books[readingKey(source, book.path)]} mode={library.reading.modes[readingKey(source, book.series)] ?? 'rtl'}
      onProgress={(position, fraction, done) => void saveProgress(source, book.path, { position, fraction, done }).catch(e => setError(errorText(e)))}
      onMode={mode => void saveMode(source, book.series, mode).catch(e => setError(errorText(e)))}
      onOpen={path => open(tab.root, path)} />;
  }
  function paneView(pane: Pane, index: number) {
    const tab = pane.tabs[pane.active] as Tab | undefined;
    const root = tab ? rootOf(tab.root) : null;
    const last = index === panes.length - 1;
    const tabLock = tab && !isBookRoot(tab.root) ? lockOf(tab.root, tab.path) : undefined;
    const lockedTab = tabLock && !tabLock.unlocked ? tabLock : undefined;
    const zone = drag?.drop?.kind === 'pane' && drag.drop.pane === index ? drag.drop.side : null;
    return <section key={index} data-drop="pane" data-pane={index} className={panes.length > 1 && index === focused ? 'note-pane focused' : 'note-pane'} onMouseDownCapture={() => { if (index !== focused) { setWorkspace(ws => ({ ...ws, focused: index })); if (tab) setFocus(tab.root); } }} aria-label={panes.length > 1 ? (index ? 'Tela da direita' : 'Tela da esquerda') : 'Nota'}>
      {zone && <div className={`drop-zone ${zone}`} aria-hidden="true"><span>{zone === 'center' ? 'Abrir nesta tela' : 'Abrir ao lado'}</span></div>}
      <header className="note-tabs organ-bar">
        {index === 0 && !sidebar && <button className="icon-button" data-tip="Mostrar a barra lateral" aria-label="Mostrar a barra lateral" onClick={() => setSidebar(true)}><SidebarIcon size={18} /></button>}
        <div className="tab-strip" role="tablist" aria-label="Notas abertas">
          {pane.tabs.map((t, k) => <div key={`${t.root}|${t.path}`} role="tab" aria-selected={k === pane.active} className={['note-tab', k === pane.active ? 'active' : '', t.pinned ? 'pinned' : ''].join(' ')} title={`${rootOf(t.root)?.name ?? outside[t.root] ?? ''} / ${t.path}`} onPointerDown={startDrag({ kind: isAttachment(t.path) ? 'file' : 'note', root: t.root, path: t.path, label: titleOf(t.path), tab: { pane: index, at: k } })} onClick={() => activate(index, k)} onAuxClick={e => { if (e.button === 1 && !t.pinned) closeTab(index, k); }} onDoubleClick={() => setTab(index, k, { pinned: !t.pinned })}>
            {/* O pino fica sempre à esquerda do título: fixa (com o mouse em cima) e mostra a aba fixada. */}
            <button className="tab-button tab-pin-button" aria-label={t.pinned ? 'Desafixar' : 'Fixar'} aria-pressed={t.pinned} data-tip={t.pinned ? 'Desafixar' : 'Fixar'} onClick={e => { e.stopPropagation(); setTab(index, k, { pinned: !t.pinned }); }}><PinIcon size={12} /></button>
            <span className="tab-title">{isBookRoot(t.root) ? bookTitle(t.path) : titleOf(t.path)}</span>
            {!t.pinned && <button className="tab-button" aria-label="Fechar aba" data-tip="Fechar" onClick={e => { e.stopPropagation(); closeTab(index, k); }}><CloseIcon size={12} /></button>}
          </div>)}
        </div>
        <span className="athena-status" role="status">{statuses[index]}</span>
        {tab && !isAttachment(tab.path) && !lockedTab && !isBookRoot(tab.root) && <div className="note-actions">
          <div className="note-mode" role="group" aria-label="Modo da nota">
            <button className={tab.editing ? 'icon-button' : 'icon-button active'} aria-pressed={!tab.editing} aria-label="Ver" data-tip="Ver" onClick={() => setTab(index, pane.active, { editing: false })}><EyeIcon size={18} /></button>
            <button className={tab.editing ? 'icon-button active' : 'icon-button'} aria-pressed={tab.editing} aria-label="Editar" data-tip="Editar" onClick={() => setTab(index, pane.active, { editing: true })}><PencilIcon size={17} /></button>
          </div>
          {root?.obsidian && !tabLock && !/\.txt$/i.test(tab.path) && <button className="icon-button" data-tip="Abrir no Obsidian" aria-label="Abrir no Obsidian, com os plugins do cofre" onClick={() => void (flushers.current[index]?.() ?? Promise.resolve()).then(() => native('athena_open_obsidian', { root: tab.root, path: tab.path })).catch(e => setError(errorText(e)))}><ExpandIcon size={17} /></button>}
          {isExternal(tab.root) ? null : confirming === index
            ? <span className="row-actions"><button onClick={() => void remove(index)}>Mover para a lixeira</button><button className="ghost-button" onClick={() => setConfirming(null)}>Cancelar</button></span>
            : <button className="icon-button" aria-label="Apagar nota" data-tip="Apagar nota" onClick={() => setConfirming(index)}><TrashIcon size={17} /></button>}
        </div>}
        <button className={panes.length > 1 ? 'icon-button active' : 'icon-button'} data-tip={panes.length > 1 ? 'Fechar esta tela' : 'Dividir a tela'} aria-label={panes.length > 1 ? 'Fechar esta tela' : 'Dividir a tela'} onClick={() => panes.length > 1 ? setWorkspace(ws => ({ ...ws, panes: ws.panes.filter((_, k) => k !== index), focused: 0 })) : split()}>{panes.length > 1 ? <CloseIcon size={17} /> : <ColumnsIcon size={17} />}</button>
        {last && !libraryMode && <button className={workspace.panel ? 'icon-button tip-end active' : 'icon-button tip-end'} data-tip={workspace.panel ? 'Fechar o painel do Git' : 'Painel do Git'} aria-label={workspace.panel ? 'Fechar o painel do Git' : 'Abrir o painel do Git'} aria-pressed={workspace.panel} onClick={() => setWorkspace(ws => ({ ...ws, panel: !ws.panel }))}><GitIcon size={18} /></button>}
      </header>
      {tab && isBookRoot(tab.root)
        ? bookView(tab)
        : tab && !tree
        ? <div className="note-scroll" aria-busy="true" />
        : tab && lockedTab
        ? <LockedView key={`${tab.root}|${lockedTab.path}`} folder={lockedTab.path} file={lockedTab.file} recovery={lockedTab.recovery} onUnlock={password => unlockItem(tab.root, lockedTab.path, password)} onForgot={() => setLockDialog({ mode: 'recover', root: tab.root, path: lockedTab.path, file: lockedTab.file })} />
        : tab && /\.txt$/i.test(tab.path)
        ? <AthenaText key={`${tab.root}|${tab.path}`} tab={tab} revision={revision} onSaved={onSaved}
            onStatus={text => setStatuses(prev => prev[index] === text ? prev : prev.map((s, k) => k === index ? text : s))}
            onError={setError} register={flush => { flushers.current[index] = flush; }} />
        : tab && isAttachment(tab.path)
        ? <AttachmentView key={`${tab.root}|${tab.path}`} root={tab.root} target={tab.path} onError={setError} />
        : tab
        ? <AthenaNote key={`${tab.root}|${tab.path}`} tab={tab} notes={root?.notes ?? []} images={tabLock?.file ? [] : (root?.files ?? []).filter(file => IMAGE.test(file.path) && lockOf(tab.root, file.path)?.path === tabLock?.path)} rootName={root?.name ?? 'Athena'} bannerHeight={root?.bannerHeight ?? undefined} revision={revision}
            onEditing={editing => setTab(index, pane.active, { editing })}
            onFollow={target => void follow(tab, target)}
            onOpen={path => open(tab.root, path)}
            onAttachment={target => open(tab.root, target, { newTab: true })}
            onRename={name => moveNote(index, tab, `${folderOf(tab.path) ? `${folderOf(tab.path)}/` : ''}${name}`)}
            askTitle={fresh === `${tab.root}|${tab.path}`} onTitled={() => setFresh(null)}
            onSaved={onSaved}
            onStatus={text => setStatuses(prev => prev[index] === text ? prev : prev.map((s, k) => k === index ? text : s))}
            onError={setError}
            register={flush => { flushers.current[index] = flush; }} />
        : index === 0 && view === 'notas'
        ? <div className="athena-welcome">
            <h1>Athena</h1>
            <p>Suas notas em Markdown, em pastas, como no Obsidian: as notas da Athena, os seus cofres e uma pasta para cada projeto da Ayla. Links como [[nota]] funcionam entre as notas.</p>
            <p>Clique numa nota para abrir; Ctrl+clique abre em nova aba. Arraste uma nota para uma pasta para movê-la, ou para a lateral da tela para ver duas notas lado a lado. Fixe uma aba para ela continuar aberta.</p>
            <div className="row-actions"><button className="primary-action" onClick={() => void addVault()}>Abrir cofre</button><button onClick={() => void today()}>Nota de hoje</button><button className="ghost-button" onClick={onOpenSettings}>Configurações</button></div>
          </div>
        : <div className="athena-welcome"><p>{view === 'biblioteca' ? 'Escolha um livro na Biblioteca para ler nesta tela.' : 'Escolha uma nota na barra lateral para abrir nesta tela.'}</p></div>}
    </section>;
  }

  function gitPanel() {
    const status = git?.status;
    return <section className="athena-git-panel" aria-label="Painel do Git">
      <header className="panel-head"><GitIcon size={16} /><b>{gitRoot?.name ?? 'Athena'}</b>
        <button className="icon-button" data-tip="Atualizar" aria-label="Atualizar o status" onClick={refreshGit}><RefreshIcon size={15} /></button>
        <button className="icon-button tip-end" data-tip="Fechar" aria-label="Fechar o painel" onClick={() => setWorkspace(ws => ({ ...ws, panel: false }))}><CloseIcon size={15} /></button>
      </header>
      <section className="panel-section">
        <h3>Sincronização</h3>
        {!git ? <p className="panel-muted">Consultando…</p>
          : !status?.repo ? <p className="panel-muted">{gitRoot?.kind === 'vault' ? 'Este cofre não é um repositório Git.' : 'Estas notas não estão num repositório Git.'}</p>
          : !git.enabled ? <><p className="panel-muted">O versionamento com Git está desligado.</p><button className="ghost-button" onClick={onOpenSettings}>Abrir Configurações</button></>
          : <>
              <dl className="panel-facts">
                <div><dt>Ramo</dt><dd>{status.branch || '—'}</dd></div>
                <div><dt>GitHub</dt><dd>{!status.remote ? 'sem remoto' : !status.upstream ? 'ainda não enviado' : status.ahead || status.behind ? [status.ahead && `${status.ahead} para enviar`, status.behind && `${status.behind} para receber`].filter(Boolean).join(' · ') : 'em dia'}</dd></div>
                {status.last_commit && <div><dt>Último commit</dt><dd title={status.last_commit.message}>{status.last_commit.message} <small>{ago(status.last_commit.at)}</small></dd></div>}
                {git.last && <div><dt>Última sincronização</dt><dd>{git.last.outcome.message || '—'} <small>{ago(git.last.at)}</small></dd></div>}
              </dl>
              {git.obsidian && <p className="panel-muted">Obsidian aberto: a sincronização automática fica com o plugin de lá.</p>}
              <button className="primary-action panel-sync" disabled={syncing} onClick={() => void sync()}>{syncing ? 'Sincronizando…' : status.remote ? 'Sincronizar com o GitHub' : 'Versionar'}</button>
            </>}
      </section>
      {status?.repo && <section className="panel-section panel-files">
        <h3>Alterações {status.changes ? <small>{status.changes}</small> : null}</h3>
        {status.files.length
          ? <ul>{status.files.map(f => {
              const note = /\.md$/i.test(f.path) && f.kind !== 'apagado' && gitRoot?.notes.some(n => n.path === f.path);
              return <li key={f.path} className={`change ${f.kind}`} title={`${f.kind}: ${f.path}`}>
                <span className="change-kind" aria-label={f.kind}>{changeLabel[f.kind]}</span>
                {note && gitRoot ? <button className="link-button" onClick={() => open(gitRoot.id, f.path)}>{f.path}</button> : <span>{f.path}</span>}
              </li>;
            })}</ul>
          : <p className="panel-muted">Nenhum arquivo alterado.</p>}
        {status.changes > status.files.length && <p className="panel-muted">E mais {status.changes - status.files.length}.</p>}
      </section>}
    </section>;
  }

  const sections: Root['kind'][] = ['ayla', 'project', 'vault'];
  return <div className={['chat-layout athena', drag ? 'dragging' : '', resizing ? 'resizing' : ''].join(' ')} onClickCapture={e => { if (swallow.current) { e.stopPropagation(); e.preventDefault(); } }}>
    {drag && <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>{drag.item.kind === 'folder' ? '📁 ' : ''}{drag.item.label}{drag.drop?.kind === 'folder' && <small> → {drag.drop.path ? drag.drop.path.slice(drag.drop.path.lastIndexOf('/') + 1) : rootOf(drag.drop.root)?.name}</small>}</div>}
    {sidebar && <aside className="sidebar organ-sidebar athena-sidebar" aria-label="Notas da Athena" style={{ flexBasis: layout.left, width: layout.left }}>
      <div className="brand"><BrandMark name="athena" size={26} /><span className="brand-name">Athena</span><button className="icon-button organ-hide" data-tip="Esconder a barra lateral" aria-label="Esconder a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon size={18} /></button></div>
      <div className="athena-switch" role="tablist" aria-label="Notas ou Biblioteca">
        <button type="button" role="tab" aria-selected={view === 'notas'} className={view === 'notas' ? 'active' : ''} onClick={() => setView('notas')}>Notas</button>
        <button type="button" role="tab" aria-selected={view === 'biblioteca'} className={view === 'biblioteca' ? 'active' : ''} onClick={() => setView('biblioteca')}>Biblioteca</button>
      </div>
      {view === 'biblioteca'
        ? <LibraryPanel library={library} current={activeTab && isBookRoot(activeTab.root) ? { source: sourceOf(activeTab.root), path: activeTab.path } : undefined}
            onOpen={(source, path) => open(bookRoot(source), path)} onReload={loadLibrary} onMark={markBooks}
            onMoved={(from, to) => retarget(bookRoot('ayla'), from, to)}
            onGuard={(mode, path, file) => setLockDialog({ mode, root: 'biblioteca', path, file })} onLockNow={path => void lockItem('biblioteca', path)} />
        : <>
      <div className="athena-actions">
        <button className="ghost-button athena-import" title="Abrir um cofre do Obsidian ou uma pasta de notas" onClick={() => void addVault()}><PlusIcon size={16} />Abrir cofre</button>
        <button className="ghost-button athena-import" title={`Copiar arquivos .txt e .md para ${rootOf(focus)?.name ?? 'Athena'} como notas .md`} onClick={() => setImporting({ root: rootOf(focus)?.available ? focus : 'ayla', folder: '' })}><ImportIcon size={16} />Importar</button>
        <button className={calendar ? 'ghost-button athena-import active' : 'ghost-button athena-import'} aria-pressed={calendar} title="Calendário: notas do dia, eventos e lembretes" onClick={() => setCalendar(v => !v)}><CalendarIcon size={16} />Calendário</button>
      </div>
      <p className="athena-focus">Novas notas em <b>{rootOf(focus)?.name ?? 'Athena'}</b></p>
      {tree?.suggested.map(path => <div key={path} className="athena-suggest"><span title={path}>Cofre do Obsidian: <b>{path.slice(path.lastIndexOf('/') + 1)}</b></span><button className="link-button" onClick={() => void addVault(path)}>Abrir</button></div>)}
      <input className="organ-search" type="search" value={query} placeholder="Buscar em todas as notas" aria-label="Buscar nas notas" onChange={e => { setQuery(e.target.value); if (!e.target.value.trim()) setHits(null); }} />
      <nav className={selectionCount ? 'athena-tree selecting' : 'athena-tree'} role="tree" aria-label="Pastas e notas" aria-multiselectable="true" inert={batchBusy}>
        {query.trim() && hits
          ? (hits.length ? hits.map(noteHit) : <p className="sidebar-label">Nada encontrado.</p>)
          : sections.map(kind => {
            const group = roots.filter(r => r.kind === kind);
            if (!group.length) return null;
            return <div key={kind} className="tree-section">{sectionLabel[kind] && <p className="sidebar-label">{sectionLabel[kind]}</p>}{group.map(rootNode)}</div>;
          })}
      </nav>
      {batchNotice && <p className="athena-focus" role="status">{batchNotice}</p>}
      {selectionCount > 0 && <div className="athena-selection" role="toolbar" aria-label="Itens selecionados" aria-busy={batchBusy}>
        <span>{selectionLabel} {selectionCount === 1 ? 'selecionada' : 'selecionadas'}</span>
        <span>{[...selectedNotes, ...selectedFolders].every(item => item.root === (selectedNotes[0] ?? selectedFolders[0]).root)
          ? 'Arraste a seleção para uma pasta da mesma raiz.' : 'Para mover, selecione itens de uma única raiz.'}</span>
        <div className="athena-actions">
          <Confirm className="ghost-button" disabled={batchBusy} ariaLabel="Apagar os itens selecionados"
            question={`Mover ${selectionLabel} para a lixeira (.trash)?${selectedFolders.length ? ' As pastas vão com tudo o que têm dentro.' : ''}`}
            confirm="Apagar" onConfirm={() => void changeSelected(selectedNotes, undefined, selectedFolders)}>Apagar</Confirm>
          <button className="ghost-button" disabled={batchBusy} aria-label="Limpar a seleção" title="Limpar a seleção" onClick={() => { setSelected([]); selectionAnchor.current = null; }}>×</button>
        </div>
      </div>}
      </>}
    </aside>}
    {sidebar && handle('left', 'Ajustar a largura da barra lateral')}
    <main className="athena-main">
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
      {calendar
        ? <AthenaCalendar onClose={() => setCalendar(false)} onOpenDay={date => void openDay(date)} />
        : <div className="note-panes" inert={batchBusy}>{panes.map(paneView)}</div>}
    </main>
    {/* Na Biblioteca (ou lendo um livro), sem o painel do Git e o grafo das notas. */}
    {!libraryMode && handle('right', 'Ajustar a largura do painel direito')}
    {!libraryMode && <aside className="athena-panel" data-git={workspace.panel} aria-label="Painel direito da Athena" style={{ flexBasis: layout.right, width: layout.right }}>
      {workspace.panel && gitPanel()}
      <AthenaGraph graph={focusedGraph} scope={rootOf(focus)?.name} activeNote={activeTab} onOpen={open} onRefresh={loadOverview} error={graphError} />
    </aside>}
    {importing && <ImportDialog root={importing.root} folder={importing.folder}
      place={importing.folder ? importing.folder.slice(importing.folder.lastIndexOf('/') + 1) : rootOf(importing.root)?.name ?? 'Athena'}
      onDone={() => { setExpanded(prev => [...new Set([...prev, `${importing.root}|`, `${importing.root}|${importing.folder}`])]); load(); loadOverview(); setRevision(r => r + 1); }}
      onClose={() => setImporting(null)} />}
    {lockDialog && <LockDialog mode={lockDialog.mode} folder={lockDialog.path} file={lockDialog.file} git={rootOf(lockDialog.root)?.git}
      recovery={lockOf(lockDialog.root, lockDialog.path)?.recovery} onSubmit={submitLock} onClose={() => { const root = lockDialog.root; setLockDialog(null); reloadFor(root); }} />}
    {trashing && <div className="dialog-backdrop" onClick={() => setTrashing(null)}>
      <div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="athena-trash-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') setTrashing(null); }}>
        <h2 id="athena-trash-title">Apagar “{trashing.title}”?</h2>
        <p>A nota vai para a lixeira (.trash) de {rootOf(trashing.root)?.name ?? 'Athena'} e sai da lista. Dá para recuperá-la de lá depois.</p>
        <div className="row-actions">
          <button className="danger-action" onClick={() => void trashNote(trashing.root, trashing.path)}>Apagar</button>
          <button className="ghost-button" autoFocus onClick={() => setTrashing(null)}>Cancelar</button>
        </div>
      </div>
    </div>}
  </div>;
}
