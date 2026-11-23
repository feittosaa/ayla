import { useMemo, useRef, useState, type MouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import Confirm from './Confirm';
import { ImportDialog } from './AthenaImport';
import { ChevronRightIcon, ImportIcon, LockIcon, PlusIcon, UnlockIcon } from './Icons';
import type { LockMode } from './AthenaLock';
import { errorText, native } from '../services/Desktop';
import { natural, readingKey, type Book, type BookLock, type Library } from '../services/Library';

type Item = { source: string; book: Book; next: boolean; at: number };

/** O que continuar: o livro em andamento de cada série ou, se o último foi terminado, o próximo. */
function continuing(library: Library): Item[] {
  const items: Item[] = [];
  for (const source of library.sources) {
    const bySeries = new Map<string, Book[]>();
    for (const book of source.books) bySeries.set(book.series, [...(bySeries.get(book.series) ?? []), book]);
    for (const books of bySeries.values()) {
      books.sort((a, b) => natural(a.path, b.path));
      let last = -1; let at = 0;
      books.forEach((book, index) => {
        const progress = library.reading.books[readingKey(source.id, book.path)];
        if (progress && progress.at > at) { last = index; at = progress.at; }
      });
      if (last < 0) continue;
      if (!library.reading.books[readingKey(source.id, books[last].path)].done) items.push({ source: source.id, book: books[last], next: false, at });
      else {
        const upcoming = books.slice(last + 1).find(b => !library.reading.books[readingKey(source.id, b.path)]?.done);
        if (upcoming) items.push({ source: source.id, book: upcoming, next: true, at });
      }
    }
  }
  return items.sort((a, b) => b.at - a.at).slice(0, 6);
}

/** Biblioteca na lateral da Athena, com a mesma estrutura das Notas: ações no topo (conectar
 * pasta e importar), linha de contexto, busca e a árvore (continuar lendo, fontes, séries e
 * capítulos). Uma bolinha ao passar o mouse seleciona capítulos ou séries inteiras para marcar
 * como lidos ou não lidos. */
export function LibraryPanel({ library, current, onOpen, onReload, onMark, onGuard, onLockNow }: {
  library: Library | null; current?: { source: string; path: string };
  onOpen: (source: string, path: string) => void; onReload: () => Promise<void>;
  onMark: (source: string, paths: string[], done: boolean) => Promise<unknown>;
  /** Senha de uma pasta ou livro dos importados: proteger, abrir, trocar ou tirar. */
  onGuard: (mode: LockMode, path: string, file: boolean) => void;
  onLockNow: (path: string) => void;
}) {
  const [closed, setClosed] = useState<string[]>([]);
  const [open, setOpen] = useState<string[]>([]);
  const [query, setQuery] = useState('');
  const [importing, setImporting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  /** Capítulos escolhidos (`fonte|caminho`) e o último clicado, para o Shift+clique. */
  const [selected, setSelected] = useState<string[]>([]);
  const [anchor, setAnchor] = useState<{ list: string; index: number } | null>(null);
  /** Nova pasta nos importados (caminho da pasta de cima) e o livro sendo arrastado. */
  const [adding, setAdding] = useState<string | null>(null);
  const [folderName, setFolderName] = useState('');
  const [drag, setDrag] = useState<{ path: string; title: string; x: number; y: number; target: string | null } | null>(null);
  const swallow = useRef(false);
  const flip = (list: string[], key: string) => list.includes(key) ? list.filter(k => k !== key) : [...list, key];
  const items = useMemo(() => library ? continuing(library) : [], [library]);
  const state = (source: string, book: Book) => library?.reading.books[readingKey(source, book.path)];
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const matches = (book: Book) => words.every(w => `${book.series} ${book.title}`.toLowerCase().includes(w));

  async function connect() {
    setBusy(true); setError(''); setNotice('');
    try { if (await native<string | null>('library_add_folder')) { setNotice('Pasta conectada.'); await onReload(); } }
    catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  async function createFolder(parent: string) {
    const name = folderName.trim();
    setAdding(null); setFolderName('');
    if (!name) return;
    try {
      const made = await native<string>('library_mkdir', { parent, name });
      setOpen(prev => [...new Set([...prev, `ayla|${parent}`, `ayla|${made}`])]);
      await onReload();
    } catch (e) { setError(errorText(e)); }
  }
  /** Arrastar um livro importado até uma pasta dos importados (ou a raiz Importados). */
  const startDrag = (book: Book) => (event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    const origin = { x: event.clientX, y: event.clientY };
    let moving = false;
    const under = (x: number, y: number) => {
      const found = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>('[data-library-folder]');
      return found ? found.dataset.libraryFolder ?? '' : null;
    };
    const move = (ev: PointerEvent) => {
      if (!moving && Math.hypot(ev.clientX - origin.x, ev.clientY - origin.y) < 5) return;
      moving = true;
      setDrag({ path: book.path, title: book.title, x: ev.clientX, y: ev.clientY, target: under(ev.clientX, ev.clientY) });
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up);
      setDrag(null);
      if (!moving) return;
      swallow.current = true;
      setTimeout(() => { swallow.current = false; }, 0);
      const target = under(ev.clientX, ev.clientY);
      if (target === null || target === book.series) return;
      void native<string>('library_move', { path: book.path, folder: target }).then(() => {
        setOpen(prev => [...new Set([...prev, `ayla|${target}`])]);
        setNotice(`"${book.title}" movido para ${target || 'Importados'}.`);
        return onReload();
      }, e => setError(errorText(e)));
    };
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up);
  };
  async function disconnect(id: string) {
    try { await native('library_remove_folder', { id }); await onReload(); } catch (e) { setError(errorText(e)); }
  }
  /** Selecionados agrupados por fonte (`fonte|caminho`). */
  const bySource = () => {
    const groups = new Map<string, string[]>();
    for (const key of selected) {
      const at = key.indexOf('|');
      groups.set(key.slice(0, at), [...(groups.get(key.slice(0, at)) ?? []), key.slice(at + 1)]);
    }
    return groups;
  };
  /** Apaga os selecionados: lixeira do sistema (recuperáveis); o progresso deles é esquecido. */
  async function remove() {
    const count = selected.length;
    setBusy(true); setError(''); setNotice('');
    try {
      for (const [source, paths] of bySource()) await native('library_trash', { source, paths });
      setNotice(`${count} ${count === 1 ? 'arquivo foi' : 'arquivos foram'} para a lixeira do sistema.`);
      setSelected([]);
      await onReload();
    } catch (e) { setError(errorText(e)); await onReload(); } finally { setBusy(false); }
  }
  async function mark(done: boolean) {
    setBusy(true); setError(''); setNotice('');
    try {
      for (const [source, paths] of bySource()) await onMark(source, paths, done);
      setNotice(`${selected.length} ${selected.length === 1 ? 'capítulo marcado' : 'capítulos marcados'} como ${done ? 'lido' : 'não lido'}${selected.length === 1 ? '' : 's'}.`);
      setSelected([]);
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }

  /** Bolinha de seleção: aparece ao passar o mouse (e sempre, com algo selecionado). */
  const dot = (checked: boolean | 'mixed', label: string, onPick: (event: MouseEvent) => void) =>
    <button type="button" role="checkbox" aria-checked={checked} aria-label={label} title={label}
      className="select-dot" onClick={event => { event.stopPropagation(); onPick(event); }} />;
  /** Ferramentas de senha (só nos importados): proteger; aberto, bloquear, trocar a senha e tirar. */
  const guardTools = (path: string, file: boolean, lock: BookLock | undefined, inside: boolean) => {
    const stop = (action: () => void) => (e: MouseEvent) => { e.stopPropagation(); action(); };
    const label = path.slice(path.lastIndexOf('/') + 1);
    if (!lock) return inside ? null : <button className="icon-button tiny" title="Proteger com senha" aria-label={`Proteger ${label} com senha`} onClick={stop(() => onGuard('protect', path, file))}><LockIcon size={13} /></button>;
    if (!lock.unlocked) return null;
    return <>
      <button className="icon-button tiny" title="Bloquear agora" aria-label={`Bloquear ${label}`} onClick={stop(() => onLockNow(path))}><LockIcon size={13} /></button>
      <button className="icon-button tiny" title="Trocar a senha" aria-label={`Trocar a senha de ${label}`} onClick={stop(() => onGuard('password', path, file))}>✱</button>
      <button className="icon-button tiny" title="Tirar a proteção" aria-label={`Tirar a proteção de ${label}`} onClick={stop(() => onGuard('unprotect', path, file))}><UnlockIcon size={13} /></button>
    </>;
  };
  const pickBook = (list: string, keys: string[], index: number) => (event: MouseEvent) => {
    if (event.shiftKey && anchor?.list === list) {
      const [from, to] = [Math.min(anchor.index, index), Math.max(anchor.index, index)];
      setSelected(prev => [...new Set([...prev, ...keys.slice(from, to + 1)])]);
    } else setSelected(prev => flip(prev, keys[index]));
    setAnchor({ list, index });
  };
  const row = (source: string, book: Book, depth: number, list: { id: string; keys: string[]; index: number }, label = book.title, hint?: string, tools?: ReactNode) => {
    const progress = state(source, book);
    const key = readingKey(source, book.path);
    const checked = selected.includes(key);
    return <div key={`${list.id}|${key}`} role="treeitem" aria-selected={checked}
      className={['tree-row note book', progress?.done ? 'done' : '', checked ? 'checked' : ''].join(' ')}
      aria-current={current?.source === source && current.path === book.path ? 'page' : undefined}
      style={{ paddingLeft: 20 + depth * 14 }} title={`${book.path}\n${book.kind.toUpperCase()}${source === 'ayla' ? '\nArraste para uma pasta para mover' : ''}`}
      onPointerDown={source === 'ayla' ? startDrag(book) : undefined}
      onClick={() => { if (!swallow.current) onOpen(source, book.path); }}>
      <span className="tree-name">{label}</span>
      {hint && <small>{hint}</small>}
      {progress?.done ? <small className="book-state" aria-label="Lido">✓</small>
        : progress && progress.fraction > 0 ? <small className="book-state">{Math.round(progress.fraction * 100)}%</small> : null}
      {tools && <span className="tree-tools">{tools}</span>}
      {dot(checked, checked ? `Tirar ${book.title} da seleção` : `Selecionar ${book.title}`, pickBook(list.id, list.keys, list.index))}
    </div>;
  };

  return <>
    <div className="athena-actions">
      <button className="new-chat" disabled={busy} title="Ler uma pasta no lugar, sem copiar (ex.: a biblioteca do MAD)" onClick={() => void connect()}><PlusIcon size={18} />Conectar pasta</button>
      <button className="ghost-button athena-import" disabled={busy} title="Copiar arquivos .epub e .cbz para a Biblioteca" onClick={() => setImporting(true)}><ImportIcon size={16} />Importar</button>
    </div>
    <p className={error ? 'athena-focus lock-error' : 'athena-focus'} role={error ? 'alert' : 'status'}>
      {error || notice || <>Livros importados vão para <b>Importados</b></>}
    </p>
    <input className="organ-search" type="search" value={query} placeholder="Buscar livros e séries" aria-label="Buscar na Biblioteca" onChange={e => setQuery(e.target.value)} />
    <nav className={selected.length ? 'athena-tree library-tree selecting' : 'athena-tree library-tree'} role="tree" aria-label="Biblioteca" aria-multiselectable="true">
      {!library ? <p className="sidebar-label">Carregando a Biblioteca…</p> : <>
        {!words.length && items.length > 0 && <div className="tree-section">
          <p className="sidebar-label">Continuar lendo</p>
          {items.map((item, index, all) => row(item.source, item.book, 0, { id: 'continuar', keys: all.map(i => readingKey(i.source, i.book.path)), index },
            item.book.series ? `${item.book.series} · ${item.book.title}` : item.book.title, item.next ? 'próximo' : undefined))}
        </div>}
        {!library.sources.some(s => s.books.length) && !words.length && <p className="sidebar-label">Importe livros ou conecte uma pasta (ex.: a biblioteca do MAD).</p>}
        {library.sources.map(source => {
          const books = source.books.filter(matches);
          if (!books.length && (words.length || (source.imported && !source.folders?.length && !library.sources.some(s => s.books.length)))) return null;
          const shown = words.length > 0 || !closed.includes(source.id);
          // Itens protegidos (só nos importados): pastas e livros sozinhos, bloqueados ou abertos.
          const locks = source.locks ?? [];
          const folderLock = (name: string) => locks.find(l => l.kind === 'folder' && (name === l.path || name.startsWith(`${l.path}/`)));
          const lockedFiles = locks.filter(l => l.kind === 'file' && !l.unlocked && (!words.length || words.every(w => l.path.toLowerCase().includes(w))));
          const parentOf = (path: string) => path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
          // Séries: as pastas dos livros e, nos importados, também as pastas vazias criadas na Athena.
          const series = [...new Set([...books.map(b => b.series), ...lockedFiles.map(l => parentOf(l.path)), ...(words.length ? [] : source.folders ?? [])])].sort(natural);
          const lockedRows = (name: string, depth: number) => lockedFiles.filter(l => parentOf(l.path) === name).map(l => {
            const label = l.path.slice(l.path.lastIndexOf('/') + 1).replace(/\.(epub|cbz)$/i, '').replace(/_/g, ' ');
            return <div key={`trancado|${l.path}`} role="treeitem" className="tree-row note book locked" style={{ paddingLeft: 20 + depth * 14 }}
              title={`${l.path}\nProtegido com senha: clique para digitar a senha`} onClick={() => onGuard('unlock', l.path, true)}>
              <span className="tree-file-icon"><LockIcon size={12} /></span><span className="tree-name">{label}</span>
            </div>;
          });
          const bookTools = (book: Book) => source.imported
            ? guardTools(book.path, true, locks.find(l => l.kind === 'file' && l.path === book.path), Boolean(folderLock(book.path))) : undefined;
          const addHere = (parent: string, depth: number) => adding === parent && source.imported
            ? <form className="tree-add" style={{ paddingLeft: 4 + depth * 14 }} onSubmit={e => { e.preventDefault(); void createFolder(parent); }}>
                <input autoFocus value={folderName} maxLength={120} placeholder="Nome da pasta" aria-label="Nome da nova pasta"
                  onChange={e => setFolderName(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') { setAdding(null); setFolderName(''); } }} onBlur={() => void createFolder(parent)} />
              </form> : null;
          const newFolder = (parent: string) => <button className="icon-button tiny" title="Nova pasta aqui" aria-label="Nova pasta aqui"
            onClick={e => { e.stopPropagation(); setAdding(parent); setFolderName(''); if (parent) setOpen(prev => [...new Set([...prev, `ayla|${parent}`])]); else setClosed(prev => prev.filter(id => id !== source.id)); }}>⊞</button>;
          return <div key={source.id} className="tree-section">
            <div className={drag && drag.target === '' && source.imported ? 'tree-row root drop-target' : 'tree-row root'} role="treeitem" aria-expanded={shown} title={source.path}
              data-library-folder={source.imported ? '' : undefined} onClick={() => setClosed(prev => flip(prev, source.id))}>
              <span className={shown ? 'tree-caret open' : 'tree-caret'}><ChevronRightIcon size={13} /></span>
              <span className="tree-name">{source.name}</span>
              {!source.available && <small>indisponível</small>}
              {source.imported && <span className="tree-tools">{newFolder('')}</span>}
              {!source.imported && <span className="tree-tools"><Confirm className="icon-button tiny" title="Desconectar a pasta (os arquivos ficam onde estão)" ariaLabel={`Desconectar ${source.name}`}
                question={`Desconectar "${source.name}" da Biblioteca? Os arquivos e o progresso ficam.`} confirm="Desconectar" onConfirm={() => void disconnect(source.id)}>×</Confirm></span>}
            </div>
            {shown && addHere('', 1)}
            {shown && series.map(name => {
              const inSeries = books.filter(b => b.series === name);
              const keys = inSeries.map(b => readingKey(source.id, b.path));
              if (!name) return [...inSeries.map((book, index) => row(source.id, book, 1, { id: `${source.id}|`, keys, index }, book.title, undefined, bookTools(book))), ...lockedRows('', 1)];
              const key = `${source.id}|${name}`;
              const expanded = words.length > 0 || open.includes(key);
              const read = inSeries.filter(b => state(source.id, b)?.done).length;
              const chosen = keys.filter(k => selected.includes(k)).length;
              const label = name.slice(name.lastIndexOf('/') + 1);
              const level = name.split('/').length - 1;
              const guard = folderLock(name);
              // Pasta protegida bloqueada: sempre fechada; o clique pede a senha.
              if (guard && !guard.unlocked) {
                return guard.path !== name ? null : <div key={key} role="group">
                  <div className="tree-row folder locked" role="treeitem" aria-expanded={false} style={{ paddingLeft: 18 + level * 14 }}
                    title="Pasta protegida: clique para digitar a senha" onClick={() => onGuard('unlock', name, false)}>
                    <span className="tree-caret"><LockIcon size={13} /></span><span className="tree-name">{label}</span>
                  </div>
                </div>;
              }
              return <div key={key} role="group">
                <div className={['tree-row folder', chosen ? 'checked' : '', drag && drag.target === name && source.imported ? 'drop-target' : ''].join(' ')} role="treeitem" aria-expanded={expanded}
                  style={{ paddingLeft: 18 + level * 14 }} data-library-folder={source.imported ? name : undefined} onClick={() => setOpen(prev => flip(prev, key))}>
                  <span className={expanded ? 'tree-caret open' : 'tree-caret'}><ChevronRightIcon size={13} /></span>
                  <span className="tree-name">{label}</span><small>{read}/{inSeries.length}</small>
                  {guard?.path === name && <span className="tree-lock" title="Pasta protegida, aberta"><UnlockIcon size={12} /></span>}
                  {source.imported && <span className="tree-tools">{!guard && newFolder(name)}{guardTools(name, false, guard?.path === name ? guard : undefined, Boolean(guard))}</span>}
                  {keys.length > 0 && dot(chosen === keys.length ? true : chosen ? 'mixed' : false, chosen === keys.length ? `Tirar ${label} da seleção` : `Selecionar todos de ${label}`,
                    () => setSelected(prev => chosen === keys.length ? prev.filter(k => !keys.includes(k)) : [...new Set([...prev, ...keys])]))}
                </div>
                {expanded && addHere(name, level + 2)}
                {expanded && inSeries.map((book, index) => row(source.id, book, 2 + level, { id: key, keys, index }, book.title, undefined, bookTools(book)))}
                {expanded && lockedRows(name, 2 + level)}
              </div>;
            })}
          </div>;
        })}
      </>}
    </nav>
    {selected.length > 0 && <div className="athena-selection" role="toolbar" aria-label="Capítulos selecionados">
      <span>{selected.length} {selected.length === 1 ? 'selecionado' : 'selecionados'}</span>
      <div className="athena-actions">
        <button className="new-chat" disabled={busy} onClick={() => void mark(true)}>Marcar como lido</button>
        <button className="ghost-button" disabled={busy} onClick={() => void mark(false)}>Não lido</button>
      </div>
      <div className="athena-actions">
        <Confirm className="ghost-button" disabled={busy} title="Apagar os selecionados (vão para a lixeira do sistema)" ariaLabel="Apagar os selecionados"
          question={`Apagar ${selected.length} ${selected.length === 1 ? 'arquivo' : 'arquivos'}? ${selected.length === 1 ? 'Ele vai' : 'Eles vão'} para a lixeira do sistema e o progresso é esquecido.`}
          confirm="Apagar" onConfirm={() => void remove()}>Apagar</Confirm>
        <button className="ghost-button" disabled={busy} title="Limpar a seleção" aria-label="Limpar a seleção" onClick={() => setSelected([])}>×</button>
      </div>
    </div>}
    {drag && <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>{drag.title}{drag.target !== null && <small> → {drag.target ? drag.target.slice(drag.target.lastIndexOf('/') + 1) : 'Importados'}</small>}</div>}
    {importing && <ImportDialog books place="Importados" onDone={() => void onReload()} onClose={() => setImporting(false)} />}
  </>;
}
