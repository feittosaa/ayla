import { useEffect, useRef, useState, type ReactNode } from 'react';
import 'pdfjs-dist/web/pdf_viewer.css';
import type { PDFDocumentProxy } from 'pdfjs-dist';
import type { EventBus, PDFViewer } from 'pdfjs-dist/web/pdf_viewer.mjs';
import { pdfjs } from '../services/PdfJs';
import { ColumnsIcon, CursorTextIcon, FitPageIcon, FitWidthIcon, HandIcon } from './Icons';

/** Trecho do PDF lido por vez: o leitor pede ao núcleo só as partes que vai desenhar. */
const CHUNK = 1024 * 1024;

/** Lê `bytes=início-fim` (fim exclusivo) pelo endereço da Ayla. */
async function part(src: string, begin: number, end: number, signal: AbortSignal) {
  const response = await fetch(src, { headers: { Range: `bytes=${begin}-${end - 1}` }, signal });
  if (!response.ok) throw new Error('indisponível');
  return { total: Number(response.headers.get('Content-Range')?.split('/')[1] ?? 0), bytes: new Uint8Array(await response.arrayBuffer()) };
}
const SPREAD = 'ayla-pdf-dupla';
const TOOL = 'ayla-pdf-ferramenta';
const ASSETS = `${import.meta.env.BASE_URL}pdfjs/`;

type Fit = 'page-width' | 'page-fit' | null;

/** Leitor de PDF da Ayla (pdf.js): fundo da Ayla, arrastar com o mouse (mão) ou selecionar texto,
 * Ctrl + roda para zoom no ponto do mouse, página simples ou dupla ajustada à largura ou à
 * página inteira (reajusta quando a tela muda de tamanho). `actions` entra no fim da barra. */
