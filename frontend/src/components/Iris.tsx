import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import MediaView from './MediaView';
import { preloadPdf } from '../services/PdfJs';
import { ChevronLeftIcon, ChevronRightIcon, CloseIcon, ExpandIcon, FileIcon, IrisIcon, RefreshIcon, SidebarIcon, VideoIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import { mediaUrl, openInSystem, type MediaRef } from '../services/Media';

type Kind = 'image' | 'video' | 'pdf';
type Source = 'athena' | 'chat' | 'gerada' | 'arquivos';
type Item = {
  key: string; kind: Kind; name: string; source: Source; place: string; date: number; bytes: number;
  /** Arquivo guardado na Ayla (anexo do chat, imagem gerada ou adicionado direto). */
  file?: string;
  /** Arquivo de uma raiz da Athena (cofre, notas ou projeto). */
  root?: string; path?: string;
};

const kinds: { id: Kind; label: string; empty: string }[] = [
  { id: 'image', label: 'Imagens', empty: 'Nenhuma imagem ainda. Fotos dos cofres da Athena, anexos do chat e imagens criadas pela Ayla aparecem aqui.' },
  { id: 'video', label: 'Vídeos', empty: 'Nenhum vídeo ainda. Vídeos dos cofres da Athena e anexos do chat aparecem aqui.' },
  { id: 'pdf', label: 'PDFs', empty: 'Nenhum PDF ainda. PDFs dos cofres da Athena, anexos do chat e arquivos adicionados aparecem aqui.' },
];
const sources: { id: Source | 'all'; label: string }[] = [
  { id: 'all', label: 'Tudo' }, { id: 'athena', label: 'Athena' }, { id: 'chat', label: 'Chat' }, { id: 'gerada', label: 'Geradas' }, { id: 'arquivos', label: 'Arquivos' },
];
const sourceLabel: Record<Source, string> = { athena: 'Athena', chat: 'Anexo do chat', gerada: 'Gerada pela Ayla', arquivos: 'Arquivos' };
const TAB = 'ayla-iris-tipo';
const SIDE = 'ayla-iris-lateral';
const PAGE = 120;

/** O item como mídia do serviço comum (o mesmo da Athena). */
const mediaOf = (item: Item): MediaRef => item.file ? { file: item.file } : { root: item.root ?? '', path: item.path ?? '' };
const url = (item: Item, thumb = false) => mediaUrl(mediaOf(item), thumb);
const month = (seconds: number) => { const text = new Date(seconds * 1000).toLocaleDateString('pt-BR', { month: 'long', year: 'numeric' }); return text.charAt(0).toUpperCase() + text.slice(1); };
const size = (bytes: number) => bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(1)} GB` : bytes >= 1024 ** 2 ? `${(bytes / 1024 ** 2).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`;
const day = (seconds: number) => new Date(seconds * 1000).toLocaleDateString('pt-BR', { day: '2-digit', month: 'short', year: 'numeric' });

/** Miniatura de um item na grade: imagem reduzida ou primeira página do PDF (geradas pelo núcleo); vídeo com ícone. */
function Thumb({ item }: { item: Item }) {
  const [failed, setFailed] = useState(false);
  if (item.kind === 'video' || failed) return <span className="iris-placeholder">{item.kind === 'video' ? <VideoIcon size={30} /> : <FileIcon size={30} />}<b>{item.kind === 'pdf' ? 'PDF' : item.name.split('.').pop()?.toUpperCase()}</b></span>;
  return <img src={url(item, true)} alt="" loading="lazy" decoding="async" onError={() => setFailed(true)} />;
}

/** Iris: as imagens, vídeos e PDFs da Ayla num lugar só — dos cofres e notas da Athena, os
 * anexos enviados no chat, as imagens que a Ayla gerou e os arquivos adicionados direto. */
export default function Iris({ active }: { active: boolean }) {
  const [items, setItems] = useState<Item[] | null>(null); const [error, setError] = useState('');
  const [kind, setKind] = useState<Kind>(() => (localStorage.getItem(TAB) as Kind | null) ?? 'image');
  const [source, setSource] = useState<Source | 'all'>('all'); const [query, setQuery] = useState('');
  const [open, setOpen] = useState<string | null>(null); const [shown, setShown] = useState(PAGE);
  const [loading, setLoading] = useState(false);
  const [sidebar, setSidebar] = useState(() => localStorage.getItem(SIDE) !== '0');
  const more = useRef<HTMLDivElement | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    native<Item[]>('iris_library').then(list => { setItems(list); setError(''); }, e => setError(errorText(e))).finally(() => setLoading(false));
  }, []);
  useEffect(() => {
    if (!active) return;
    let alive = true;
    native<Item[]>('iris_library').then(list => { if (alive) { setItems(list); setError(''); } }, e => { if (alive) setError(errorText(e)); });
    return () => { alive = false; };
  }, [active]);
  useEffect(() => { localStorage.setItem(TAB, kind); }, [kind]);
  useEffect(() => { if (active) preloadPdf(); }, [active]);
  useEffect(() => { localStorage.setItem(SIDE, sidebar ? '1' : '0'); }, [sidebar]);
  useEffect(() => {
    const stop = listen('notes-updated', () => { if (active) load(); });
    return () => { void stop.then(off => off()); };
  }, [active, load]);

  const counts = useMemo(() => {
    const out: Record<Kind, number> = { image: 0, video: 0, pdf: 0 };
    for (const item of items ?? []) out[item.kind]++;
    return out;
  }, [items]);
  const visible = useMemo(() => {
    const words = query.trim().toLowerCase();
    return (items ?? []).filter(item => item.kind === kind && (source === 'all' || item.source === source)
      && (!words || `${item.name} ${item.place}`.toLowerCase().includes(words)));
  }, [items, kind, source, query]);
  const perSource = useMemo(() => {
    const out: Record<string, number> = { all: 0 };
    for (const item of items ?? []) if (item.kind === kind) { out.all++; out[item.source] = (out[item.source] ?? 0) + 1; }
    return out;
  }, [items, kind]);
  // Mais itens ao chegar perto do fim da grade (não carrega tudo de uma vez).
  useEffect(() => {
    const sentinel = more.current;
    if (!sentinel) return;
    const observer = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) setShown(n => n + PAGE); }, { rootMargin: '600px' });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [visible.length, shown]);
  const groups = useMemo(() => {
    const out: { label: string; items: Item[] }[] = [];
    for (const item of visible.slice(0, shown)) {
      const label = month(item.date);
      if (out[out.length - 1]?.label !== label) out.push({ label, items: [] });
      out[out.length - 1].items.push(item);
    }
    return out;
  }, [visible, shown]);

  const index = open ? visible.findIndex(item => item.key === open) : -1;
  const current = index >= 0 ? visible[index] : null;
  const step = useCallback((delta: number) => {
    if (index < 0 || !visible.length) return;
    setOpen(visible[(index + delta + visible.length) % visible.length].key);
  }, [index, visible]);
  useEffect(() => {
    if (!current) return;
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(null);
      else if (event.key === 'ArrowRight' && current.kind !== 'pdf') step(1);
      else if (event.key === 'ArrowLeft' && current.kind !== 'pdf') step(-1);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [current, step]);

  function system(item: Item) {
    void openInSystem(mediaOf(item)).catch(e => setError(errorText(e)));
  }
  const choose = (next: Kind) => { setKind(next); setShown(PAGE); setSource('all'); };

  // Mesmo esqueleto da Athena: barra lateral (marca, busca, origem) e, à direita, a barra
  // superior com os tipos de arquivo no lugar das abas.
  return <div className="chat-layout iris">
    {sidebar && <aside className="sidebar organ-sidebar iris-sidebar" aria-label="Filtros da Iris">
      <div className="brand"><IrisIcon size={22} /><span className="brand-name">Iris</span><button className="icon-button organ-hide" data-tip="Esconder a barra lateral" aria-label="Esconder a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon size={18} /></button></div>
      <input className="organ-search" type="search" value={query} placeholder="Buscar pelo nome ou lugar" aria-label="Buscar arquivos" onChange={e => { setQuery(e.target.value); setShown(PAGE); }} />
      <div className="iris-sources" role="group" aria-label="Origem">
        <p className="sidebar-label">Origem</p>
        {sources.map(s => (s.id === 'all' || perSource[s.id]) ? <button key={s.id} className={source === s.id ? 'iris-source active' : 'iris-source'} aria-pressed={source === s.id} onClick={() => { setSource(s.id); setShown(PAGE); }}><span>{s.label}</span><small>{perSource[s.id] ?? 0}</small></button> : null)}
      </div>
    </aside>}
    <section className="iris-main">
      <header className="note-tabs organ-bar">
        {!sidebar && <button className="icon-button" data-tip="Mostrar a barra lateral" aria-label="Mostrar a barra lateral" onClick={() => setSidebar(true)}><SidebarIcon size={18} /></button>}
        <div className="tab-strip iris-kinds" role="tablist" aria-label="Tipo de arquivo">
          {kinds.map(k => <button key={k.id} role="tab" aria-selected={kind === k.id} className={kind === k.id ? 'note-tab iris-kind active' : 'note-tab iris-kind'} onClick={() => choose(k.id)}><span className="tab-title">{k.label}</span><small>{counts[k.id]}</small></button>)}
        </div>
        <button className="icon-button tip-end" data-tip="Atualizar" aria-label="Atualizar a lista" disabled={loading} onClick={load}><RefreshIcon size={17} /></button>
      </header>
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
    <main className="iris-grid-area">
      {items === null ? <p className="iris-empty" role="status">Reunindo os arquivos…</p>
        : !visible.length ? <p className="iris-empty">{query || source !== 'all' ? 'Nada encontrado com esse filtro.' : kinds.find(k => k.id === kind)?.empty}</p>
        : <>
            {groups.map(group => <section key={group.label} className="iris-month" aria-label={group.label}>
              <h2>{group.label}</h2>
              <div className="iris-grid">
                {group.items.map(item => <button key={item.key} className={`iris-card ${item.kind}`} title={`${item.name}\n${sourceLabel[item.source]} · ${item.place}`} onClick={() => setOpen(item.key)}>
                  <span className="iris-thumb"><Thumb item={item} /></span>
                  <span className="iris-caption"><b>{item.name}</b><small>{sourceLabel[item.source]} · {item.place}</small></span>
                </button>)}
              </div>
            </section>)}
            {shown < visible.length && <div ref={more} className="iris-more" aria-hidden="true" />}
          </>}
    </main>
    </section>
    {current && <div className="iris-viewer" role="dialog" aria-modal="true" aria-label={current.name} onClick={() => setOpen(null)}>
      <header className="iris-viewer-bar" onClick={e => e.stopPropagation()}>
        <span className="iris-viewer-title"><b>{current.name}</b><small>{sourceLabel[current.source]} · {current.place} · {day(current.date)} · {size(current.bytes)}</small></span>
        <span className="iris-viewer-count">{index + 1} de {visible.length}</span>
        <button className="icon-button" data-tip="Abrir no visualizador do sistema" aria-label="Abrir no visualizador do sistema" onClick={() => system(current)}><ExpandIcon size={17} /></button>
        <button className="icon-button tip-end" data-tip="Fechar (Esc)" aria-label="Fechar" onClick={() => setOpen(null)}><CloseIcon size={18} /></button>
      </header>
      <div className="iris-stage" onClick={e => e.stopPropagation()}>
        <MediaView key={current.key} media={mediaOf(current)} name={current.name} kind={current.kind} autoPlay onError={setError} />
      </div>
      {visible.length > 1 && <>
        <button className="iris-nav prev" aria-label="Anterior" onClick={e => { e.stopPropagation(); step(-1); }}><ChevronLeftIcon size={26} /></button>
        <button className="iris-nav next" aria-label="Próximo" onClick={e => { e.stopPropagation(); step(1); }}><ChevronRightIcon size={26} /></button>
      </>}
    </div>}
  </div>;
}
