import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import Confirm from './Confirm';
import AthenaNote, { AttachmentView, type Note, type Tab } from './AthenaNote';
import { ChevronRightIcon, FileIcon, ImageIcon, VideoIcon, CloseIcon, ColumnsIcon, ExpandIcon, EyeIcon, GitIcon, PencilIcon, PinIcon, PlusIcon, RefreshIcon, SidebarIcon, TrashIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import { preloadPdf } from '../services/PdfJs';
import { VIDEO, folderOf, isAttachment, titleOf } from '../services/Wiki';
import BrandMark from './BrandMark';

type Root = { id: string; kind: 'ayla' | 'vault' | 'project'; name: string; path: string; available: boolean; obsidian: boolean; git: boolean; folders: string[]; notes: Note[]; files?: Note[] };
type Tree = { roots: Root[]; suggested: string[]; git_enabled: boolean };
type Hit = { root: string; rootName: string; path: string; title: string; snippet: string };
type Outcome = { committed: number; pulled: boolean; pushed: boolean; message: string };
type Change = { path: string; kind: 'novo' | 'modificado' | 'apagado' | 'renomeado' | 'conflito' };
type GitStatus = { repo: boolean; branch: string; changes: number; files: Change[]; remote: boolean; upstream: boolean; ahead: number; behind: number; last_commit: { message: string; at: number } | null };
type GitInfo = { enabled: boolean; obsidian: boolean; status: GitStatus; last: { at: number; outcome: Outcome } | null };
type Adding = { root: string; folder: string; kind: 'note' | 'folder' };
type Folder = { name: string; path: string; folders: Folder[]; notes: Note[]; files: Note[] };
type TagEntry = { tag: string; notes: { root: string; rootName: string; path: string; title: string }[] };
type Pane = { tabs: Tab[]; active: number };
/** O que está sendo arrastado: nota ou pasta da árvore, ou uma aba. */
type DragItem = { kind: 'note' | 'file' | 'folder'; root: string; path: string; label: string; tab?: { pane: number; at: number } };
/** Onde soltar: numa pasta (move o arquivo) ou numa tela (abre a nota; nas laterais, divide a tela). */
type Drop = { kind: 'folder'; root: string; path: string } | { kind: 'pane'; pane: number; side: 'center' | 'left' | 'right' };
/** Abas abertas (até duas telas lado a lado), a tela em foco e o painel do Git: ficam salvos neste computador. */
type Workspace = { panes: Pane[]; focused: number; panel: boolean };

/** Pastas e notas de uma raiz em árvore, em ordem alfabética (como o Obsidian). */
function build(root: Root): Folder {
  const top: Folder = { name: root.name, path: '', folders: [], notes: [], files: [] };
  const find = (path: string): Folder => {
    if (!path) return top;
    const parent = find(folderOf(path));
    const name = path.slice(path.lastIndexOf('/') + 1);
    let folder = parent.folders.find(f => f.name === name);
    if (!folder) { folder = { name, path, folders: [], notes: [], files: [] }; parent.folders.push(folder); }
    return folder;
  };
  root.folders.forEach(find);
  root.notes.forEach(note => find(folderOf(note.path)).notes.push(note));
  (root.files ?? []).forEach(file => find(folderOf(file.path)).files.push(file));
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
function restore(): Workspace {
  try {
    const saved = JSON.parse(localStorage.getItem(WORKSPACE) ?? 'null') as Workspace | null;
    if (!saved || !Array.isArray(saved.panes) || !saved.panes.length) return empty;
    const panes = saved.panes.slice(0, 2).map(p => {
      const tabs = (Array.isArray(p.tabs) ? p.tabs : []).filter(t => typeof t?.root === 'string' && typeof t.path === 'string').map(t => ({ root: t.root, path: t.path, pinned: !!t.pinned, editing: !isAttachment(t.path) }));
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
export default function Athena({ active, onOpenSettings }: { active: boolean; onOpenSettings: () => void }) {
  const [tree, setTree] = useState<Tree | null>(null); const [expanded, setExpanded] = useState<string[]>(['ayla|']);
  const [query, setQuery] = useState(''); const [hits, setHits] = useState<Hit[] | null>(null);
  const [error, setError] = useState(''); const [adding, setAdding] = useState<Adding | null>(null); const [name, setName] = useState('');
  useEffect(() => { if (active) preloadPdf(); }, [active]);
  /** Nota recém-criada que abre pedindo o título. */
  const [fresh, setFresh] = useState<string | null>(null);
  const [folderEdit, setFolderEdit] = useState<{ root: string; path: string; name: string } | null>(null);
  const [tags, setTags] = useState<TagEntry[]>([]); const [tag, setTag] = useState<string | null>(null); const [showTags, setShowTags] = useState(false);
  const [focus, setFocus] = useState('ayla'); const [sidebar, setSidebar] = useState(true);
  const [workspace, setWorkspace] = useState<Workspace>(restore);
  const [statuses, setStatuses] = useState(['', '']); const [confirming, setConfirming] = useState<number | null>(null);
  /** Nota a apagar pela lixeira da barra lateral, à espera da confirmação no modal. */
  const [trashing, setTrashing] = useState<{ root: string; path: string; title: string } | null>(null);
  const [revision, setRevision] = useState(0); const [saved, setSaved] = useState(0);
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
  const loadTags = useCallback(() => { void native<TagEntry[]>('athena_tags').then(setTags).catch(() => setTags([])); }, []);
  const rootOf = useCallback((id: string) => tree?.roots.find(r => r.id === id) ?? null, [tree]);
  const expand = useCallback((root: string, path: string) => {
    const keys = [`${root}|`];
    const parts = path.split('/').slice(0, -1);
    parts.forEach((_, i) => keys.push(`${root}|${parts.slice(0, i + 1).join('/')}`));
    setExpanded(prev => [...new Set([...prev, ...keys])]);
  }, []);
  useEffect(() => { localStorage.setItem(WORKSPACE, JSON.stringify(workspace)); }, [workspace]);
  useEffect(() => { if (active) { load(); loadTags(); } }, [active, load, loadTags]);

  const panes = workspace.panes;
  const focused = Math.min(workspace.focused, panes.length - 1);
  const activeTab = panes[focused].tabs[panes[focused].active] as Tab | undefined;

  /** Abre a nota na tela em foco: na aba atual (se não estiver fixada) ou numa aba nova. */
  const open = useCallback((root: string, path: string, options: { newTab?: boolean; edit?: boolean } = {}) => {
    setError(''); setFocus(root); expand(root, path);
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
    });
  }, [expand]);
  const updatePane = (index: number, change: (pane: Pane) => Pane) => setWorkspace(ws => {
    const next = ws.panes.map((p, k) => k === index ? change(p) : p);
    if (next.length > 1 && !next[index].tabs.length) { const rest = next.filter((_, k) => k !== index); return { ...ws, panes: rest, focused: 0 }; }
    return { ...ws, panes: next };
  });
  const closeTab = (index: number, at: number) => updatePane(index, pane => {
    const tabs = pane.tabs.filter((_, k) => k !== at);
    return { tabs, active: Math.max(0, Math.min(at < pane.active ? pane.active - 1 : pane.active, tabs.length - 1)) };
  });
  const setTab = (index: number, at: number, change: Partial<Tab>) => updatePane(index, pane => ({ ...pane, tabs: pane.tabs.map((t, k) => k === at ? { ...t, ...change } : t) }));
  const activate = (index: number, at: number) => {
    const tab = panes[index]?.tabs[at];
    if (tab) setFocus(tab.root);
    setWorkspace(ws => ({ ...ws, focused: index, panes: ws.panes.map((p, k) => k === index ? { ...p, active: at } : p) }));
  };
  /** Troca o caminho de notas abertas (renomear ou mover nota ou pasta). */
  const retarget = (root: string, from: string, to: string, folder = false) => setWorkspace(ws => ({ ...ws, panes: ws.panes.map(p => ({ ...p, tabs: p.tabs.map(t => t.root !== root ? t
    : folder && t.path.startsWith(`${from}/`) ? { ...t, path: `${to}${t.path.slice(from.length)}` }
    : !folder && t.path === from ? { ...t, path: to } : t) })) }));
  /** Fecha as abas que não existem mais (nota apagada, cofre fechado). */
  const drop = (gone: (tab: Tab) => boolean) => setWorkspace(ws => {
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

  useEffect(() => {
    const stop = listen<{ root?: string; path?: string }>('notes-updated', event => {
      load(); loadTags(); setRevision(r => r + 1);
      if (event.payload?.root && event.payload.path) open(event.payload.root, event.payload.path, { newTab: true });
    });
    return () => { void stop.then(off => off()); };
  }, [load, loadTags, open]);
  useEffect(() => {
    if (!query.trim()) return;
    const id = setTimeout(() => void native<Hit[]>('athena_search', { query }).then(setHits).catch(e => setError(errorText(e))), 250);
    return () => clearTimeout(id);
  }, [query]);

  // Painel do Git: segue a raiz em foco; atualiza ao abrir, ao trocar de raiz e pouco depois de salvar.
  const gitRoot = rootOf(focus);
  const refreshGit = useCallback(() => {
    if (!workspace.panel || !gitRoot) return;
    void native<GitInfo>('athena_git_status', { root: gitRoot.id }).then(setGit).catch(() => setGit(null));
  }, [workspace.panel, gitRoot]);
  useEffect(() => { setGit(null); }, [focus]);
  useEffect(() => { const id = setTimeout(refreshGit, saved ? 1200 : 0); return () => clearTimeout(id); }, [refreshGit, saved, revision]);
  useEffect(() => { window.addEventListener('focus', refreshGit); return () => window.removeEventListener('focus', refreshGit); }, [refreshGit]);

  const onSaved = useCallback(() => { load(); loadTags(); setSaved(n => n + 1); }, [load, loadTags]);
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
      load(); setFresh(`${root}|${made}`); open(root, made, { edit: true });
    } catch (e) { setError(errorText(e)); }
  }
  async function today() {
    const root = rootOf(focus)?.available ? focus : 'ayla';
    try { const daily = await native<string>('athena_daily', { root }); load(); open(root, daily); } catch (e) { setError(errorText(e)); }
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
      setConfirming(null); setTrashing(null); drop(t => same(t, root, path)); load(); loadTags();
    } catch (e) { setTrashing(null); setError(errorText(e)); }
  }
  async function addVault(path?: string) {
    setError('');
    try {
      const folder = path ?? await native<string | null>('athena_pick_folder');
      if (!folder) return;
      await native('athena_add_vault', { path: folder });
      setExpanded(prev => [...prev, `vault:${folder}|`]); setFocus(`vault:${folder}`); load(); vaultsChanged();
    } catch (e) { setError(errorText(e)); }
  }
  async function closeVault(root: Root) {
    try {
      await native('athena_remove_vault', { path: root.path });
      drop(t => t.root === root.id); if (focus === root.id) setFocus('ayla'); load(); vaultsChanged();
    } catch (e) { setError(errorText(e)); }
  }
  async function sync() {
    if (!gitRoot) return;
    setSyncing(true); setError('');
    try {
      await flushAll();
      const outcome = await native<Outcome>('athena_git_sync', { root: gitRoot.id });
      setStatuses([outcome.message, outcome.message]); load(); loadTags(); setRevision(r => r + 1);
    } catch (e) { setError(errorText(e)); } finally { setSyncing(false); }
  }
  /** Renomeia ou move a nota da aba (o núcleo atualiza os links nas outras notas). */
  async function moveNote(index: number, tab: { root: string; path: string }, to: string) {
    setError('');
    try {
      await flushAll();
      const result = await native<{ path: string; relinked: number }>('athena_rename', { root: tab.root, path: tab.path, to });
      retarget(tab.root, tab.path, result.path); expand(tab.root, result.path); load(); loadTags(); setRevision(r => r + 1);
      const text = result.relinked ? `Links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}` : folderOf(result.path) !== folderOf(tab.path) ? 'Nota movida' : 'Nota renomeada';
      setStatuses(prev => prev.map((s, k) => k === index ? text : s));
    } catch (e) { setError(errorText(e)); }
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
      setFolderEdit(null); setExpanded(prev => [...prev, `${root}|${result.path}`]); retarget(root, path, result.path, true); load(); setRevision(r => r + 1);
      if (result.relinked) setStatuses(prev => prev.map(() => `Pasta renomeada · links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}`));
    } catch (e) { setError(errorText(e)); }
  }
  /** Pasta ou tela sob o ponteiro, se aceitar o item (mesma raiz, não para dentro de si). */
  function findDrop(item: DragItem, x: number, y: number): Drop | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-drop]');
    if (!el) return null;
    if (el.dataset.drop === 'folder') {
      const root = el.dataset.root ?? ''; const path = el.dataset.path ?? '';
      if (root !== item.root || folderOf(item.path) === path || (item.kind === 'note' && isAttachment(item.path))) return null;
      if (item.kind === 'folder' && (path === item.path || path.startsWith(`${item.path}/`))) return null;
      return { kind: 'folder', root, path };
    }
    if (el.dataset.drop === 'pane' && item.kind !== 'folder') {
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
    if (e.button !== 0 || (e.target as HTMLElement).closest('input, form')) return;
    const sx = e.clientX; const sy = e.clientY; let moving = false; let last: Drop | null = null;
    const move = (ev: PointerEvent) => {
      if (!moving && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
      if (!moving) window.getSelection()?.removeAllRanges();
      moving = true; last = findDrop(item, ev.clientX, ev.clientY);
      setDrag({ item, x: ev.clientX, y: ev.clientY, drop: last });
    };
    const finish = (cancel: boolean) => {
      window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); window.removeEventListener('keydown', key);
      if (!moving) return;
      swallow.current = true; setTimeout(() => { swallow.current = false; }, 0);
      setDrag(null);
      if (!cancel && last) void dropItem(item, last);
    };
    const up = () => finish(false);
    const key = (ev: KeyboardEvent) => { if (ev.key === 'Escape') finish(true); };
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up); window.addEventListener('keydown', key);
  };
  const dropping = (root: string, path: string) => drag?.drop?.kind === 'folder' && drag.drop.root === root && drag.drop.path === path;

  /** Clique num link [[…]]: abre a nota da mesma raiz ou, como no Obsidian, cria se ainda não existe. */
  async function follow(tab: Tab, target: string) {
    const root = rootOf(tab.root);
    if (isAttachment(target)) { open(tab.root, target.split('#')[0].trim()); return; }
    const found = root && resolve(root, target);
    if (found) { open(tab.root, found); return; }
    try {
      const made = await native<string>('athena_create', { root: tab.root, folder: folderOf(target), title: target.slice(target.lastIndexOf('/') + 1) });
      load(); open(tab.root, made, { edit: true });
    } catch (e) { setError(errorText(e)); }
  }
  const showTag = (t: string) => { setTag(t); setShowTags(true); setQuery(''); setHits(null); setSidebar(true); };
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
      drop(t => t.root === root && t.path.startsWith(`${folder}/`)); load(); loadTags();
    } catch (e) { setError(errorText(e)); }
  }
  function tools(root: string, folder: string): ReactNode {
    const start = (kind: Adding['kind']) => (e: { stopPropagation: () => void }) => { e.stopPropagation(); setFocus(root); setAdding({ root, folder, kind }); setName(''); setExpanded(prev => [...new Set([...prev, `${root}|${folder}`])]); };
    return <span className="tree-tools">
      <button className="icon-button tiny" title="Nova nota aqui" aria-label="Nova nota aqui" onClick={e => { e.stopPropagation(); setFocus(root); setExpanded(prev => [...new Set([...prev, `${root}|${folder}`])]); void newNote(root, folder); }}><PlusIcon size={14} /></button>
      <button className="icon-button tiny" title="Nova pasta aqui" aria-label="Nova pasta aqui" onClick={start('folder')}>⊞</button>
      {folder && <Confirm className="icon-button tiny" title="Apagar a pasta (vai para a lixeira do cofre)" ariaLabel={`Apagar a pasta ${folder}`} question={`Apagar a pasta "${folder.slice(folder.lastIndexOf('/') + 1)}" e tudo o que tem dentro? Ela vai para a lixeira (.trash) da raiz.`} confirm="Apagar" onConfirm={() => void removeFolder(root, folder)}><TrashIcon size={13} /></Confirm>}
      {folder && <button className="icon-button tiny" title="Renomear ou mover a pasta" aria-label="Renomear ou mover a pasta" onClick={e => { e.stopPropagation(); setFolderEdit({ root, path: folder, name: folder.slice(folder.lastIndexOf('/') + 1) }); }}>✎</button>}
    </span>;
  }
  function folderNode(root: Root, folder: Folder, depth: number): ReactNode {
    const key = `${root.id}|${folder.path}`;
    const shown = !folder.path || expanded.includes(key);
    const inner = folder.path ? depth + 1 : depth;
    return <div key={key} role="group" className={folder.path && dropping(root.id, folder.path) ? 'tree-drop drop-target' : 'tree-drop'} data-drop={folder.path ? 'folder' : undefined} data-root={root.id} data-path={folder.path}>
      {folder.path && (folderEdit?.root === root.id && folderEdit.path === folder.path
        ? <form className="tree-add" style={{ paddingLeft: 4 + depth * 14 }} onSubmit={e => void renameFolder(e)}>
            <input autoFocus value={folderEdit.name} maxLength={200} title="Novo nome, ou caminho (ex.: Jogos/RPG) para mover" onChange={e => setFolderEdit({ ...folderEdit, name: e.target.value })} onKeyDown={e => { if (e.key === 'Escape') setFolderEdit(null); }} />
          </form>
        : <div className="tree-row folder" style={{ paddingLeft: 4 + depth * 14 }} onPointerDown={startDrag({ kind: 'folder', root: root.id, path: folder.path, label: folder.name })} onClick={() => { setFocus(root.id); toggle(key); }} role="treeitem" aria-expanded={shown}>
            <span className={shown ? 'tree-caret open' : 'tree-caret'}><ChevronRightIcon size={13} /></span><span className="tree-name">{folder.name}</span>{tools(root.id, folder.path)}
          </div>)}
      {shown && <>
        {addForm(root.id, folder.path, inner)}
        {folder.folders.map(child => folderNode(root, child, inner))}
        {[...folder.notes.map(n => ({ ...n, file: false })), ...folder.files.map(f => ({ ...f, file: true }))].sort((a, b) => a.title.localeCompare(b.title, 'pt-BR')).map(item => item.file
          ? <div key={item.path} role="treeitem" className="tree-row note file" aria-current={activeTab && same(activeTab, root.id, item.path) ? 'page' : undefined} style={{ paddingLeft: 20 + inner * 14 }} title={`${item.path}\nClique para abrir; arraste para uma pasta para mover`} onPointerDown={startDrag({ kind: 'file', root: root.id, path: item.path, label: item.title })} onClick={openFromTree(root.id, item.path)} onAuxClick={openFromTree(root.id, item.path)}>
              <span className="tree-file-icon">{/\.pdf$/i.test(item.path) ? <FileIcon size={13} /> : VIDEO.test(item.path) ? <VideoIcon size={13} /> : <ImageIcon size={13} />}</span><span className="tree-name">{item.title.replace(/\.[^.]+$/, '')}</span><small>{item.title.slice(item.title.lastIndexOf('.') + 1).toUpperCase()}</small>
            </div>
          : <div key={item.path} role="treeitem" className="tree-row note" aria-current={activeTab && same(activeTab, root.id, item.path) ? 'page' : undefined} style={{ paddingLeft: 20 + inner * 14 }} title={`${item.path}\nCtrl+clique ou botão do meio: abrir em nova aba. Arraste para uma pasta (mover) ou para a tela (abrir ao lado)`} onPointerDown={startDrag({ kind: 'note', root: root.id, path: item.path, label: item.title })} onClick={openFromTree(root.id, item.path)} onAuxClick={openFromTree(root.id, item.path)}>
          <span className="tree-name">{item.title}</span>
          <button className="icon-button tiny tree-trash" title="Apagar nota" aria-label={`Apagar a nota ${item.title}`} onPointerDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); setTrashing({ root: root.id, path: item.path, title: item.title }); }}><TrashIcon size={13} /></button>
        </div>)}
      </>}
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
        {root.kind === 'vault' && <Confirm className="icon-button tiny" title="Fechar cofre (os arquivos ficam onde estão)" ariaLabel={`Fechar o cofre ${root.name}`} question="Fechar este cofre na Athena? Os arquivos ficam onde estão." confirm="Fechar" onConfirm={() => void closeVault(root)}>×</Confirm>}
      </div>
      {shown && root.available && folderNode(root, build(root), 1)}
    </div>;
  }

  function paneView(pane: Pane, index: number) {
    const tab = pane.tabs[pane.active] as Tab | undefined;
    const root = tab ? rootOf(tab.root) : null;
    const last = index === panes.length - 1;
    const zone = drag?.drop?.kind === 'pane' && drag.drop.pane === index ? drag.drop.side : null;
    return <section key={index} data-drop="pane" data-pane={index} className={panes.length > 1 && index === focused ? 'note-pane focused' : 'note-pane'} onMouseDownCapture={() => { if (index !== focused) { setWorkspace(ws => ({ ...ws, focused: index })); if (tab) setFocus(tab.root); } }} aria-label={panes.length > 1 ? (index ? 'Tela da direita' : 'Tela da esquerda') : 'Nota'}>
      {zone && <div className={`drop-zone ${zone}`} aria-hidden="true"><span>{zone === 'center' ? 'Abrir nesta tela' : 'Abrir ao lado'}</span></div>}
      <header className="note-tabs organ-bar">
        {index === 0 && !sidebar && <button className="icon-button" data-tip="Mostrar a barra lateral" aria-label="Mostrar a barra lateral" onClick={() => setSidebar(true)}><SidebarIcon size={18} /></button>}
        <div className="tab-strip" role="tablist" aria-label="Notas abertas">
          {pane.tabs.map((t, k) => <div key={`${t.root}|${t.path}`} role="tab" aria-selected={k === pane.active} className={['note-tab', k === pane.active ? 'active' : '', t.pinned ? 'pinned' : ''].join(' ')} title={`${rootOf(t.root)?.name ?? ''} / ${t.path}`} onPointerDown={startDrag({ kind: isAttachment(t.path) ? 'file' : 'note', root: t.root, path: t.path, label: titleOf(t.path), tab: { pane: index, at: k } })} onClick={() => activate(index, k)} onAuxClick={e => { if (e.button === 1 && !t.pinned) closeTab(index, k); }} onDoubleClick={() => setTab(index, k, { pinned: !t.pinned })}>
            {t.pinned && <span className="tab-pin" aria-label="Fixada"><PinIcon size={12} /></span>}
            <span className="tab-title">{titleOf(t.path)}</span>
            <button className="tab-button" aria-label={t.pinned ? 'Desafixar' : 'Fixar'} data-tip={t.pinned ? 'Desafixar' : 'Fixar'} onClick={e => { e.stopPropagation(); setTab(index, k, { pinned: !t.pinned }); }}><PinIcon size={12} /></button>
            {!t.pinned && <button className="tab-button" aria-label="Fechar aba" data-tip="Fechar" onClick={e => { e.stopPropagation(); closeTab(index, k); }}><CloseIcon size={12} /></button>}
          </div>)}
        </div>
        <span className="athena-status" role="status">{statuses[index]}</span>
        {tab && !isAttachment(tab.path) && <div className="note-actions">
          <div className="note-mode" role="group" aria-label="Modo da nota">
            <button className={tab.editing ? 'icon-button' : 'icon-button active'} aria-pressed={!tab.editing} aria-label="Ver" data-tip="Ver" onClick={() => setTab(index, pane.active, { editing: false })}><EyeIcon size={18} /></button>
            <button className={tab.editing ? 'icon-button active' : 'icon-button'} aria-pressed={tab.editing} aria-label="Editar" data-tip="Editar" onClick={() => setTab(index, pane.active, { editing: true })}><PencilIcon size={17} /></button>
          </div>
          {root?.obsidian && <button className="icon-button" data-tip="Abrir no Obsidian" aria-label="Abrir no Obsidian, com os plugins do cofre" onClick={() => void (flushers.current[index]?.() ?? Promise.resolve()).then(() => native('athena_open_obsidian', { root: tab.root, path: tab.path })).catch(e => setError(errorText(e)))}><ExpandIcon size={17} /></button>}
          {confirming === index
            ? <span className="row-actions"><button onClick={() => void remove(index)}>Mover para a lixeira</button><button className="ghost-button" onClick={() => setConfirming(null)}>Cancelar</button></span>
            : <button className="icon-button" aria-label="Apagar nota" data-tip="Apagar nota" onClick={() => setConfirming(index)}><TrashIcon size={17} /></button>}
        </div>}
        <button className={panes.length > 1 ? 'icon-button active' : 'icon-button'} data-tip={panes.length > 1 ? 'Fechar esta tela' : 'Dividir a tela'} aria-label={panes.length > 1 ? 'Fechar esta tela' : 'Dividir a tela'} onClick={() => panes.length > 1 ? setWorkspace(ws => ({ ...ws, panes: ws.panes.filter((_, k) => k !== index), focused: 0 })) : split()}>{panes.length > 1 ? <CloseIcon size={17} /> : <ColumnsIcon size={17} />}</button>
        {last && <button className={workspace.panel ? 'icon-button tip-end active' : 'icon-button tip-end'} data-tip={workspace.panel ? 'Fechar o painel do Git' : 'Painel do Git'} aria-label={workspace.panel ? 'Fechar o painel do Git' : 'Abrir o painel do Git'} aria-pressed={workspace.panel} onClick={() => setWorkspace(ws => ({ ...ws, panel: !ws.panel }))}><GitIcon size={18} /></button>}
      </header>
      {tab && isAttachment(tab.path)
        ? <AttachmentView key={`${tab.root}|${tab.path}`} root={tab.root} target={tab.path} onError={setError} />
        : tab
        ? <AthenaNote key={`${tab.root}|${tab.path}`} tab={tab} notes={root?.notes ?? []} revision={revision}
            onEditing={editing => setTab(index, pane.active, { editing })}
            onFollow={target => void follow(tab, target)}
            onOpen={path => open(tab.root, path)}
            onAttachment={target => open(tab.root, target, { newTab: true })}
            onRename={name => moveNote(index, tab, `${folderOf(tab.path) ? `${folderOf(tab.path)}/` : ''}${name}`)}
            askTitle={fresh === `${tab.root}|${tab.path}`} onTitled={() => setFresh(null)}
            onSaved={onSaved} onTag={showTag}
            onStatus={text => setStatuses(prev => prev[index] === text ? prev : prev.map((s, k) => k === index ? text : s))}
            onError={setError}
            register={flush => { flushers.current[index] = flush; }} />
        : index === 0
        ? <div className="athena-welcome">
            <h1>Athena</h1>
            <p>Suas notas em Markdown, em pastas, como no Obsidian: as notas da Athena, os seus cofres e uma pasta para cada projeto da Ayla. Links como [[nota]] funcionam entre as notas.</p>
            <p>Clique numa nota para abrir; Ctrl+clique abre em nova aba. Arraste uma nota para uma pasta para movê-la, ou para a lateral da tela para ver duas notas lado a lado. Fixe uma aba para ela continuar aberta.</p>
            <div className="row-actions"><button className="primary-action" onClick={() => void addVault()}>Abrir cofre</button><button onClick={() => void today()}>Nota de hoje</button><button className="ghost-button" onClick={onOpenSettings}>Configurações</button></div>
          </div>
        : <div className="athena-welcome"><p>Escolha uma nota na barra lateral para abrir nesta tela.</p></div>}
    </section>;
  }

  function gitPanel() {
    const status = git?.status;
    return <aside className="athena-panel" aria-label="Painel do Git" style={{ flexBasis: layout.right, width: layout.right }}>
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
    </aside>;
  }

  const sections: Root['kind'][] = ['ayla', 'vault', 'project'];
  return <div className={['chat-layout athena', drag ? 'dragging' : '', resizing ? 'resizing' : ''].join(' ')} onClickCapture={e => { if (swallow.current) { e.stopPropagation(); e.preventDefault(); } }}>
    {drag && <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>{drag.item.kind === 'folder' ? '📁 ' : ''}{drag.item.label}{drag.drop?.kind === 'folder' && <small> → {drag.drop.path ? drag.drop.path.slice(drag.drop.path.lastIndexOf('/') + 1) : rootOf(drag.drop.root)?.name}</small>}</div>}
    {sidebar && <aside className="sidebar organ-sidebar athena-sidebar" aria-label="Notas da Athena" style={{ flexBasis: layout.left, width: layout.left }}>
      <div className="brand"><BrandMark name="athena" size={26} /><span className="brand-name">Athena</span><button className="icon-button organ-hide" data-tip="Esconder a barra lateral" aria-label="Esconder a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon size={18} /></button></div>
      <div className="athena-actions">
        <button className="new-chat" onClick={() => void addVault()}><PlusIcon size={18} />Abrir cofre</button>
        <button className="ghost-button" title={`Nota de hoje em ${rootOf(focus)?.name ?? 'Athena'}`} onClick={() => void today()}>Hoje</button>
      </div>
      <p className="athena-focus">Novas notas em <b>{rootOf(focus)?.name ?? 'Athena'}</b></p>
      {tree?.suggested.map(path => <div key={path} className="athena-suggest"><span title={path}>Cofre do Obsidian: <b>{path.slice(path.lastIndexOf('/') + 1)}</b></span><button className="link-button" onClick={() => void addVault(path)}>Abrir</button></div>)}
      {tags.length > 0 && <div className="athena-tags">
        <button className="link-button" aria-expanded={showTags} onClick={() => setShowTags(v => !v)}>{showTags ? '▾' : '▸'} Tags ({tags.length})</button>
        {showTags && <div className="tag-list">{tags.map(t => <button key={t.tag} className={tag === t.tag ? 'tag-chip active' : 'tag-chip'} onClick={() => { setTag(tag === t.tag ? null : t.tag); setQuery(''); setHits(null); }}>#{t.tag} <small>{t.notes.length}</small></button>)}</div>}
      </div>}
      <input className="organ-search" type="search" value={query} placeholder="Buscar em todas as notas" aria-label="Buscar nas notas" onChange={e => { setQuery(e.target.value); if (!e.target.value.trim()) setHits(null); }} />
      <nav className="athena-tree" role="tree" aria-label="Pastas e notas">
        {tag && !query.trim()
          ? <>
              <div className="tree-filter"><span>#{tag}</span><button className="link-button" onClick={() => setTag(null)}>Limpar</button></div>
              {(tags.find(t => t.tag === tag)?.notes ?? []).map(n => <div key={`${n.root}|${n.path}`} className="tree-row note hit" onPointerDown={startDrag({ kind: 'note', root: n.root, path: n.path, label: n.title })} onClick={openFromTree(n.root, n.path)} onAuxClick={openFromTree(n.root, n.path)} title={n.path}><span className="tree-name">{n.title}</span><small>{n.rootName}</small></div>)}
            </>
          : query.trim() && hits
          ? (hits.length ? hits.map(hit => <div key={`${hit.root}|${hit.path}`} className="tree-row note hit" onPointerDown={startDrag({ kind: 'note', root: hit.root, path: hit.path, label: hit.title })} onClick={openFromTree(hit.root, hit.path)} onAuxClick={openFromTree(hit.root, hit.path)} title={hit.path}>
              <span className="tree-name">{hit.title}</span><small>{hit.rootName}{hit.snippet ? ` · ${hit.snippet}` : ''}</small>
            </div>) : <p className="sidebar-label">Nada encontrado.</p>)
          : sections.map(kind => {
            const group = roots.filter(r => r.kind === kind);
            if (!group.length) return null;
            return <div key={kind} className="tree-section">{sectionLabel[kind] && <p className="sidebar-label">{sectionLabel[kind]}</p>}{group.map(rootNode)}</div>;
          })}
      </nav>
    </aside>}
    {sidebar && handle('left', 'Ajustar a largura da barra lateral')}
    <main className="athena-main">
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
      <div className="note-panes">{panes.map(paneView)}</div>
    </main>
    {workspace.panel && handle('right', 'Ajustar a largura do painel do Git')}
    {workspace.panel && gitPanel()}
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