export default function PdfView({ src, actions, onSystem }: { src: string; actions?: ReactNode; onSystem?: () => void }) {
  const box = useRef<HTMLDivElement | null>(null); const sheet = useRef<HTMLDivElement | null>(null);
  const viewer = useRef<PDFViewer | null>(null);
  const [state, setState] = useState<{ src: string; ready?: boolean; error?: boolean }>({ src });
  if (state.src !== src) setState({ src });
  const [page, setPage] = useState(1); const [pages, setPages] = useState(0); const [scale, setScale] = useState(1);
  const [fit, setFit] = useState<Fit>('page-width');
  const [spread, setSpread] = useState(() => localStorage.getItem(SPREAD) === '1');
  const [hand, setHand] = useState(() => localStorage.getItem(TOOL) !== 'texto');
  const fitNow = useRef<Fit>('page-width');

  useEffect(() => {
    const container = box.current; const inner = sheet.current;
    if (!container || !inner) return;
    const abort = new AbortController();
    let task: { destroy: () => Promise<void> } | null = null; let bus: EventBus | null = null;
    void (async () => {
      try {
        // Começo do arquivo (com o tamanho total) junto com o leitor; o resto vem por trechos,
        // conforme as páginas aparecem, sem esperar o PDF inteiro.
        const [{ pdf, viewer: ui, worker }, first] = await Promise.all([pdfjs(), part(src, 0, CHUNK, abort.signal)]);
        if (!first.total) throw new Error('indisponível');
        if (abort.signal.aborted) return;
        bus = new ui.EventBus();
        const links = new ui.PDFLinkService({ eventBus: bus });
        const view = new ui.PDFViewer({ container, viewer: inner, eventBus: bus, linkService: links, removePageBorders: true });
        links.setViewer(view);
        bus.on('pagesinit', () => { view.spreadMode = localStorage.getItem(SPREAD) === '1' ? 1 : 0; view.currentScaleValue = fitNow.current ?? 'page-width'; });
        bus.on('pagechanging', (e: { pageNumber: number }) => setPage(e.pageNumber));
        bus.on('scalechanging', (e: { scale: number; presetValue?: string }) => setScale(e.scale));
        const range = new pdf.PDFDataRangeTransport(first.total, first.bytes);
        range.requestDataRange = (begin: number, end: number) => {
          part(src, begin, end, abort.signal).then(({ bytes }) => range.onDataRange(begin, bytes), () => { if (!abort.signal.aborted) setState({ src, error: true }); });
        };
        const loadingTask = pdf.getDocument({ range, rangeChunkSize: CHUNK, disableAutoFetch: true, disableStream: true, cMapUrl: `${ASSETS}cmaps/`, cMapPacked: true, standardFontDataUrl: `${ASSETS}standard_fonts/`, wasmUrl: `${ASSETS}wasm/`, worker });
        task = loadingTask;
        const doc = await loadingTask.promise;
        if (abort.signal.aborted) return;
        view.setDocument(doc); links.setDocument(doc);
        viewer.current = view; setPages(doc.numPages); setPage(1);
        setState({ src, ready: true });
      } catch {
        if (!abort.signal.aborted) setState({ src, error: true });
      }
    })();
    // Tela mudou de tamanho (tela dupla, barra lateral, janela): reajusta se estiver "ajustado".
    const resize = new ResizeObserver(() => { const view = viewer.current; if (view && fitNow.current) view.currentScaleValue = fitNow.current; });
    resize.observe(container);
    return () => { abort.abort(); resize.disconnect(); viewer.current?.setDocument(null as unknown as PDFDocumentProxy); viewer.current = null; void task?.destroy(); };
  }, [src]);

  // Ctrl + roda: zoom ancorado no mouse (o navegador não pode tratar a roda: ouvinte não passivo).
  useEffect(() => {
    const container = box.current;
    if (!container) return;
    const wheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey) || !viewer.current) return;
      e.preventDefault();
      fitNow.current = null; setFit(null);
      viewer.current.updateScale({ scaleFactor: Math.exp(-e.deltaY / 400), origin: [e.clientX, e.clientY] });
    };
    container.addEventListener('wheel', wheel, { passive: false });
    return () => container.removeEventListener('wheel', wheel);
  }, []);

  // Mão: arrastar move o PDF (como no leitor do Obsidian). Links continuam clicáveis.
  const drag = useRef<{ x: number; y: number; left: number; top: number; id: number } | null>(null);
  function down(e: React.PointerEvent<HTMLDivElement>) {
    if (!hand || e.button !== 0 || (e.target as HTMLElement).closest('a, input, button')) return;
    const c = e.currentTarget;
    drag.current = { x: e.clientX, y: e.clientY, left: c.scrollLeft, top: c.scrollTop, id: e.pointerId };
    c.setPointerCapture(e.pointerId); c.classList.add('grabbing'); e.preventDefault();
  }
  function move(e: React.PointerEvent<HTMLDivElement>) {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    e.currentTarget.scrollLeft = d.left - (e.clientX - d.x); e.currentTarget.scrollTop = d.top - (e.clientY - d.y);
  }
  function up(e: React.PointerEvent<HTMLDivElement>) { if (drag.current?.id === e.pointerId) { drag.current = null; e.currentTarget.classList.remove('grabbing'); } }

  function choose(next: Fit) { fitNow.current = next; setFit(next); if (viewer.current && next) viewer.current.currentScaleValue = next; }
  function zoom(steps: number) { fitNow.current = null; setFit(null); viewer.current?.updateScale({ steps }); }
  function toggleSpread() {
    const next = !spread; setSpread(next); localStorage.setItem(SPREAD, next ? '1' : '0');
    if (viewer.current) { viewer.current.spreadMode = next ? 1 : 0; if (fitNow.current) viewer.current.currentScaleValue = fitNow.current; }
  }
  function toggleHand() { const next = !hand; setHand(next); localStorage.setItem(TOOL, next ? 'mao' : 'texto'); }

  return <div className="pdf-view">
    <div className="pdf-toolbar" role="toolbar" aria-label="Leitor de PDF">
      <span className="pdf-pages" aria-live="polite">{pages ? `${page} / ${pages}` : '…'}</span>
      <span className="pdf-sep" />
      <button className="icon-button" aria-label="Diminuir o zoom" data-tip="Diminuir" onClick={() => zoom(-1)}>−</button>
      <span className="pdf-zoom">{Math.round(scale * 100)}%</span>
      <button className="icon-button" aria-label="Aumentar o zoom" data-tip="Aumentar (Ctrl + roda)" onClick={() => zoom(1)}>+</button>
      <button className={fit === 'page-width' ? 'icon-button active' : 'icon-button'} aria-pressed={fit === 'page-width'} aria-label="Ajustar à largura" data-tip="Ajustar à largura" onClick={() => choose('page-width')}><FitWidthIcon size={17} /></button>
      <button className={fit === 'page-fit' ? 'icon-button active' : 'icon-button'} aria-pressed={fit === 'page-fit'} aria-label="Página inteira" data-tip="Página inteira" onClick={() => choose('page-fit')}><FitPageIcon size={17} /></button>
      <span className="pdf-sep" />
      <button className={spread ? 'icon-button active' : 'icon-button'} aria-pressed={spread} aria-label="Página dupla" data-tip={spread ? 'Página simples' : 'Página dupla'} onClick={toggleSpread}><ColumnsIcon size={17} /></button>
      <button className="icon-button" aria-label={hand ? 'Selecionar texto' : 'Arrastar com a mão'} data-tip={hand ? 'Selecionar texto' : 'Arrastar com a mão'} onClick={toggleHand}>{hand ? <CursorTextIcon size={17} /> : <HandIcon size={17} />}</button>
      {actions && <><span className="spacer" />{actions}</>}
    </div>
    <div className="pdf-stage">
      <div ref={box} className={hand ? 'pdf-scroll hand' : 'pdf-scroll'} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
        <div ref={sheet} className="pdfViewer" />
      </div>
      {!state.ready && !state.error && <p className="pdf-status" role="status">Abrindo o PDF…</p>}
      {state.error && <p className="pdf-status">
        Não foi possível abrir este PDF aqui.
        {onSystem && <button className="link-button" onClick={onSystem}>Abrir no visualizador do sistema</button>}
      </p>}
    </div>
  </div>;
}
