import { invoke } from '@tauri-apps/api/core';
import { StaticCanvas } from 'fabric';
import { native } from './Desktop';
import { pdfjs } from './PdfJs';
import { withSoftwareCanvases } from './DesignCanvas';

/** Arquivo da Athena/Iris aberto nos designs. `base`: cópia intacta guardada pela Ayla; `gravado`:
 * hash do arquivo como o editor o viu por último (só grava se ainda for esse). */
export type Documento = { tipo: 'pdf' | 'imagem'; nome: string; file?: string | null; root?: string | null; path?: string | null; base: string; gravado: string };
/** Uma página do documento no design: tamanho lógico e os elementos adicionados (JSON do Fabric). */
export type PaginaDoc = { largura: number; altura: number; objetos: string | null };
export type DadosDoc = { documento: Documento; paginas: PaginaDoc[] };

export const refOf = (d: Pick<Documento, 'file' | 'root' | 'path'>) => ({ file: d.file ?? null, root: d.file ? null : d.root ?? null, path: d.file ? null : d.path ?? null });

export function readDoc(data: string | undefined): DadosDoc | null {
  if (!data) return null;
  try { const v = JSON.parse(data); return v && v.documento && Array.isArray(v.paginas) ? v as DadosDoc : null; } catch { return null; }
}

/** PDF: cada ponto vira 2 px no editor; a camada é gravada a 3 px por ponto (216 dpi). */
export const PDF_PX = 2;
export const PDF_LAYER = 1.5;
/** Imagens: o editor trabalha com até 4000 px no lado maior (a gravação volta ao tamanho real). */
const IMAGE_SIDE = 4000;
const ASSETS = `${import.meta.env.BASE_URL}pdfjs/`;

export type Aberto = {
  paginas: { largura: number; altura: number }[];
  /** Fundo da página `i` (URL de objeto), desenhado a partir da base. */
  fundo: (i: number) => Promise<string>;
  /** Imagem: tamanho real e o elemento (para compor ao gravar). */
  imagem?: { el: HTMLImageElement; largura: number; altura: number; tipo: string };
  fechar: () => void;
};

async function baseBytes(base: string) { return new Uint8Array(await invoke<ArrayBuffer>('design_documento_base', { base })); }

export async function abrir(doc: Documento): Promise<Aberto> {
  const bytes = await baseBytes(doc.base);
  if (doc.tipo === 'imagem') {
    const ext = doc.base.split('.').pop()!.toLowerCase();
    const tipo = ext === 'png' ? 'image/png' : ext === 'webp' ? 'image/webp' : 'image/jpeg';
    const url = URL.createObjectURL(new Blob([bytes], { type: tipo }));
    const el = new Image();
    el.src = url;
    await el.decode();
    const k = Math.min(1, IMAGE_SIDE / Math.max(el.naturalWidth, el.naturalHeight));
    return {
      paginas: [{ largura: Math.round(el.naturalWidth * k), altura: Math.round(el.naturalHeight * k) }],
      fundo: async () => url, imagem: { el, largura: el.naturalWidth, altura: el.naturalHeight, tipo }, fechar: () => URL.revokeObjectURL(url),
    };
  }
  const { pdf, worker } = await pdfjs();
  const task = pdf.getDocument({ data: bytes, cMapUrl: `${ASSETS}cmaps/`, cMapPacked: true, standardFontDataUrl: `${ASSETS}standard_fonts/`, wasmUrl: `${ASSETS}wasm/`, worker });
  const proxy = await task.promise;
  const paginas: { largura: number; altura: number }[] = [];
  for (let i = 1; i <= proxy.numPages; i++) {
    const vp = (await proxy.getPage(i)).getViewport({ scale: PDF_PX });
    paginas.push({ largura: Math.round(vp.width), altura: Math.round(vp.height) });
  }
  const feitos = new Map<number, Promise<string>>();
  const urls: string[] = [];
  const fundo = (i: number) => {
    let p = feitos.get(i);
    if (!p) {
      p = (async () => {
        const page = await proxy.getPage(i + 1);
        const vp = page.getViewport({ scale: PDF_PX });
        const canvas = document.createElement('canvas');
        canvas.width = Math.round(vp.width); canvas.height = Math.round(vp.height);
        const ctx = canvas.getContext('2d')!;
        ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, canvas.width, canvas.height);
        await page.render({ canvasContext: ctx, canvas, viewport: vp }).promise;
        const blob = await new Promise<Blob>((ok, fail) => canvas.toBlob(b => b ? ok(b) : fail(new Error('página')), 'image/jpeg', 0.92));
        const url = URL.createObjectURL(blob); urls.push(url);
        return url;
      })();
      feitos.set(i, p);
    }
    return p;
  };
  return { paginas, fundo, fechar: () => { urls.forEach(u => URL.revokeObjectURL(u)); void task.destroy(); } };
}

