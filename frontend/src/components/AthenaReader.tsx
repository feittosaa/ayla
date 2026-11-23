import { useCallback, useEffect, useRef, useState } from 'react';
import { FullscreenIcon } from './Icons';
import { errorText } from '../services/Desktop';
import { bookUrl, natural, type Book, type Progress, type ReadingMode } from '../services/Library';
import { cleanSvg, withPolicy } from '../services/BookSafety';

// ---------- Segurança do EPUB (ver services/BookSafety.ts) ----------
type Detail = { type: string; data: unknown; isScript?: boolean; allow?: boolean };
type BookObject = { transformTarget?: EventTarget; toc?: { label: string; href: string }[]; dir?: string };
function harden(book: BookObject) {
  book.transformTarget?.addEventListener('load', event => { const detail = (event as CustomEvent<Detail>).detail; if (detail.isScript) detail.allow = false; });
  book.transformTarget?.addEventListener('data', event => {
    const detail = (event as CustomEvent<Detail>).detail;
    const type = String(detail.type);
    const change = /html/i.test(type) ? withPolicy : /svg/i.test(type) ? cleanSvg : null;
    if (change) detail.data = Promise.resolve(detail.data).then(data => typeof data === 'string' ? change(data) : data);
  });
}

type FoliateView = HTMLElement & {
  open: (book: unknown) => Promise<void>; init: (options: { lastLocation?: string; showTextStart?: boolean }) => Promise<void>;
  goLeft: () => Promise<void>; goRight: () => Promise<void>; next: () => Promise<void>; prev: () => Promise<void>; goTo: (target: string) => Promise<unknown>; close: () => void;
  renderer: HTMLElement & { setStyles?: (css: string) => void; next: () => Promise<void> };
  book: BookObject;
};

function readerColors() {
  const style = getComputedStyle(document.documentElement);
  const read = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  return { text: read('--ayla-text', '#eee'), accent: read('--ayla-accent', '#ff9f43'), line: read('--ayla-border', '#444'), scheme: style.colorScheme || 'dark' };
}

/** Letra padrão do EPUB (em %), um pouco maior que a do navegador, para leitura longa. */
const DEFAULT_FONT = 115;
/** Roda do mouse virando páginas: um gesto por vez (a roda manda vários eventos seguidos). */
function turnOnWheel(turn: (forward: boolean) => void) {
  let locked = 0; let sum = 0;
  return (event: WheelEvent) => {
    event.preventDefault();
    const now = Date.now();
    if (now < locked) return;
    sum += Math.abs(event.deltaY) > Math.abs(event.deltaX) ? event.deltaY : event.deltaX;
    if (Math.abs(sum) < 40) return;
    turn(sum > 0);
    sum = 0; locked = now + 350;
  };
}

