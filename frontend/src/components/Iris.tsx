import type { ProjectSource } from './ProjectSources';
import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from 'react';
import { emit, listen } from '@tauri-apps/api/event';
import MediaView from './MediaView';
import { preloadPdf } from '../services/PdfJs';
import { ChevronLeftIcon, ChevronRightIcon, CloseIcon, CursorTextIcon, ExpandIcon, FileIcon, PencilIcon, PlusIcon, RefreshIcon, SidebarIcon, TrashIcon, VideoIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import { mediaUrl, openInSystem, type MediaRef } from '../services/Media';
import { setMediaVersions } from '../services/Wiki';
import BrandMark from './BrandMark';
import IrisUpscale, { type UpscaleResult } from './IrisUpscale';
import IrisEditor, { type EditResult } from './IrisEditor';
import IrisDesigns from './IrisDesigns';
import IrisFicha from './IrisFicha';
import IrisBatch, { type PickedFolder } from './IrisBatch';
import { OPEN_DESIGN_EVENT, hasPendingDesign } from '../services/Design';

type Kind = 'image' | 'video' | 'pdf';
type Source = 'athena' | 'chat' | 'gerada' | 'arquivos' | 'computer';
type LibraryRoot = { id: string; path: string; name: string; folders: string[]; available: boolean; hasMedia: boolean };
type FolderDrop = { target: string; after: boolean };
type FolderSelection = { root: string; path: string };
type Item = {
  key: string; kind: Kind; name: string; source: Source; place: string; date: number; added?: number | null; bytes: number;
  /** Arquivo guardado na Ayla (anexo do chat, imagem gerada ou adicionado direto). */
  file?: string;
  /** Raiz de navegação. Nos arquivos de conversa, aponta para o projeto da conversa. */
  root?: string | null; path?: string;
};

const kinds: { id: Kind; label: string; empty: string }[] = [
  { id: 'image', label: 'Imagens', empty: 'Nenhuma imagem ainda. Fotos dos cofres da Athena, anexos do chat e imagens criadas pela Ayla aparecem aqui.' },
  { id: 'video', label: 'Vídeos', empty: 'Nenhum vídeo ainda. Vídeos dos cofres da Athena e anexos do chat aparecem aqui.' },
  { id: 'pdf', label: 'PDFs', empty: 'Nenhum PDF ainda. PDFs dos cofres da Athena, anexos do chat e arquivos adicionados aparecem aqui.' },
];
const sources: { id: Source | 'all'; label: string }[] = [
  { id: 'all', label: 'Tudo' }, { id: 'athena', label: 'Athena' }, { id: 'chat', label: 'Chat' }, { id: 'gerada', label: 'Geradas' }, { id: 'arquivos', label: 'Arquivos' },
];
const sourceLabel: Record<Source, string> = { computer: 'Sistema', athena: 'Athena', chat: 'Anexo do chat', gerada: 'Gerada pela Ayla', arquivos: 'Arquivos' };
function itemLocation(item: Item) {
  const label = sourceLabel[item.source];
  return item.place === label || item.place.startsWith(label + ' / ') ? item.place : `${label} · ${item.place}`;
}
const orders = {
  'modified-desc': 'Alteração · recentes primeiro', 'modified-asc': 'Alteração · antigas primeiro',
  'added-desc': 'Adição · recentes primeiro', 'added-asc': 'Adição · antigas primeiro',
  'name-asc': 'Nome · A a Z', 'name-desc': 'Nome · Z a A',
  'size-desc': 'Tamanho · maiores primeiro', 'size-asc': 'Tamanho · menores primeiro',
};
type Order = keyof typeof orders;
const names = new Intl.Collator('pt-BR', { numeric: true, sensitivity: 'base' });
function compareItems(a: Item, b: Item, order: Order) {
  const [field, direction] = order.split('-');
  if (field === 'added') {
    const aMissing = a.added == null;
    const bMissing = b.added == null;
    if (aMissing !== bMissing) return aMissing ? 1 : -1;
  }
  const byName = names.compare(a.name, b.name);
  const delta = field === 'name' ? byName : field === 'size' ? a.bytes - b.bytes
    : field === 'added' ? (a.added ?? 0) - (b.added ?? 0) : a.date - b.date;
  return (direction === 'asc' ? delta : -delta) || byName || a.key.localeCompare(b.key);
}
const TAB = 'ayla-iris-tipo';
const SIDE = 'ayla-iris-lateral';
const PAGE = 60;

/** O item como mídia do serviço comum (o mesmo da Athena). */
const mediaOf = (item: Item): MediaRef => item.file ? { file: item.file } : { root: item.root ?? '', path: item.path ?? '' };
const url = (item: Item, thumb = false) => mediaUrl(mediaOf(item), thumb);
const month = (seconds: number) => { const text = new Date(seconds * 1000).toLocaleDateString('pt-BR', { month: 'long', year: 'numeric' }); return text.charAt(0).toUpperCase() + text.slice(1); };
const size = (bytes: number) => bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(1)} GB` : bytes >= 1024 ** 2 ? `${(bytes / 1024 ** 2).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`;
const day = (seconds: number) => new Date(seconds * 1000).toLocaleDateString('pt-BR', { day: '2-digit', month: 'short', year: 'numeric' });

/** Ampliar: quadrado com setas para fora. */
const UpscaleIcon = () => <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1.5" /><path d="M3 9V3h6M21 9V3h-6M3 15v6h6M21 15v6h-6" /></svg>;

/** Ficha de RPG: folha com linhas e um dado. */
const FichaIcon = () => <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h4" /><path d="M14 3v5h5M9 9h2M9 13h4" /><rect x="14" y="13" width="7" height="7" rx="1.5" /><path d="M16.5 15.5h.01M18.5 17.5h.01" /></svg>;

/** Aguarda o cartão ficar visível antes de pedir uma miniatura. */
function Thumb({ item }: { item: Item }) {
  const [requested, setRequested] = useState(false);
  const [failed, setFailed] = useState(false);
  const placeholder = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (requested || !placeholder.current) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const observer = new IntersectionObserver(([entry]) => {
      clearTimeout(timer);
      if (entry.isIntersecting) timer = setTimeout(() => setRequested(true), 150);
    });
    observer.observe(placeholder.current);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
    };
  }, [requested]);
  if (!requested || failed) return <span ref={placeholder} className="iris-placeholder">
    {item.kind === 'video' ? <VideoIcon size={30} /> : <FileIcon size={30} />}
    <b>{item.kind === 'pdf' ? 'PDF' : item.name.split('.').pop()?.toUpperCase()}</b>
  </span>;
  return <img src={url(item, true) + '&v=' + item.date + '-' + item.bytes} alt="" decoding="async" onError={() => setFailed(true)} />;
}

/** Seleção para as ações em lote: no modo Selecionar (ou com Ctrl), clicar numa pasta marca a pasta. */
type FolderPick = { picking: boolean; picked: (root: string, path: string) => boolean; pick: (root: string, path: string, name: string) => void };

function LibraryFolder({ root, selected, selectFolder, actions, drop, pick }: {
  root: LibraryRoot; selected: FolderSelection | null; selectFolder: (selection: FolderSelection) => void;
  actions?: React.ReactNode; drop?: FolderDrop; pick: FolderPick;
}) {
  const choose = (event: React.MouseEvent, path: string, name: string) => {
    if (pick.picking || event.ctrlKey || event.metaKey) pick.pick(root.id, path, name);
    else selectFolder({ root: root.id, path });
  };
  const pressed = (path: string) => pick.picking ? pick.picked(root.id, path) : selected?.root === root.id && selected.path === path;
  const [expanded, setExpanded] = useState(false);
  const directories = useMemo(() => {
    const byParent = new Map<string, string[]>();
    for (const path of root.folders) {
      const parent = path.slice(0, Math.max(0, path.lastIndexOf('/')));
      if (!byParent.has(parent)) byParent.set(parent, []);
      byParent.get(parent)!.push(path);
    }
    for (const siblings of byParent.values()) {
      siblings.sort((a, b) => names.compare(a.split('/').pop()!, b.split('/').pop()!));
    }
    return byParent;
  }, [root.folders]);
  function directoryTree(parent: string): React.ReactNode {
    return directories.get(parent)?.map(path => {
      const name = path.split('/').pop();
      const button = <button className={pick.picking ? 'iris-dir picking' : 'iris-dir'} title={path} aria-pressed={pressed(path)} onClick={event => choose(event, path, name ?? path)}>{name}</button>;
      return directories.has(path) ? <div key={path} className="iris-dir-group">
        <details><summary aria-label={`Subpastas de ${name}`}>▸</summary><div className="iris-dir-children">{directoryTree(path)}</div></details>{button}
      </div> : <div key={path}>{button}</div>;
    });
  }

  return <nav aria-label={root.path}>
    <div className="iris-action-row iris-root-row" data-root-id={actions ? root.id : undefined} data-drop={drop && (drop.after ? 'after' : 'before')}>
      {root.available && directories.has('') && <button type="button" className="iris-caret" aria-expanded={expanded}
        aria-label={`Subpastas de ${root.name}`} onClick={() => setExpanded(value => !value)}>▸</button>}
      <button className={pick.picking ? 'iris-dir picking' : 'iris-dir'} title={root.path} aria-pressed={pressed('')} onClick={event => choose(event, '', root.name)}>{root.name}</button>
      {actions && <div className="iris-row-actions">{actions}</div>}
    </div>
    {root.available ? <div className="iris-dir-children" hidden={!expanded}>{directoryTree('')}</div> : <p role="status">Pasta indisponível</p>}
  </nav>;
}

/** Iris: as imagens, vídeos e PDFs da Ayla num lugar só — dos cofres e notas da Athena, os
 * anexos enviados no chat, as imagens que a Ayla gerou e os arquivos adicionados direto. */
export default function Iris({ active, external, projectTarget }: { projectTarget?: { item: ProjectSource; id: number }; active: boolean; external?: { id: number; paths: string[] } }) {
  const [library, setLibrary] = useState<{ items: Item[]; roots: LibraryRoot[]; athenaRoots: LibraryRoot[] } | null>(null);
  const items = library?.items ?? null;
  const [tileSize, setTileSize] = useState(() => {
    const saved = Number(localStorage.getItem('ayla-iris-zoom'));
    return saved >= 90 && saved <= 360 ? saved : 170;
  });
  const [order, setOrder] = useState<Order>(() => {
    const saved = localStorage.getItem('ayla-iris-ordem');
    return saved && Object.hasOwn(orders, saved) ? saved as Order : 'modified-desc';
  });
  const [selection, setSelection] = useState<FolderSelection | null>(null);
  const dragging = useRef<string | null>(null);
  const [drop, setDrop] = useState<FolderDrop>();
  const [changing, setChanging] = useState(false);
  const request = useRef(0); const [error, setError] = useState('');
  const [kind, setKind] = useState<Kind>(() => kinds.find(k => k.id === localStorage.getItem(TAB))?.id ?? 'image');
  // Designs (estilo Canva) ficam numa aba própria, ao lado dos tipos de arquivo.
  const [designing, setDesigning] = useState(() => hasPendingDesign() || localStorage.getItem(TAB) === 'design');
  useEffect(() => {
    const open = () => setDesigning(true);
    window.addEventListener(OPEN_DESIGN_EVENT, open);
    return () => window.removeEventListener(OPEN_DESIGN_EVENT, open);
  }, []);
  const [designCount, setDesignCount] = useState<number | null>(null);
  const [source, setSource] = useState<Source | 'all'>(() => localStorage.getItem(TAB) === 'folder' ? 'computer' : 'all'); const [query, setQuery] = useState('');
  const [open, setOpen] = useState<string | null>(null); const [shown, setShown] = useState(PAGE);
  const [confirmTrash, setConfirmTrash] = useState(false);
  const [trashing, setTrashing] = useState(false);
  const [trashError, setTrashError] = useState('');
  const [loading, setLoading] = useState(false);
  const [sidebar, setSidebar] = useState(() => localStorage.getItem(SIDE) !== '0');
  // Seleção para as ações em lote (upscale, mover, lixeira): cartões pela chave, pastas por raiz|caminho.
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [pickedFolders, setPickedFolders] = useState<Map<string, PickedFolder>>(() => new Map());
  const anchor = useRef<string | null>(null);
  const more = useRef<HTMLDivElement | null>(null);

  const grid = useRef<HTMLElement>(null);
  useEffect(() => {
    const area = grid.current;
    if (!area || !active) return;
    function zoom(event: WheelEvent) {
      if (!event.ctrlKey || !event.deltaY) return;
      event.preventDefault();
      setTileSize(size => Math.max(90, Math.min(360, size - Math.sign(event.deltaY) * 10)));
    }
    area.addEventListener('wheel', zoom, { passive: false });
    return () => area.removeEventListener('wheel', zoom);
  }, [active]);
  useEffect(() => { localStorage.setItem('ayla-iris-zoom', String(tileSize)); }, [tileSize]);

  const load = useCallback(() => {
    const id = ++request.current;
    setLoading(true);
    return native<NonNullable<typeof library>>('iris_library').then(result => {
      if (request.current !== id) return;
      setSelection(selected => {
        if (!selected) return null;
        const root = result.roots.find(r => r.id === selected.root) ?? result.athenaRoots.find(r => r.id === selected.root);
        if (!root?.hasMedia) return null;
        return selected.path && !root.folders.includes(selected.path) ? { root: root.id, path: '' } : selected;
      });
      for (const item of result.items) if (item.root && item.path) setMediaVersions(item.root, [{ path: item.path, modified: item.date }]);
      setLibrary(result); setError('');
    }, e => { if (request.current === id) setError(errorText(e)); })
      .finally(() => { if (request.current === id) setLoading(false); });
  }, []);
  useEffect(() => {
    if (active) void load();
    return () => { ++request.current; };
  }, [active, load]);

  async function changeFolder(disconnect: boolean, root?: string) {
    setChanging(true);
    try {
      await native('iris_change_folder', { disconnect, root });
      await load();
    } catch (e) { setError(errorText(e)); }
    finally { setChanging(false); }
  }
  async function moveFolder(root: string, placement: FolderDrop) {
    if (root === placement.target) return;
    setChanging(true);
    try {
      await native('iris_move_folder', { root, ...placement });
      await load();
    } catch (e) { setError(errorText(e)); }
    finally { setChanging(false); }
  }

  function dropAt(event: PointerEvent<HTMLButtonElement>): FolderDrop | undefined {
    const row = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('.iris-root-row');
    const target = row?.dataset.rootId;
    if (!row || !target || target === dragging.current) return;
    const bounds = row.getBoundingClientRect();
    return { target, after: event.clientY >= bounds.top + bounds.height / 2 };
  }

  useEffect(() => { localStorage.setItem(TAB, designing ? 'design' : kind); }, [kind, designing]);
  useEffect(() => { if (active) preloadPdf(); }, [active]);
  useEffect(() => { localStorage.setItem(SIDE, sidebar ? '1' : '0'); }, [sidebar]);
  useEffect(() => {
    const stop = listen('notes-updated', () => { if (active) load(); });
    return () => { void stop.then(off => off()); };
  }, [active, load]);

  const scopedItems = useMemo(() => (items ?? []).filter(item => selection
    ? item.root === selection.root && (!selection.path || item.path?.startsWith(selection.path + '/'))
    : source === 'all' || item.source === source
  ), [items, source, selection]);
  const counts = useMemo(() => {
    const out: Record<Kind, number> = { image: 0, video: 0, pdf: 0 };
    for (const item of scopedItems) out[item.kind]++;
    return out;
  }, [scopedItems]);
  const visible = useMemo(() => {
    const words = query.trim().toLowerCase();
    return scopedItems.filter(item => item.kind === kind
      && (!words || `${item.name} ${item.place}`.toLowerCase().includes(words))).sort((a, b) => compareItems(a, b, order));
  }, [scopedItems, kind, query, order]);
  const pickedItems = useMemo(() => (items ?? []).filter(item => picked.has(item.key)), [items, picked]);
  const clearPicks = useCallback(() => {
    setPicking(false); setPicked(new Set()); setPickedFolders(new Map()); anchor.current = null;
  }, []);
  function clickCard(item: Item, event: React.MouseEvent) {
    const start = event.shiftKey ? anchor.current : null;
    if (!picking && !event.ctrlKey && !event.metaKey && !event.shiftKey) { setOpen(item.key); return; }
    setPicking(true);
    // O intervalo é calculado agora: a função de atualização roda depois, com a âncora já trocada.
    const from = start === null ? -1 : visible.findIndex(i => i.key === start);
    const to = visible.findIndex(i => i.key === item.key);
    setPicked(previous => {
      const next = new Set(previous);
      if (from >= 0 && to >= 0) for (const i of visible.slice(Math.min(from, to), Math.max(from, to) + 1)) next.add(i.key);
      else if (next.has(item.key)) next.delete(item.key);
      else next.add(item.key);
      return next;
    });
    anchor.current = item.key;
  }
  const folderPick: FolderPick = {
    picking,
    picked: (root, path) => pickedFolders.has(`${root}|${path}`),
    pick: (root, path, name) => {
      setPicking(true);
      setPickedFolders(previous => {
        const next = new Map(previous);
        const key = `${root}|${path}`;
        if (next.has(key)) next.delete(key);
        else next.set(key, { root, path, name, athena: !root.startsWith('iris:') });
        return next;
      });
    },
  };
  // Esc sai da seleção; Ctrl+A pega tudo o que está na tela (com o filtro atual).
  useEffect(() => {
    if (!active || !picking || open || designing) return;
    const key = (event: KeyboardEvent) => {
      if (document.querySelector('.dialog-backdrop')) return;
      const typing = event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]');
      if (event.key === 'Escape') clearPicks();
      else if (!typing && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
        event.preventDefault();
        setPicked(previous => new Set([...previous, ...visible.map(i => i.key)]));
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [active, picking, open, designing, visible, clearPicks]);
  const perSource = useMemo(() => {
    const out: Record<string, number> = { all: 0 };
    for (const item of items ?? []) if (item.kind === kind) { out.all++; out[item.source] = (out[item.source] ?? 0) + 1; }
    return out;
  }, [items, kind]);
  // Mais itens ao chegar perto do fim da grade (não carrega tudo de uma vez).
  useEffect(() => {
    const sentinel = more.current;
    if (!sentinel) return;
    const observer = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) setShown(n => n + PAGE); }, { rootMargin: '160px' });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [visible.length, shown]);
  const groups = useMemo(() => {
    const out: { label: string; items: Item[] }[] = [];
    for (const item of visible.slice(0, shown)) {
      const label = order.startsWith('modified') ? month(item.date) : order.startsWith('added') ? (item.added == null ? 'Sem data de adição' : month(item.added)) : '';
      if (out[out.length - 1]?.label !== label) out.push({ label, items: [] });
      out[out.length - 1].items.push(item);
    }
    return out;
  }, [visible, shown, order]);

  // "Abrir com Iris": a mídia e as vizinhas da mesma pasta, só para ver (nada é conectado).
  const [outside, setOutside] = useState<Item[] | null>(null);
  const handledProject = useRef(0);
  useEffect(() => {
    if (!projectTarget || !library || handledProject.current === projectTarget.id) return;
    handledProject.current = projectTarget.id;
    const { item } = projectTarget;
    setDesigning(false); setKind('image'); setQuery(''); setOutside(null); setOpen(null);
    if (item.kind === 'image') {
      const image = library.items.find(i => i.key === item.path);
      if (!image) { setError('Esta imagem está indisponível.'); return; }
      setSource('all'); setSelection(null); setOpen(image.key);
    } else {
      const system = library.roots.find(r => r.id === item.root);
      const root = system ?? library.athenaRoots.find(r => r.id === item.root);
      if (!root?.available || (item.path && !root.folders.includes(item.path))) { setError('Esta pasta está indisponível.'); return; }
      setSelection({root:item.root,path:item.path}); setSource(system ? 'computer' : 'athena'); setShown(PAGE);
    }
  }, [projectTarget, library]);
  const handled = useRef(0);
  useEffect(() => {
    if (!external || external.id === handled.current || !external.paths.length) return;
    handled.current = external.id;
    void native<{ items: Item[]; open: string }>('iris_open_external', { path: external.paths[0] }).then(result => {
      setOutside([...result.items].sort((a, b) => names.compare(a.name, b.name)));
      setConfirmTrash(false); setTrashError(''); setOpen(result.open);
    }, e => setError(errorText(e)));
  }, [external]);
  const list = outside ?? visible;
  const index = open ? list.findIndex(item => item.key === open) : -1;
  const current = index >= 0 ? list[index] : null;
  // Ampliar a imagem aberta; ao terminar, a cópia aparece na lista e abre no visualizador.
  const [upscaling, setUpscaling] = useState(false);
  const [upscaled, setUpscaled] = useState('');
  async function afterUpscale(result: UpscaleResult) {
    setUpscaling(false);
    setUpscaled(`Salva como ${result.name}.`);
    await load();
    if (!outside) setOpen(result.file ? `arquivo:${result.file}` : `${result.root}|${result.path}`);
  }
  // Editor: a cópia abre no visualizador; ao substituir, fica no mesmo item (já atualizado).
  const [editing, setEditing] = useState(false);
  // Ficha de RPG (PDF de um cofre da Athena): valores na nota e camada no PDF.
  const [sheet, setSheet] = useState(false);
  // Renomear pela barra do visualizador (a extensão fica; nas pastas da Athena, os links das notas acompanham).
  const [renaming, setRenaming] = useState<string | null>(null);
  const renameBusy = useRef(false);
  async function renameCurrent(item: Item, name: string) {
    if (renameBusy.current) return;
    renameBusy.current = true;
    setRenaming(null);
    try {
      const current = item.name.replace(/\.[^.]+$/, '');
      if (!name.trim() || name.trim() === current) return;
      const result = await native<{ path?: string; root?: string; file?: string; name?: string; relinked?: number }>('iris_rename', { ...mediaOf(item), name: name.trim() });
      await load();
      setOpen(result.file ? item.key : `${result.root}|${result.path}`);
      setUpscaled(result.relinked ? `Renomeada · links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}.` : 'Renomeada.');
    } catch (e) { setUpscaled(`Não deu para renomear: ${errorText(e)}`); } finally { renameBusy.current = false; }
  }
  async function afterEdit(result: EditResult) {
    setEditing(false);
    setUpscaled(result.replaced ? `Original substituído: ${result.name}.` : `Salva como ${result.name}.`);
    await load();
    // A Athena (aqui ou na janela separada) relê os anexos e mostra a imagem nova nas notas.
    if (result.replaced && result.root) void emit('notes-updated', { root: result.root });
    if (!outside && !result.replaced) setOpen(result.file ? `arquivo:${result.file}` : `${result.root}|${result.path}`);
  }
  function closeViewer() {
    setEditing(false); setSheet(false);
    setUpscaling(false); setUpscaled('');
    setConfirmTrash(false);
    setTrashError('');
    setOpen(null);
    setOutside(null);
  }

  async function trashCurrent(item: Item) {
    setTrashing(true);
    setTrashError('');
    try {
      await native('iris_trash_image', mediaOf(item));
      closeViewer();
      await load();
    } catch (e) {
      setTrashError(errorText(e));
    } finally {
      setTrashing(false);
    }
  }
  const step = useCallback((delta: number) => {
    if (index < 0 || !list.length) return;
    setOpen(list[(index + delta + list.length) % list.length].key);
  }, [index, list]);
  useEffect(() => {
    if (!current) return;
    const key = (event: KeyboardEvent) => {
      // Digitando o nome novo: as setas andam no texto, não entre as imagens.
      if (trashing || editing || sheet || renaming !== null) return;
      if (confirmTrash) {
        if (event.key === 'Escape') { setConfirmTrash(false); setTrashError(''); }
        return;
      }
      if (event.key === 'Escape') closeViewer();
      else if (event.key === 'ArrowRight' && current.kind !== 'pdf') step(1);
      else if (event.key === 'ArrowLeft' && current.kind !== 'pdf') step(-1);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [current, step, confirmTrash, trashing, editing, sheet, renaming]);

  function system(item: Item) {
    void openInSystem(mediaOf(item)).catch(e => setError(errorText(e)));
  }
  const choose = (next: Kind) => { setKind(next); setDesigning(false); setShown(PAGE); };

  function selectFolder(next: 'athena' | 'computer', folder: FolderSelection) {
    setSelection(folder);
    setSource(next);
    setShown(PAGE);
  }
  const systemRoots = library?.roots.filter(root => root.hasMedia) ?? [];
  const emptySystemRoots = library?.roots.filter(root => !root.hasMedia) ?? [];
  const systemFolderActions = (root: LibraryRoot) => <>
    <button type="button" className="icon-button tip-end" data-tip="Trocar pasta" aria-label="Trocar pasta" disabled={changing} onClick={() => changeFolder(false, root.id)}><RefreshIcon size={15} /></button>
    <button type="button" className="icon-button tip-end" data-tip="Desconectar" aria-label="Desconectar" disabled={changing} onClick={() => changeFolder(true, root.id)}><CloseIcon size={15} /></button>
  </>;
  // Mesmo esqueleto da Athena: barra lateral (marca, busca, origem) e, à direita, a barra
  // superior com os tipos de arquivo no lugar das abas.
  return <div className="chat-layout iris">
    {sidebar && <aside className="sidebar organ-sidebar iris-sidebar" aria-label="Filtros da Iris">
      <div className="brand"><BrandMark name="iris" size={22} /><span className="brand-name">Iris</span><button className="icon-button organ-hide" data-tip="Esconder a barra lateral" aria-label="Esconder a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon size={18} /></button></div>
      <input className="organ-search" type="search" value={query} placeholder="Buscar pelo nome ou lugar" aria-label="Buscar arquivos" onChange={e => { setQuery(e.target.value); setShown(PAGE); }} />
      <div className="iris-sources" role="group" aria-label="Origem">
        <p className="sidebar-label">Origem</p>
        {sources.map(s => {
          const hasAthenaRoots = s.id === 'athena' && !!library?.athenaRoots.length;
          if (s.id !== 'all' && s.id !== source && !perSource[s.id] && !hasAthenaRoots) return null;
          const button = <button key={s.id} className={source === s.id ? 'iris-source active' : 'iris-source'} aria-pressed={source === s.id} onClick={() => { setSource(s.id); setSelection(null); setShown(PAGE); }}><span>{s.label}</span><small>{perSource[s.id] ?? 0}</small></button>;
          if (s.id !== 'athena') return button;
          return <section key={s.id} className="iris-folder" aria-label={s.label}>
            {button}
            {active && library?.athenaRoots.map(root => <LibraryFolder key={root.id} root={root} selected={selection}
              selectFolder={folder => selectFolder('athena', folder)} pick={folderPick} />)}
          </section>;
        })}
        <section className="iris-folder" aria-label="Sistema">
          <div className="iris-action-row iris-system-row">
            <button className={source === 'computer' ? 'iris-source active' : 'iris-source'} aria-pressed={source === 'computer'}
              aria-describedby="iris-system-count" onClick={() => { setSource('computer'); setSelection(null); setShown(PAGE); }}>Sistema</button>
            <div className="iris-row-actions">
              <button type="button" className="icon-button tip-end" data-tip="Adicionar pasta" aria-label="Adicionar pasta" disabled={changing || !library}
                onClick={() => void changeFolder(false)}><PlusIcon size={17} /></button>
            </div>
            <small id="iris-system-count">{perSource.computer ?? 0}</small>
          </div>
          {active && systemRoots.map((root, index, roots) => <LibraryFolder key={root.id} root={root} selected={selection}
            selectFolder={folder => selectFolder('computer', folder)} drop={drop?.target === root.id ? drop : undefined} pick={folderPick}
            actions={<>
              {roots.length > 1 && <button type="button" className="icon-button iris-grip tip-end"
                aria-label={`Reordenar ${root.name}`} data-tip="Arraste ou use as setas ↑ ↓" disabled={changing}
                onKeyDown={event => {
                  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
                  event.preventDefault();
                  const after = event.key === 'ArrowDown';
                  const neighbor = roots[index + (after ? 1 : -1)];
                  if (neighbor) void moveFolder(root.id, { target: neighbor.id, after });
                }}
                onPointerDown={event => {
                  if (event.button !== 0) return;
                  dragging.current = root.id;
                  event.currentTarget.setPointerCapture(event.pointerId);
                }}
                onPointerMove={event => { if (dragging.current === root.id) setDrop(dropAt(event)); }}
                onPointerUp={event => {
                  if (dragging.current !== root.id) return;
                  const placement = dropAt(event);
                  dragging.current = null;
                  setDrop(undefined);
                  if (placement) void moveFolder(root.id, placement);
                }}
                onLostPointerCapture={() => { dragging.current = null; setDrop(undefined); }}><span aria-hidden="true">⠿</span></button>}
              {systemFolderActions(root)}
            </>} />)}
          {active && emptySystemRoots.length > 0 && <details className="iris-empty-roots">
            <summary>Gerenciar outras pastas</summary>
            {emptySystemRoots.map(root => <div key={root.id} className="iris-action-row">
              <span className="iris-dir" title={root.path}>{root.name}</span>
              <div className="iris-row-actions">{systemFolderActions(root)}</div>
            </div>)}
          </details>}
        </section>
      </div>
    </aside>}
    <section className="iris-main">
      <header className="note-tabs organ-bar">
        {!sidebar && <button className="icon-button" data-tip="Mostrar a barra lateral" aria-label="Mostrar a barra lateral" onClick={() => setSidebar(true)}><SidebarIcon size={18} /></button>}
        <div className="tab-strip iris-kinds" role="tablist" aria-label="Biblioteca da Iris">
          {kinds.map(k => { const on = !designing && kind === k.id; return <button key={k.id} role="tab" aria-selected={on} className={on ? 'note-tab iris-kind active' : 'note-tab iris-kind'} onClick={() => choose(k.id)}><span className="tab-title">{k.label}</span><small>{counts[k.id]}</small></button>; })}
          <button role="tab" aria-selected={designing} className={designing ? 'note-tab iris-kind active' : 'note-tab iris-kind'} onClick={() => setDesigning(true)}><span className="tab-title">Designs</span>{designCount !== null && <small>{designCount}</small>}</button>
        </div>
        {!designing && <button className="icon-button tip-end" data-tip="Atualizar" aria-label="Atualizar a lista" disabled={loading} onClick={load}><RefreshIcon size={17} /></button>}
      </header>
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
    {designing && <main className="iris-grid-area"><IrisDesigns active={active && designing} onCount={setDesignCount} onExported={() => void load()} /></main>}
    <main className="iris-grid-area" ref={grid} hidden={designing}>
      <div className="iris-controls">
      <button className={picking ? 'ghost-button iris-pick-toggle active' : 'ghost-button iris-pick-toggle'} aria-pressed={picking}
        onClick={() => { if (picking) clearPicks(); else setPicking(true); }}>{picking ? 'Cancelar seleção' : 'Selecionar'}</button>
      <label className="iris-order">Ordenar por <select aria-label="Ordenar por" value={order} onChange={event => {
        const value = event.target.value as Order;
        setOrder(value); setShown(PAGE); localStorage.setItem('ayla-iris-ordem', value);
      }}>{Object.entries(orders).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      </div>
      {items === null ? <p className="iris-empty" role="status">Reunindo os arquivos…</p>
        : !visible.length ? <p className="iris-empty">{query || source !== 'all' ? 'Nada encontrado com esse filtro.' : kinds.find(k => k.id === kind)?.empty}</p>
        : <>
            {groups.map(group => <section key={group.label} className="iris-month" aria-label={group.label}>
              {group.label && <h2>{group.label}</h2>}
              <div className="iris-grid" style={{ gridTemplateColumns: `repeat(auto-fill, minmax(min(100%, ${tileSize}px), 1fr))` }}>
                {group.items.map(item => <button key={item.key} className={`iris-card ${item.kind}${picked.has(item.key) ? ' picked' : ''}`} title={`${item.name}\n${itemLocation(item)}`}
                  aria-pressed={picking ? picked.has(item.key) : undefined} onClick={event => clickCard(item, event)}>
                  <span className="iris-thumb"><Thumb key={`${item.date}-${item.bytes}`} item={item} />{picking && <span className="iris-check" aria-hidden="true" />}</span>
                  <span className="iris-caption"><b>{item.name}</b><small>{itemLocation(item)}</small></span>
                </button>)}
              </div>
            </section>)}
            {shown < visible.length && <div ref={more} className="iris-more" aria-hidden="true" />}
          </>}
    </main>
    {picking && !designing && <IrisBatch items={pickedItems} folders={[...pickedFolders.values()]} onClear={clearPicks}
      onChanged={() => { setPicked(new Set()); setPickedFolders(new Map()); anchor.current = null; void load(); }} />}
    </section>
    {current && <div className="iris-viewer" role="dialog" aria-modal="true" aria-label={current.name} onClick={() => { if (!confirmTrash) closeViewer(); }}>
      <header className="iris-viewer-bar" onClick={e => e.stopPropagation()}>
        <span className="iris-viewer-title">{renaming !== null
          ? <form className="iris-rename" onSubmit={e => { e.preventDefault(); void renameCurrent(current, renaming); }}>
              <input autoFocus value={renaming} maxLength={200} aria-label={`Novo nome de ${current.name}`} title="Novo nome (a extensão fica)" onFocus={e => e.currentTarget.select()}
                onChange={e => setRenaming(e.target.value)} onBlur={() => void renameCurrent(current, renaming)} onKeyDown={e => { if (e.key === 'Escape') { e.stopPropagation(); setRenaming(null); } }} />
            </form>
          : <b>{current.name}</b>}<small>{itemLocation(current)} · {day(current.date)} · {size(current.bytes)}</small></span>
        <span className="iris-viewer-count">{index + 1} de {list.length}</span>
        {!outside && <button className="icon-button" data-tip="Renomear" aria-label="Renomear" onClick={() => setRenaming(current.name.replace(/\.[^.]+$/, ''))}><CursorTextIcon size={17} /></button>}
        <button className="icon-button" data-tip="Abrir no visualizador do sistema" aria-label="Abrir no visualizador do sistema" onClick={() => system(current)}><ExpandIcon size={17} /></button>
        {current.kind === 'image' && !/\.gif$/i.test(current.name) && <button className={upscaling ? 'icon-button active' : 'icon-button'} data-tip="Upscale de imagem" aria-label="Upscale de imagem" aria-pressed={upscaling}
          onClick={() => { setUpscaled(''); setUpscaling(v => !v); }}><UpscaleIcon /></button>}
        {(current.kind === 'video' || (current.kind === 'image' && !/\.gif$/i.test(current.name))) && <button className="icon-button" data-tip="Editar" aria-label="Editar"
          onClick={() => { setUpscaled(''); setUpscaling(false); setEditing(true); }}><PencilIcon size={17} /></button>}
        {current.kind === 'pdf' && current.source === 'athena' && !outside && current.root && current.path && <button className="icon-button" data-tip="Ficha de RPG" aria-label="Ficha de RPG"
          onClick={() => { setUpscaled(''); setUpscaling(false); setSheet(true); }}><FichaIcon /></button>}
        {current.kind === 'image' && !outside && <button className="icon-button tip-end" data-tip="Excluir imagem" aria-label="Excluir imagem"
          disabled={confirmTrash} onClick={() => setConfirmTrash(true)}><TrashIcon size={17} /></button>}
        <button className="icon-button tip-end" data-tip="Fechar (Esc)" aria-label="Fechar" disabled={trashing} onClick={closeViewer}><CloseIcon size={18} /></button>
      </header>
      {editing && <IrisEditor key={current.key} target={{ file: current.file, root: current.root, path: current.path, name: current.name, kind: current.kind === 'video' ? 'video' : 'image', src: `${url(current)}?v=${current.date}-${current.bytes}` }}
        onDone={result => void afterEdit(result)} onClose={() => setEditing(false)} />}
      {sheet && current.root && current.path && <IrisFicha key={current.key} target={{ root: current.root, path: current.path, name: current.name }}
        onChanged={() => void load()} onClose={() => setSheet(false)} />}
      {upscaling && <IrisUpscale key={current.key} target={{ file: current.file, root: current.root, path: current.path, name: current.name, src: `${url(current)}?v=${current.date}-${current.bytes}` }} onDone={result => void afterUpscale(result)} onClose={() => setUpscaling(false)} />}
      {upscaled && !upscaling && <div className="iris-upscale" role="status" onClick={e => e.stopPropagation()}><span>{upscaled}</span><button className="ghost-button" onClick={() => setUpscaled('')}>OK</button></div>}
      {confirmTrash && <div className="iris-trash-confirm" role="group" aria-label="Confirmar exclusão" onClick={e => e.stopPropagation()}>
        <span>Mover “{current.name}” para a lixeira do sistema?</span>
        <button className="ghost-button" autoFocus disabled={trashing} onClick={() => { setConfirmTrash(false); setTrashError(''); }}>Cancelar</button>
        <button className="ghost-button" disabled={trashing} onClick={() => void trashCurrent(current)}>Mover para a lixeira</button>
        {trashError && <span role="alert">{trashError}</span>}
      </div>}
      <div className="iris-stage" onClick={e => e.stopPropagation()}>
        {/* Com o editor aberto, o vídeo do visualizador sai (um player a menos aberto). */}
        {!editing && <MediaView key={current.key + current.date + current.bytes} media={mediaOf(current)} name={current.name} kind={current.kind} version={`${current.date}-${current.bytes}`} autoPlay onError={setError} />}
      </div>
      {list.length > 1 && !confirmTrash && !editing && !sheet && <>
        <button className="iris-nav prev" aria-label="Anterior" onClick={e => { e.stopPropagation(); step(-1); }}><ChevronLeftIcon size={26} /></button>
        <button className="iris-nav next" aria-label="Próximo" onClick={e => { e.stopPropagation(); step(1); }}><ChevronRightIcon size={26} /></button>
      </>}
    </div>}
  </div>;
}
