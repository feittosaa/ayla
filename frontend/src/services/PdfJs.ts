import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url';

type Lib = { pdf: typeof import('pdfjs-dist'); viewer: typeof import('pdfjs-dist/web/pdf_viewer.mjs'); worker: InstanceType<typeof import('pdfjs-dist').PDFWorker> };
let loading: Promise<Lib> | null = null;
/** pdf.js carregado uma vez, com um único worker reaproveitado por todos os PDFs (abrir o
 * segundo PDF não recria o worker). O leitor (pdf_viewer) lê a biblioteca de `globalThis.pdfjsLib`. */
export function pdfjs(): Promise<Lib> {
  loading ??= (async () => {
    const pdf = await import('pdfjs-dist');
    pdf.GlobalWorkerOptions.workerSrc = workerUrl;
    (globalThis as unknown as { pdfjsLib: typeof pdf }).pdfjsLib = pdf;
    const viewer = await import('pdfjs-dist/web/pdf_viewer.mjs');
    return { pdf, viewer, worker: new pdf.PDFWorker() };
  })();
  return loading;
}
/** Adianta o carregamento do leitor (Athena e Iris chamam ao abrir), para o 1º PDF abrir rápido. */
export function preloadPdf() {
  const start = () => void pdfjs().catch(() => { loading = null; });
  if ('requestIdleCallback' in window) window.requestIdleCallback(start, { timeout: 3000 }); else setTimeout(start, 1500);
}