/** Livro EPUB: páginas, índice, tamanho da letra e posição lembrada (CFI). */
function EpubReader({ url, start, onLocation, onError }: {
  url: string; start: string; onLocation: (cfi: string, fraction: number) => void; onError: (text: string) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<FoliateView | null>(null);
  const [toc, setToc] = useState<{ label: string; href: string }[]>([]);
  const [size, setSize] = useState(() => Number(localStorage.getItem('ayla-reader-font-size')) || DEFAULT_FONT);
  const [ready, setReady] = useState(false);
  const events = useRef({ onLocation, onError });
  useEffect(() => { events.current = { onLocation, onError }; });
  const style = useCallback((percent: number) => {
    const { text, accent, line, scheme } = readerColors();
    // Livros costumam fixar texto preto e alturas de linha em pontos: as cores passam a ser as da
    // Ayla (o texto segue o tema; os fundos ficam transparentes) e as linhas ganham espaço.
    view.current?.renderer.setStyles?.(`
      html { color-scheme: ${scheme}; font-size: ${percent}% !important; }
      html, body { color: ${text} !important; background: transparent !important; }
      body *:not(a):not(img):not(svg):not(image) { color: inherit !important; background-color: transparent !important; border-color: ${line} !important; }
      a, a * { color: ${accent} !important; }
      p, div, span, li, blockquote, dd, dt, td, th, section { line-height: 1.6 !important; }
      h1, h2, h3, h4, h5, h6 { line-height: 1.3 !important; }
      img, svg { max-width: 100%; height: auto; }`);
  }, []);
  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const { makeBook } = await import('../vendor/foliate-js/view.js');
        const book = await makeBook(url) as BookObject;
        if (!alive || !host.current) return;
        harden(book);
        const element = document.createElement('foliate-view') as FoliateView;
        element.className = 'epub-view';
        host.current.replaceChildren(element);
        view.current = element;
        await element.open(book);
        element.renderer.setAttribute('flow', 'paginated');
        element.renderer.setAttribute('max-inline-size', '760px');
        element.renderer.setAttribute('gap', '6%');
        style(Number(localStorage.getItem('ayla-reader-font-size')) || DEFAULT_FONT);
        // Cada capítulo é um documento próprio: as setas e a roda valem também com o foco no texto.
        element.addEventListener('load', event => {
          const doc = (event as CustomEvent<{ doc: Document }>).detail?.doc;
          doc?.addEventListener('keydown', keys.current);
          doc?.addEventListener('wheel', wheel.current, { passive: false });
        });
        element.addEventListener('relocate', event => {
          const detail = (event as CustomEvent<{ cfi: string; fraction: number }>).detail;
          if (detail?.cfi) events.current.onLocation(detail.cfi, detail.fraction ?? 0);
        });
        await element.init({ lastLocation: start || undefined, showTextStart: !start });
        if (alive) { setToc((book.toc ?? []).filter(item => item.href)); setReady(true); }
      } catch (e) { if (alive) events.current.onError(`Não foi possível abrir o livro: ${errorText(e)}`); }
    })();
    return () => { alive = false; view.current?.close?.(); view.current?.remove(); view.current = null; };
    // O livro é aberto uma vez; a posição inicial só vale na abertura.
  }, [url]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { localStorage.setItem('ayla-reader-font-size', String(size)); style(size); }, [size, style]);
  /** Setas (esquerda/direita seguem a direção do livro), Page Up/Down, espaço e a roda do mouse. */
  const keys = useRef((event: KeyboardEvent) => {
    if ((event.target as HTMLElement)?.closest?.('input, textarea, select')) return;
    const move = { ArrowLeft: 'left', ArrowRight: 'right', PageDown: 'next', ' ': 'next', ArrowDown: 'next', PageUp: 'prev', ArrowUp: 'prev' }[event.key];
    if (!move) return;
    event.preventDefault();
    const current = view.current;
    if (move === 'left') void current?.goLeft(); else if (move === 'right') void current?.goRight();
    else if (move === 'next') void current?.next(); else void current?.prev();
  });
  const wheel = useRef(turnOnWheel(forward => { if (forward) void view.current?.next(); else void view.current?.prev(); }));
  useEffect(() => {
    const body = host.current?.parentElement;
    const onKey = (event: KeyboardEvent) => keys.current(event);
    const onWheel = (event: WheelEvent) => wheel.current(event);
    body?.addEventListener('keydown', onKey);
    body?.addEventListener('wheel', onWheel, { passive: false });
    return () => { body?.removeEventListener('keydown', onKey); body?.removeEventListener('wheel', onWheel); };
  }, []);
  return <div className="reader-body epub" tabIndex={0}>
    <div className="reader-controls">
      {toc.length > 0 && <select aria-label="Índice" value="" onChange={e => { if (e.target.value) void view.current?.goTo(e.target.value); }}>
        <option value="">Índice</option>
        {toc.map(item => <option key={item.href} value={item.href}>{item.label}</option>)}
      </select>}
      <button type="button" className="icon-button" aria-label="Letra menor" title="Letra menor" onClick={() => setSize(s => Math.max(70, s - 10))}>A−</button>
      <button type="button" className="icon-button" aria-label="Letra maior" title="Letra maior" onClick={() => setSize(s => Math.min(200, s + 10))}>A+</button>
    </div>
    <button type="button" className="reader-side left" aria-label="Página anterior" title="Página anterior (←)" onClick={() => void view.current?.goLeft()}><span aria-hidden="true">‹</span></button>
    <div ref={host} className="epub-host" aria-busy={!ready} />
    <button type="button" className="reader-side right" aria-label="Próxima página" title="Próxima página (→)" onClick={() => void view.current?.goRight()}><span aria-hidden="true">›</span></button>
  </div>;
}

// ---------- Mangá (CBZ) ----------
const IMAGE = /\.(jpe?g|png|webp|gif|avif|bmp)$/i;
type ZipEntry = { filename: string; directory: boolean; getData: (writer: unknown) => Promise<Blob> };

