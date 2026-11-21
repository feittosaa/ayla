import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import Markdown from './Markdown';
import Confirm from './Confirm';
import { BookIcon, ChevronRightIcon, PlusIcon, TrashIcon } from './Icons';
import { errorText, native } from '../services/Desktop';

type Note = { path: string; title: string; modified: number; bytes: number };
type Root = { id: string; kind: 'ayla' | 'vault' | 'project'; name: string; path: string; available: boolean; obsidian: boolean; git: boolean; folders: string[]; notes: Note[] };
type Tree = { roots: Root[]; suggested: string[]; git_enabled: boolean };
type Hit = { root: string; rootName: string; path: string; title: string; snippet: string };
type Outcome = { committed: number; pulled: boolean; pushed: boolean; message: string };
type GitInfo = { enabled: boolean; obsidian: boolean; status: { repo: boolean; branch: string; changes: number; remote: boolean }; last: { at: number; outcome: Outcome } | null };
type Open = { root: string; path: string };
type Adding = { root: string; folder: string; kind: 'note' | 'folder' };
type Folder = { name: string; path: string; folders: Folder[]; notes: Note[] };
type TagEntry = { tag: string; notes: { root: string; rootName: string; path: string; title: string }[] };

/** Pastas e notas de uma raiz em árvore, em ordem alfabética (como o Obsidian). */
function build(root: Root): Folder {
  const top: Folder = { name: root.name, path: '', folders: [], notes: [] };
  const find = (path: string): Folder => {
    if (!path) return top;
    const parent = find(path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
    const name = path.slice(path.lastIndexOf('/') + 1);
    let folder = parent.folders.find(f => f.name === name);
    if (!folder) { folder = { name, path, folders: [], notes: [] }; parent.folders.push(folder); }
    return folder;
  };
  root.folders.forEach(find);
  root.notes.forEach(note => find(note.path.includes('/') ? note.path.slice(0, note.path.lastIndexOf('/')) : '').notes.push(note));
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

/** Athena: notas em Markdown em árvore — notas da Ayla, cofres (ex.: do Obsidian) e projetos —, com
 * links entre notas, busca, editor, nota do dia e Git por cofre. */
export default function Athena({ active, onOpenSettings }: { active: boolean; onOpenSettings: () => void }) {
  const [tree, setTree] = useState<Tree | null>(null); const [expanded, setExpanded] = useState<string[]>(['ayla|']);
  const [query, setQuery] = useState(''); const [hits, setHits] = useState<Hit[] | null>(null);
  const [current, setCurrent] = useState<Open | null>(null); const [content, setContent] = useState(''); const [editing, setEditing] = useState(false);
  const [backlinks, setBacklinks] = useState<{ path: string; title: string }[]>([]);
  const [status, setStatus] = useState(''); const [error, setError] = useState(''); const [adding, setAdding] = useState<Adding | null>(null); const [name, setName] = useState('');
  const [confirming, setConfirming] = useState(false); const [git, setGit] = useState<GitInfo | null>(null); const [syncing, setSyncing] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null); const [folderEdit, setFolderEdit] = useState<{ root: string; path: string; name: string } | null>(null);
  const [tags, setTags] = useState<TagEntry[]>([]); const [tag, setTag] = useState<string | null>(null); const [showTags, setShowTags] = useState(false);
  const [suggest, setSuggest] = useState<{ query: string; start: number } | null>(null); const editor = useRef<HTMLTextAreaElement | null>(null);
  const stamp = useRef<number | null>(null); const timer = useRef<ReturnType<typeof setTimeout> | null>(null); const dirty = useRef(false);

  const load = useCallback(() => { void native<Tree>('athena_tree').then(setTree).catch(e => setError(errorText(e))); }, []);
  const rootOf = useCallback((id: string) => tree?.roots.find(r => r.id === id) ?? null, [tree]);
  const expand = useCallback((root: string, path: string) => {
    const keys = [`${root}|`];
    const parts = path.split('/').slice(0, -1);
    parts.forEach((_, i) => keys.push(`${root}|${parts.slice(0, i + 1).join('/')}`));
    setExpanded(prev => [...new Set([...prev, ...keys])]);
  }, []);
  const open = useCallback(async (root: string, path: string, edit = false) => {
    setError(''); setConfirming(false);
    try {
      const note = await native<{ content: string; modified: number }>('athena_read', { root, path });
      setCurrent({ root, path }); setContent(note.content); stamp.current = note.modified; dirty.current = false; setStatus('');
      setEditing(edit || !note.content.trim() || /^# [^\n]*\n*$/.test(note.content)); expand(root, path);
      void native<{ path: string; title: string }[]>('athena_backlinks', { root, path }).then(setBacklinks).catch(() => setBacklinks([]));
      void native<GitInfo>('athena_git_status', { root }).then(setGit).catch(() => setGit(null));
    } catch (e) { setError(errorText(e)); }
  }, [expand]);
  const loadTags = useCallback(() => { void native<TagEntry[]>('athena_tags').then(setTags).catch(() => setTags([])); }, []);
  useEffect(() => { if (active) { load(); loadTags(); } }, [active, load, loadTags]);
  const loadAttachment = useCallback((target: string) => current ? native<string | null>('athena_attachment', { root: current.root, target }) : Promise.resolve(null), [current]);
  useEffect(() => {
    const stop = listen<{ root?: string; path?: string }>('notes-updated', event => { load(); if (event.payload?.root && event.payload.path) void open(event.payload.root, event.payload.path); });
    return () => { void stop.then(off => off()); };
  }, [load, open]);
  useEffect(() => {
    if (!query.trim()) return;
    const id = setTimeout(() => void native<Hit[]>('athena_search', { query }).then(setHits).catch(e => setError(errorText(e))), 250);
    return () => clearTimeout(id);
  }, [query]);

  const save = useCallback(async (target: Open, text: string) => {
    try {
      stamp.current = await native<number>('athena_write', { root: target.root, path: target.path, content: text, expected: stamp.current });
      dirty.current = false; setStatus('Salva'); load(); loadTags();
    } catch (e) { setError(errorText(e)); await open(target.root, target.path, true); }
  }, [load, open]);
  function change(text: string) {
    setContent(text); dirty.current = true; setStatus('Salvando…');
    if (timer.current) clearTimeout(timer.current);
    const target = current;
    if (target) timer.current = setTimeout(() => void save(target, text), 700);
  }
  async function flush() {
    if (timer.current) clearTimeout(timer.current);
    if (dirty.current && current) await save(current, content);
  }
  async function go(root: string, path: string, edit = false) { await flush(); await open(root, path, edit); }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!adding) return;
    const { root, folder, kind } = adding; setError('');
    try {
      if (kind === 'folder') { const made = await native<string>('athena_mkdir', { root, parent: folder, name: name || 'Nova pasta' }); setExpanded(prev => [...prev, `${root}|${made}`, `${root}|${folder}`]); load(); }
      else { const made = await native<string>('athena_create', { root, folder, title: name || 'Nova nota' }); load(); await go(root, made, true); }
      setAdding(null); setName('');
    } catch (e) { setError(errorText(e)); }
  }
  async function today() {
    const root = current?.root ?? 'ayla';
    try { const daily = await native<string>('athena_daily', { root }); load(); await go(root, daily, true); } catch (e) { setError(errorText(e)); }
  }
  async function remove() {
    if (!current) return;
    try { await native('athena_trash', current); setCurrent(null); setContent(''); setConfirming(false); load(); } catch (e) { setError(errorText(e)); }
  }
  async function addVault(path?: string) {
    setError('');
    try {
      const folder = path ?? await native<string | null>('athena_pick_folder');
      if (!folder) return;
      await native('athena_add_vault', { path: folder });
      setExpanded(prev => [...prev, `vault:${folder}|`]); load();
    } catch (e) { setError(errorText(e)); }
  }
  async function closeVault(root: Root) {
    try { await native('athena_remove_vault', { path: root.path }); if (current?.root === root.id) setCurrent(null); load(); } catch (e) { setError(errorText(e)); }
  }
  async function sync() {
    if (!current) return;
    setSyncing(true); setError('');
    try { const outcome = await native<Outcome>('athena_git_sync', { root: current.root }); setStatus(outcome.message); load(); await open(current.root, current.path, editing); }
    catch (e) { setError(errorText(e)); } finally { setSyncing(false); }
  }
  /** Clique num link [[…]]: abre a nota da mesma raiz ou, como no Obsidian, cria se ainda não existe. */
  /** Renomeia ou move a nota (o núcleo atualiza os links nas outras notas). */
  async function moveNote(to: string) {
    if (!current) return;
    await flush(); setError('');
    try {
      const result = await native<{ path: string; relinked: number }>('athena_rename', { root: current.root, path: current.path, to });
      setRenaming(null); load(); loadTags(); await open(current.root, result.path, editing);
      setStatus(result.relinked ? `Links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}` : 'Nota renomeada');
    } catch (e) { setError(errorText(e)); }
  }
  async function renameFolder(event: FormEvent) {
    event.preventDefault();
    if (!folderEdit) return;
    const { root, path, name: next } = folderEdit;
    const parent = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
    const to = next.includes('/') ? next : (parent ? `${parent}/${next}` : next);
    try {
      const result = await native<{ path: string; relinked: number }>('athena_rename_folder', { root, path, to });
      setFolderEdit(null); setExpanded(prev => [...prev, `${root}|${result.path}`]); load();
      if (current?.root === root && current.path.startsWith(`${path}/`)) await open(root, `${result.path}${current.path.slice(path.length)}`, editing);
      if (result.relinked) setStatus(`Pasta renomeada · links atualizados em ${result.relinked} ${result.relinked === 1 ? 'nota' : 'notas'}`);
    } catch (e) { setError(errorText(e)); }
  }
  /** Barra do editor: envolve a seleção ou insere no começo da linha. */
  function format(before: string, after = '', line = false) {
    const field = editor.current;
    if (!field) return;
    const { selectionStart: a, selectionEnd: b } = field;
    let next: string; let caret: number;
    if (line) {
      const start = content.lastIndexOf('\n', a - 1) + 1;
      next = content.slice(0, start) + before + content.slice(start); caret = b + before.length;
    } else {
      next = content.slice(0, a) + before + content.slice(a, b) + after + content.slice(b); caret = b + before.length;
    }
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
    if (!field || !suggest) return;
    const caret = field.selectionStart;
    const next = content.slice(0, suggest.start) + title + ']]' + content.slice(caret).replace(/^\]\]/, '');
    change(next); setSuggest(null);
    const position = suggest.start + title.length + 2;
    requestAnimationFrame(() => { field.focus(); field.setSelectionRange(position, position); });
  }

  async function follow(target: string) {
    if (!current) return;
    const root = rootOf(current.root);
    const found = root && resolve(root, target);
    if (found) { await go(current.root, found); return; }
    try {
      const folder = target.includes('/') ? target.slice(0, target.lastIndexOf('/')) : '';
      const made = await native<string>('athena_create', { root: current.root, folder, title: target.slice(target.lastIndexOf('/') + 1) });
      load(); await go(current.root, made, true);
    } catch (e) { setError(errorText(e)); }
  }

  const toggle = (key: string) => setExpanded(prev => prev.includes(key) ? prev.filter(k => k !== key) : [...prev, key]);
  const roots = useMemo(() => tree?.roots ?? [], [tree]);
  const openRoot = current ? rootOf(current.root) : null;
  const title = current ? (current.path.slice(current.path.lastIndexOf('/') + 1).replace(/\.md$/i, '')) : '';

  function addForm(root: string, folder: string, depth: number) {
    if (!adding || adding.root !== root || adding.folder !== folder) return null;
    return <form className="tree-add" style={{ paddingLeft: 12 + depth * 14 }} onSubmit={e => void submit(e)}>
      <input autoFocus value={name} maxLength={120} placeholder={adding.kind === 'folder' ? 'Nome da pasta' : 'Título da nota'} onChange={e => setName(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') { setAdding(null); setName(''); } }} />
    </form>;
  }
  function tools(root: string, folder: string): ReactNode {
    const start = (kind: Adding['kind']) => (e: { stopPropagation: () => void }) => { e.stopPropagation(); setAdding({ root, folder, kind }); setName(''); setExpanded(prev => [...new Set([...prev, `${root}|${folder}`])]); };
    return <span className="tree-tools">
      <button className="icon-button tiny" title="Nova nota aqui" aria-label="Nova nota aqui" onClick={start('note')}><PlusIcon size={14} /></button>
      <button className="icon-button tiny" title="Nova pasta aqui" aria-label="Nova pasta aqui" onClick={start('folder')}>⊞</button>
      {folder && <button className="icon-button tiny" title="Renomear ou mover a pasta" aria-label="Renomear ou mover a pasta" onClick={e => { e.stopPropagation(); setFolderEdit({ root, path: folder, name: folder.slice(folder.lastIndexOf('/') + 1) }); }}>✎</button>}
    </span>;
  }
  function folderNode(root: Root, folder: Folder, depth: number): ReactNode {
    const key = `${root.id}|${folder.path}`;
    const shown = !folder.path || expanded.includes(key);
    const inner = folder.path ? depth + 1 : depth;
    return <div key={key} role="group">
      {folder.path && (folderEdit?.root === root.id && folderEdit.path === folder.path
        ? <form className="tree-add" style={{ paddingLeft: 4 + depth * 14 }} onSubmit={e => void renameFolder(e)}>
            <input autoFocus value={folderEdit.name} maxLength={200} title="Novo nome, ou caminho (ex.: Jogos/RPG) para mover" onChange={e => setFolderEdit({ ...folderEdit, name: e.target.value })} onKeyDown={e => { if (e.key === 'Escape') setFolderEdit(null); }} />
          </form>
        : <div className="tree-row folder" style={{ paddingLeft: 4 + depth * 14 }} onClick={() => toggle(key)} role="treeitem" aria-expanded={shown}>
            <span className={shown ? 'tree-caret open' : 'tree-caret'}><ChevronRightIcon size={13} /></span><span className="tree-name">{folder.name}</span>{tools(root.id, folder.path)}
          </div>)}
      {shown && <>
        {addForm(root.id, folder.path, inner)}
        {folder.folders.map(child => folderNode(root, child, inner))}
        {folder.notes.map(note => <div key={note.path} role="treeitem" className="tree-row note" aria-current={current?.root === root.id && current.path === note.path ? 'page' : undefined} style={{ paddingLeft: 20 + inner * 14 }} title={note.path} onClick={() => void go(root.id, note.path)}>
          <span className="tree-name">{note.title}</span>
        </div>)}
      </>}
    </div>;
  }
  function rootNode(root: Root): ReactNode {
    const key = `${root.id}|`;
    const shown = expanded.includes(key);
    return <div key={root.id} className="tree-root">
      <div className="tree-row root" onClick={() => root.available && toggle(key)} role="treeitem" aria-expanded={shown} title={root.path}>
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

  const sections: Root['kind'][] = ['ayla', 'vault', 'project'];
  return <div className="chat-layout athena">
    <aside className="sidebar athena-sidebar" aria-label="Notas da Athena">
      <div className="brand"><BookIcon size={22} /><span className="brand-name">Athena</span></div>
      <div className="athena-actions">
        <button className="new-chat" onClick={() => void addVault()}><PlusIcon size={18} />Abrir cofre</button>
        <button className="ghost-button" onClick={() => void today()}>Hoje</button>
      </div>
      {tree?.suggested.map(path => <div key={path} className="athena-suggest"><span title={path}>Cofre do Obsidian: <b>{path.slice(path.lastIndexOf('/') + 1)}</b></span><button className="link-button" onClick={() => void addVault(path)}>Abrir</button></div>)}
      {tags.length > 0 && <div className="athena-tags">
        <button className="link-button" aria-expanded={showTags} onClick={() => setShowTags(v => !v)}>{showTags ? '▾' : '▸'} Tags ({tags.length})</button>
        {showTags && <div className="tag-list">{tags.map(t => <button key={t.tag} className={tag === t.tag ? 'tag-chip active' : 'tag-chip'} onClick={() => { setTag(tag === t.tag ? null : t.tag); setQuery(''); setHits(null); }}>#{t.tag} <small>{t.notes.length}</small></button>)}</div>}
      </div>}
      <input className="athena-search" type="search" value={query} placeholder="Buscar em todas as notas" aria-label="Buscar nas notas" onChange={e => { setQuery(e.target.value); if (!e.target.value.trim()) setHits(null); }} />
      <nav className="athena-tree" role="tree" aria-label="Pastas e notas">
        {tag && !query.trim()
          ? <>
              <div className="tree-filter"><span>#{tag}</span><button className="link-button" onClick={() => setTag(null)}>Limpar</button></div>
              {(tags.find(t => t.tag === tag)?.notes ?? []).map(n => <div key={`${n.root}|${n.path}`} className="tree-row note hit" onClick={() => void go(n.root, n.path)} title={n.path}><span className="tree-name">{n.title}</span><small>{n.rootName}</small></div>)}
            </>
          : query.trim() && hits
          ? (hits.length ? hits.map(hit => <div key={`${hit.root}|${hit.path}`} className="tree-row note hit" onClick={() => void go(hit.root, hit.path)} title={hit.path}>
              <span className="tree-name">{hit.title}</span><small>{hit.rootName}{hit.snippet ? ` · ${hit.snippet}` : ''}</small>
            </div>) : <p className="sidebar-label">Nada encontrado.</p>)
          : sections.map(kind => {
            const group = roots.filter(r => r.kind === kind);
            if (!group.length) return null;
            return <div key={kind} className="tree-section">{sectionLabel[kind] && <p className="sidebar-label">{sectionLabel[kind]}</p>}{group.map(rootNode)}</div>;
          })}
      </nav>
      {openRoot?.git && git?.enabled && <div className="athena-git">
        <span>{openRoot.name} · Git {git.status.branch}{git.status.changes ? ` · ${git.status.changes} ${git.status.changes === 1 ? 'alteração' : 'alterações'}` : ' · em dia'}</span>
        {git.last?.outcome.message && <small>{git.last.outcome.message}</small>}
        {git.obsidian && <small>Obsidian aberto: a sincronização automática fica com o plugin.</small>}
        <button className="ghost-button" disabled={syncing} onClick={() => void sync()}>{syncing ? 'Sincronizando…' : git.status.remote ? 'Sincronizar com o GitHub' : 'Versionar'}</button>
      </div>}
    </aside>
    <main className="athena-main">
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
      {current ? <>
        <header className="athena-header">
          <div className="athena-heading">
            <small>{openRoot?.name}{current.path.includes('/') ? ` / ${current.path.slice(0, current.path.lastIndexOf('/')).split('/').join(' / ')}` : ''}</small>
            {renaming !== null
              ? <form onSubmit={e => { e.preventDefault(); const folder = current.path.includes('/') ? current.path.slice(0, current.path.lastIndexOf('/') + 1) : ''; void moveNote(`${folder}${renaming}`); }}><input className="athena-rename" autoFocus value={renaming} maxLength={120} aria-label="Novo nome da nota" onChange={e => setRenaming(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') setRenaming(null); }} onBlur={() => setRenaming(null)} /></form>
              : <h1 title="Clique para renomear" onClick={() => setRenaming(title)}>{title}</h1>}
          </div>
          {openRoot && <select className="athena-move" aria-label="Mover para a pasta" value={current.path.includes('/') ? current.path.slice(0, current.path.lastIndexOf('/')) : ''} onChange={e => void moveNote(`${e.target.value ? `${e.target.value}/` : ''}${title}`)}>
            <option value="">{openRoot.name} (raiz)</option>
            {openRoot.folders.map(f => <option key={f} value={f}>{f.split('/').join(' / ')}</option>)}
          </select>}
          <span className="athena-status" role="status">{status}</span>
          <div className="segmented" role="group" aria-label="Modo da nota">
            <label><input type="radio" name="athena-mode" checked={!editing} onChange={() => setEditing(false)} /><span>Ver</span></label>
            <label><input type="radio" name="athena-mode" checked={editing} onChange={() => setEditing(true)} /><span>Editar</span></label>
          </div>
          {openRoot?.obsidian && <button className="ghost-button" title="Abrir no Obsidian, com os plugins do cofre" onClick={() => void flush().then(() => native('athena_open_obsidian', current)).catch(e => setError(errorText(e)))}>Abrir no Obsidian</button>}
          {confirming
            ? <span className="row-actions"><button onClick={() => void remove()}>Mover para a lixeira</button><button className="ghost-button" onClick={() => setConfirming(false)}>Cancelar</button></span>
            : <button className="icon-button" aria-label="Apagar nota" title="Apagar nota" onClick={() => setConfirming(true)}><TrashIcon size={18} /></button>}
        </header>
        {editing
          ? <div className="athena-edit">
              <div className="athena-toolbar" role="toolbar" aria-label="Formatação">
                <button type="button" title="Negrito" onClick={() => format('**', '**')}><b>B</b></button>
                <button type="button" title="Itálico" onClick={() => format('*', '*')}><i>I</i></button>
                <button type="button" title="Título" onClick={() => format('## ', '', true)}>H</button>
                <button type="button" title="Lista" onClick={() => format('- ', '', true)}>•</button>
                <button type="button" title="Tarefa" onClick={() => format('- [ ] ', '', true)}>☐</button>
                <button type="button" title="Link para nota" onClick={() => { format('[[', ']]'); setSuggest({ query: '', start: (editor.current?.selectionStart ?? 0) + 2 }); }}>[[ ]]</button>
                <button type="button" title="Tag" onClick={() => format('#')}>#</button>
              </div>
              <textarea ref={editor} className="athena-editor" value={content} spellCheck aria-label="Conteúdo da nota" onChange={e => typed(e.target.value, e.target.selectionStart)} onBlur={() => { setTimeout(() => setSuggest(null), 150); void flush(); }} onKeyDown={e => { if (e.key === 'Escape') setSuggest(null); }} />
              {suggest && openRoot && (() => {
                const q = suggest.query.toLowerCase();
                const options = openRoot.notes.filter(n => n.title.toLowerCase().includes(q) && n.path !== current.path).slice(0, 8);
                return options.length ? <ul className="link-suggest" role="listbox" aria-label="Notas para o link">{options.map(n => <li key={n.path}><button type="button" onMouseDown={e => { e.preventDefault(); pick(n.title); }}>{n.title}<small>{n.path.includes('/') ? n.path.slice(0, n.path.lastIndexOf('/')) : ''}</small></button></li>)}</ul> : null;
              })()}
            </div>
          : <div className="athena-view" onDoubleClick={() => setEditing(true)}>
              {content.trim() ? <Markdown text={content} onNoteLink={target => void follow(target)} onTag={t => { setTag(t); setShowTags(true); setQuery(''); setHits(null); }} loadAttachment={loadAttachment} /> : <p className="athena-empty">Nota vazia. Clique em Editar para escrever.</p>}
              {backlinks.length > 0 && <section className="athena-backlinks"><h2>Mencionada em</h2>{backlinks.map(b => <button key={b.path} className="link-button" onClick={() => void go(current.root, b.path)}>{b.title}</button>)}</section>}
            </div>}
      </> : <div className="athena-welcome">
        <h1>Athena</h1>
        <p>Suas notas em Markdown, em pastas, como no Obsidian: as notas da Ayla, os seus cofres e uma pasta para cada projeto da Ayla. Links como [[nota]] funcionam entre as notas.</p>
        <p>Na conversa, a Ayla consulta as notas e cita de onde tirou. Peça também "cria uma nota sobre…" ou "salva isso como nota".</p>
        <div className="row-actions"><button className="primary-action" onClick={() => void addVault()}>Abrir cofre</button><button onClick={() => void today()}>Nota de hoje</button><button className="ghost-button" onClick={onOpenSettings}>Configurações</button></div>
      </div>}
    </main>
  </div>;
}