/** Camada transparente (PNG em base64) de uma página: só os elementos, no tamanho de gravação.
 * `null` se a página não tem nada. */
export async function camada(pagina: PaginaDoc, multiplicador: number): Promise<string | null> {
  if (!pagina.objetos) return null;
  const parsed = JSON.parse(pagina.objetos);
  if (!parsed.objects?.length) return null;
  const el = document.createElement('canvas');
  const canvas = withSoftwareCanvases(() => new StaticCanvas(el, { width: pagina.largura, height: pagina.altura, renderOnAddRemove: false, enableRetinaScaling: false }));
  try {
    await canvas.loadFromJSON({ ...parsed, background: '', backgroundImage: undefined });
    canvas.backgroundColor = '';
    canvas.renderAll();
    return canvas.toDataURL({ format: 'png', multiplier: multiplicador, enableRetinaScaling: false }).split(',')[1];
  } finally { void canvas.dispose(); }
}

/** Imagem final (original + camada) no formato do arquivo, em base64. */
export async function compor(img: NonNullable<Aberto['imagem']>, pagina: PaginaDoc): Promise<string> {
  const el = document.createElement('canvas');
  el.width = img.largura; el.height = img.altura;
  const ctx = el.getContext('2d')!;
  ctx.drawImage(img.el, 0, 0);
  const layer = await camada(pagina, img.largura / pagina.largura);
  if (layer) {
    const over = new Image();
    over.src = `data:image/png;base64,${layer}`;
    await over.decode();
    ctx.drawImage(over, 0, 0, img.largura, img.altura);
  }
  const blob = await new Promise<Blob>((ok, fail) => el.toBlob(b => b ? ok(b) : fail(new Error('imagem')), img.tipo, 0.95));
  const buffer = new Uint8Array(await blob.arrayBuffer());
  let binary = '';
  for (let i = 0; i < buffer.length; i += 0x8000) binary += String.fromCharCode(...buffer.subarray(i, i + 0x8000));
  return btoa(binary);
}

/** Prévia para a lista de designs: a 1ª página com os elementos por cima (PNG base64). */
export async function previa(fundoUrl: string, pagina: PaginaDoc, lado = 360): Promise<string> {
  const k = lado / Math.max(pagina.largura, pagina.altura);
  const el = document.createElement('canvas');
  el.width = Math.max(1, Math.round(pagina.largura * k)); el.height = Math.max(1, Math.round(pagina.altura * k));
  const ctx = el.getContext('2d')!;
  const bg = new Image(); bg.src = fundoUrl; await bg.decode();
  ctx.drawImage(bg, 0, 0, el.width, el.height);
  const layer = await camada(pagina, k);
  if (layer) { const over = new Image(); over.src = `data:image/png;base64,${layer}`; await over.decode(); ctx.drawImage(over, 0, 0, el.width, el.height); }
  return el.toDataURL('image/png').split(',')[1];
}

/** Abre um PDF/imagem da biblioteca: guarda a base e devolve o documento. */
export async function documentoDe(item: { file?: string | null; root?: string | null; path?: string | null }): Promise<Documento> {
  const ref = refOf(item);
  const r = await native<{ tipo: 'pdf' | 'imagem'; nome: string; base: string; atual: string }>('design_documento', ref);
  return { tipo: r.tipo, nome: r.nome, ...ref, base: r.base, gravado: r.atual };
}