/** Capítulo em CBZ: páginas da direita para a esquerda, da esquerda para a direita, duas a duas ou
 * em rolagem vertical. As imagens são lidas do zip na memória, sem cópia no disco. */
function ComicReader({ url, mode, start, onPage, onEnd, onError }: {
  url: string; mode: ReadingMode; start: number; onPage: (page: number, total: number) => void; onEnd: () => void; onError: (text: string) => void;
}) {
  const [entries, setEntries] = useState<ZipEntry[] | null>(null);
  const [pages, setPages] = useState<Record<number, string>>({});
  const [page, setPage] = useState(0);
  const loading = useRef(new Map<number, Promise<void>>());
  const urls = useRef<string[]>([]);
  const writer = useRef<((type: string) => unknown) | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const events = useRef({ onPage, onEnd, onError });
  useEffect(() => { events.current = { onPage, onEnd, onError }; });
  const total = entries?.length ?? 0;
  const step = mode === 'double' ? 2 : 1;

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const zip = await import('../vendor/foliate-js/vendor/zip.js');
        zip.configure({ useWebWorkers: false });
        const response = await fetch(url);
        if (!response.ok) throw new Error(`o arquivo não abriu (${response.status})`);
        const reader = new zip.ZipReader(new zip.BlobReader(await response.blob()));
        const list = (await reader.getEntries() as ZipEntry[])
          .filter(entry => !entry.directory && IMAGE.test(entry.filename) && !entry.filename.startsWith('__MACOSX'))
          .sort((a, b) => natural(a.filename, b.filename));
        if (!alive) return;
        if (!list.length) throw new Error('este CBZ não tem imagens');
        writer.current = type => new zip.BlobWriter(type);
        setEntries(list);
        setPage(Math.min(Math.max(0, start), list.length - 1));
      } catch (e) { if (alive) events.current.onError(`Não foi possível abrir o capítulo: ${errorText(e)}`); }
    })();
    const created = urls.current;
    const pending = loading.current;
    return () => { alive = false; created.forEach(u => URL.revokeObjectURL(u)); created.length = 0; pending.clear(); };
    // O capítulo é aberto uma vez; a página inicial só vale na abertura.
  }, [url]); // eslint-disable-line react-hooks/exhaustive-deps

  const load = useCallback((index: number) => {
    if (!entries || index < 0 || index >= entries.length || loading.current.has(index)) return;
    const entry = entries[index];
    const type = /\.png$/i.test(entry.filename) ? 'image/png' : /\.webp$/i.test(entry.filename) ? 'image/webp' : /\.gif$/i.test(entry.filename) ? 'image/gif' : 'image/jpeg';
    loading.current.set(index, entry.getData(writer.current?.(type)).then(blob => {
      const address = URL.createObjectURL(blob);
      urls.current.push(address);
      setPages(current => ({ ...current, [index]: address }));
    }, () => {}));
  }, [entries]);

  // Páginas à vista e as vizinhas; na rolagem vertical, o capítulo inteiro, em ordem.
  useEffect(() => {
    if (!entries) return;
    if (mode === 'vertical') { entries.forEach((_, i) => load(i)); return; }
    for (let i = page - 2; i <= page + 3; i++) load(i);
  }, [entries, page, mode, load]);
  useEffect(() => { if (total) events.current.onPage(page, total); }, [page, total]);

  const go = useCallback((delta: number) => {
    if (!total) return;
    const next = page + delta * step;
    if (next >= total) { events.current.onEnd(); return; }
    setPage(Math.max(0, Math.min(total - 1, next)));
  }, [page, step, total]);
  // Setas seguem a direção da leitura: da direita para a esquerda, a seta para a esquerda avança.
  const forwardKey = mode === 'rtl' ? 'ArrowLeft' : 'ArrowRight';
  const backKey = mode === 'rtl' ? 'ArrowRight' : 'ArrowLeft';
  useEffect(() => {
    if (mode === 'vertical') return;
    const keys = (event: KeyboardEvent) => {
      if ((event.target as HTMLElement)?.closest?.('input, textarea, select')) return;
      if ([forwardKey, ' ', 'PageDown', 'ArrowDown'].includes(event.key)) { event.preventDefault(); go(1); }
      if ([backKey, 'PageUp', 'ArrowUp'].includes(event.key)) { event.preventDefault(); go(-1); }
    };
    const element = scroller.current;
    const onWheel = turnOnWheel(forward => go(forward ? 1 : -1));
    element?.addEventListener('keydown', keys);
    element?.addEventListener('wheel', onWheel, { passive: false });
    return () => { element?.removeEventListener('keydown', keys); element?.removeEventListener('wheel', onWheel); };
  }, [go, mode, forwardKey, backKey]);

  // Rolagem vertical: página atual pela imagem no meio da tela; no fim, rolar mais segue para o próximo.
  useEffect(() => {
    const element = scroller.current;
    if (mode !== 'vertical' || !element || !total) return;
    const watch = new IntersectionObserver(found => {
      for (const item of found) if (item.isIntersecting) setPage(Number((item.target as HTMLElement).dataset.page));
    }, { root: element, rootMargin: '-50% 0px -50% 0px' });
    element.querySelectorAll('[data-page]').forEach(node => watch.observe(node));
    let pushes = 0;
    const wheel = (event: WheelEvent) => {
      const bottom = element.scrollTop + element.clientHeight >= element.scrollHeight - 4;
      pushes = bottom && event.deltaY > 0 ? pushes + 1 : 0;
      if (pushes >= 3) { pushes = 0; events.current.onEnd(); }
    };
    element.addEventListener('wheel', wheel, { passive: true });
    return () => { watch.disconnect(); element.removeEventListener('wheel', wheel); };
  }, [mode, total, pages]);
  // Ao entrar na rolagem vertical, começa na página em que estava.
  const placed = useRef(false);
  useEffect(() => {
    if (mode !== 'vertical' || placed.current || !pages[page]) return;
    placed.current = true;
    scroller.current?.querySelector(`[data-page="${page}"]`)?.scrollIntoView({ block: 'start' });
  }, [mode, page, pages]);
  useEffect(() => { if (mode !== 'vertical') placed.current = false; }, [mode]);

  if (!entries) return <div className="reader-body comic" aria-busy="true"><p className="reader-wait">Abrindo o capítulo…</p></div>;
  if (mode === 'vertical') {
    return <div ref={scroller} className="reader-body comic vertical" tabIndex={0}>
      {entries.map((entry, i) => <div key={entry.filename} data-page={i} className="comic-strip">
        {pages[i] ? <img src={pages[i]} alt={`Página ${i + 1}`} /> : <div className="comic-placeholder" />}
      </div>)}
      <button type="button" className="comic-end" onClick={() => events.current.onEnd()}>Fim do capítulo · continuar</button>
    </div>;
  }
  const shown = mode === 'double' ? [page, page + 1].filter(i => i < total) : [page];
  return <div ref={scroller} className={`reader-body comic paged ${mode}`} tabIndex={0}
    onClick={event => {
      const rect = event.currentTarget.getBoundingClientRect();
      const left = event.clientX < rect.left + rect.width / 2;
      go(left === (mode === 'rtl') ? 1 : -1);
    }}>
    <div className="comic-spread">
      {shown.map(i => pages[i] ? <img key={i} src={pages[i]} alt={`Página ${i + 1}`} draggable={false} /> : <div key={i} className="comic-placeholder" />)}
    </div>
  </div>;
}

const modes: { mode: ReadingMode; label: string; title: string }[] = [
  { mode: 'rtl', label: '⇠', title: 'Da direita para a esquerda (mangá)' },
  { mode: 'ltr', label: '⇢', title: 'Da esquerda para a direita' },
  { mode: 'double', label: '▯▯', title: 'Página dupla' },
  { mode: 'vertical', label: '⇣', title: 'Rolagem vertical (webtoon)' },
];

/** Leitor de um livro da Biblioteca: EPUB ou CBZ, com progresso salvo, modos por série e o
 * próximo capítulo da série no fim. */
export default function AthenaReader({ source, book, series, progress, mode, onProgress, onMode, onOpen }: {
  source: string; book: Book; series: Book[]; progress?: Progress; mode: ReadingMode;
  onProgress: (position: string, fraction: number, done: boolean) => void; onMode: (mode: ReadingMode) => void; onOpen: (path: string) => void;
}) {
  const [error, setError] = useState('');
  const [where, setWhere] = useState({ label: '', fraction: progress?.fraction ?? 0 });
  const [finished, setFinished] = useState(false);
  const index = series.findIndex(b => b.path === book.path);
  const previous = index > 0 ? series[index - 1] : undefined;
  const next = index >= 0 ? series[index + 1] : undefined;
  // Grava o progresso pouco depois da última mudança e ao fechar o livro.
  const pending = useRef<{ position: string; fraction: number; done: boolean } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const save = useRef(onProgress);
  useEffect(() => { save.current = onProgress; });
  const flush = useCallback(() => {
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    if (pending.current) { const p = pending.current; pending.current = null; save.current(p.position, p.fraction, p.done); }
  }, []);
  const remember = useCallback((position: string, fraction: number, done: boolean) => {
    pending.current = { position, fraction, done };
    if (done) { flush(); return; }
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(flush, 800);
  }, [flush]);
  useEffect(() => flush, [flush]);
  const url = bookUrl(source, book.path);
  // Tela cheia do leitor (Esc sai, como no vídeo).
  const frame = useRef<HTMLDivElement>(null);
  const [full, setFull] = useState(false);
  useEffect(() => {
    const change = () => setFull(document.fullscreenElement === frame.current && Boolean(frame.current));
    document.addEventListener('fullscreenchange', change);
    return () => document.removeEventListener('fullscreenchange', change);
  }, []);
  const fullscreen = () => {
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined);
    else void frame.current?.requestFullscreen().catch(() => undefined);
  };
  const end = () => {
    remember(String(Math.max(0, Number(progress?.position) || 0)), 1, true);
    if (next) onOpen(next.path); else setFinished(true);
  };

  return <div ref={frame} className={full ? 'note-scroll reader full' : 'note-scroll reader'}>
    <header className="reader-bar">
      <div className="reader-title"><b>{book.title}</b>{book.series && <small>{book.series}</small>}</div>
      <span className="reader-where" aria-live="polite">{where.label || `${Math.round(where.fraction * 100)}%`}</span>
      {book.kind === 'cbz' && <div className="reader-modes" role="group" aria-label="Modo de leitura">
        {modes.map(m => <button key={m.mode} type="button" className={m.mode === mode ? 'icon-button active' : 'icon-button'} aria-pressed={m.mode === mode} title={m.title} aria-label={m.title} onClick={() => onMode(m.mode)}>{m.label}</button>)}
      </div>}
      <button type="button" className="icon-button" disabled={!previous} title={previous ? `Anterior: ${previous.title}` : 'Primeiro da série'} aria-label="Capítulo anterior" onClick={() => previous && onOpen(previous.path)}>⏮</button>
      <button type="button" className="icon-button" disabled={!next} title={next ? `Próximo: ${next.title}` : 'Último da série'} aria-label="Próximo capítulo" onClick={() => next && onOpen(next.path)}>⏭</button>
      <button type="button" className={progress?.done ? 'icon-button active' : 'icon-button'} title={progress?.done ? 'Lido' : 'Marcar como lido'} aria-label="Marcar como lido" onClick={() => remember(progress?.position ?? '0', 1, true)}>✓</button>
      <button type="button" className={full ? 'icon-button active' : 'icon-button'} title={full ? 'Sair da tela cheia (Esc)' : 'Tela cheia'} aria-label={full ? 'Sair da tela cheia' : 'Tela cheia'} aria-pressed={full} onClick={fullscreen}><FullscreenIcon size={16} /></button>
    </header>
    <div className="reader-progress" aria-hidden="true"><span style={{ width: `${Math.round(where.fraction * 100)}%` }} /></div>
    {error ? <p role="alert" className="app-error reader-error">{error}</p>
      : book.kind === 'epub'
      ? <EpubReader key={url} url={url} start={progress?.position ?? ''} onError={setError}
          onLocation={(cfi, fraction) => { setWhere({ label: '', fraction }); remember(cfi, fraction, fraction >= 0.995); }} />
      : <ComicReader key={url} url={url} mode={mode} start={Number(progress?.done ? 0 : progress?.position) || 0} onError={setError} onEnd={end}
          onPage={(page, total) => { setWhere({ label: `${page + 1} / ${total}`, fraction: (page + 1) / total }); remember(String(page), (page + 1) / total, page >= total - 1); }} />}
    {finished && <div className="reader-finished" role="status">Fim da série. <button type="button" className="link-button" onClick={() => setFinished(false)}>Fechar</button></div>}
  </div>;
}
