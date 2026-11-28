/** Designs da Iris (estilo Canva): formatos, modelos e regras puras do editor. O editor em si
 * (Fabric.js) fica em `IrisDesignEditor`; aqui ficam só dados e contas testáveis. */

export type DesignSummary = { id: string; name: string; width: number; height: number; created: number; modified: number; preview: string | null };
export type DesignFile = DesignSummary & { data: string };
export type PackFont = { family: string; faces: { weight: number; style: string; files: string[] }[] };

export type Format = { id: string; label: string; width: number; height: number; hint: string };
export const formats: Format[] = [
  { id: 'apresentacao', label: 'Apresentação', width: 1920, height: 1080, hint: 'Slides 16:9, com as páginas embaixo' },
  { id: 'post', label: 'Post quadrado', width: 1080, height: 1080, hint: 'Instagram, Facebook' },
  { id: 'retrato', label: 'Post retrato', width: 1080, height: 1350, hint: 'Instagram 4:5' },
  { id: 'story', label: 'Story', width: 1080, height: 1920, hint: 'Stories, Reels, status' },
  { id: 'wallpaper', label: 'Wallpaper', width: 1920, height: 1080, hint: 'Área de trabalho Full HD' },
  { id: 'capa', label: 'Capa de vídeo', width: 1280, height: 720, hint: 'Miniatura de vídeo 16:9' },
  { id: 'a4', label: 'A4', width: 794, height: 1123, hint: 'Folha para imprimir (PDF)' },
];
export const MIN_SIDE = 16;
export const MAX_SIDE = 8000;
/** Unidades do tamanho livre. O design é medido em px de tela (96 por polegada), como o PDF
 * exportado: 210 × 297 mm viram 794 × 1123 px, a mesma folha A4 do formato pronto. */
export type Unit = 'px' | 'cm' | 'mm';
const PX_PER: Record<Unit, number> = { px: 1, cm: 96 / 2.54, mm: 96 / 25.4 };
export const toPx = (value: number, unit: Unit) => value * PX_PER[unit];
/** Valor na unidade, com até 1 casa (cm/mm) ou inteiro (px). */
export const fromPx = (px: number, unit: Unit) => unit === 'px' ? Math.round(px) : Math.round((px / PX_PER[unit]) * 10) / 10;
export const clampSide = (value: number) => Math.round(Math.min(MAX_SIDE, Math.max(MIN_SIDE, Number.isFinite(value) ? value : MIN_SIDE)));

/** Página do PDF em pontos: o design é medido em px de tela (96 por polegada), então A4
 * (794 × 1123 px) vira a folha A4 de verdade (595 × 842 pt). */
export const pdfPage = (width: number, height: number): [number, number] => [Math.round(width * 0.75 * 100) / 100, Math.round(height * 0.75 * 100) / 100];

/** Escala da exportação: PDF em ~300 dpi; imagens com pelo menos 1920 px no lado maior. */
export function exportScale(width: number, height: number, format: 'png' | 'jpg' | 'pdf') {
  const side = Math.max(width, height);
  const wanted = format === 'pdf' ? 3 : Math.max(1, Math.ceil(1920 / side));
  return Math.max(1, Math.min(wanted, Math.floor(MAX_SIDE / side) || 1));
}

/** Prévia guardada: lado maior de até 400 px. */
export const previewScale = (width: number, height: number) => Math.min(1, 400 / Math.max(width, height));

export type Fill = string | { angle: number; colors: [string, string] };
/** Pontos de um gradiente linear (em px) que cobre um retângulo inteiro no ângulo pedido. */
export function gradientCoords(angle: number, width: number, height: number) {
  const rad = (angle * Math.PI) / 180;
  const cos = Math.cos(rad); const sin = Math.sin(rad);
  const half = (Math.abs(width * cos) + Math.abs(height * sin)) / 2;
  const cx = width / 2; const cy = height / 2;
  const round = (n: number) => Math.round(n * 100) / 100 || 0;
  return { x1: round(cx - cos * half), y1: round(cy - sin * half), x2: round(cx + cos * half), y2: round(cy + sin * half) };
}

/** Desfazer/refazer com fotos do projeto (JSON). Fotos iguais seguidas não contam. */
export class History {
  private past: string[] = [];
  private future: string[] = [];
  private current: string;
  private limit: number;
  constructor(current: string, limit = 60) { this.current = current; this.limit = limit; }
  get canUndo() { return this.past.length > 0; }
  get canRedo() { return this.future.length > 0; }
  get value() { return this.current; }
  push(next: string) {
    if (next === this.current) return false;
    this.past.push(this.current);
    if (this.past.length > this.limit) this.past.shift();
    this.current = next; this.future = [];
    return true;
  }
  undo() {
    const previous = this.past.pop();
    if (previous === undefined) return null;
    this.future.push(this.current); this.current = previous;
    return previous;
  }
  redo() {
    const next = this.future.pop();
    if (next === undefined) return null;
    this.past.push(this.current); this.current = next;
    return next;
  }
}

/** Família com reserva genérica (sem o pacote de fontes, o texto sai numa fonte do sistema). */
export const genericOf: Record<string, string> = {
  'Inter': 'sans-serif', 'Montserrat': 'sans-serif', 'Poppins': 'sans-serif', 'Oswald': 'sans-serif', 'Bebas Neue': 'sans-serif', 'Anton': 'sans-serif',
  'Playfair Display': 'serif', 'Lora': 'serif', 'Roboto Slab': 'serif', 'Lobster': 'cursive', 'Pacifico': 'cursive', 'Dancing Script': 'cursive',
};
export const systemFonts = ['sans-serif', 'serif', 'monospace'];
export const fontValue = (family: string) => genericOf[family] ? `${family}, ${genericOf[family]}` : family;
/** Nome mostrado no seletor para o valor guardado no texto. */
export const fontName = (value: string) => value.split(',')[0].trim().replace(/^['"]|['"]$/g, '');

export type Spec =
  | { type: 'text'; text: string; left: number; top: number; width: number; fontSize: number; font: string; bold?: boolean; italic?: boolean; fill: string; align?: 'left' | 'center' | 'right'; spacing?: number; lineHeight?: number; opacity?: number }
  | { type: 'rect'; left: number; top: number; width: number; height: number; fill: Fill; radius?: number; angle?: number; opacity?: number }
  | { type: 'circle'; left: number; top: number; radius: number; fill: Fill; opacity?: number; stroke?: string; strokeWidth?: number };
export type Template = { id: string; name: string; format: string; background: Fill; objects: Spec[] };

/** Modelos prontos (fontes do pacote opcional; sem ele, as reservas do sistema). */
export const templates: Template[] = [
  { id: 'oferta', name: 'Oferta', format: 'post', background: { angle: 135, colors: ['#ff7a59', '#ff3d77'] }, objects: [
    { type: 'circle', left: 760, top: -120, radius: 260, fill: '#ffffff', opacity: 0.12 },
    { type: 'circle', left: -140, top: 760, radius: 220, fill: '#ffffff', opacity: 0.1 },
    { type: 'text', text: 'SÓ HOJE', left: 90, top: 150, width: 900, fontSize: 64, font: 'Montserrat', bold: true, fill: '#ffe8de', spacing: 300 },
    { type: 'text', text: 'Mega\nOferta', left: 90, top: 240, width: 900, fontSize: 190, font: 'Bebas Neue', fill: '#ffffff', lineHeight: 0.9 },
    { type: 'circle', left: 720, top: 560, radius: 140, fill: '#ffffff' },
    { type: 'text', text: '-30%', left: 720, top: 640, width: 280, fontSize: 96, font: 'Anton', fill: '#ff3d77', align: 'center' },
    { type: 'rect', left: 90, top: 840, width: 420, height: 110, fill: '#1d1d2b', radius: 55 },
    { type: 'text', text: 'Compre agora', left: 90, top: 872, width: 420, fontSize: 44, font: 'Poppins', bold: true, fill: '#ffffff', align: 'center' },
  ] },
  { id: 'citacao', name: 'Citação', format: 'post', background: '#f4efe6', objects: [
    { type: 'text', text: '“', left: 90, top: 90, width: 300, fontSize: 320, font: 'Playfair Display', bold: true, fill: '#c8a97e' },
    { type: 'text', text: 'Faça o que puder, com o que tiver, onde estiver.', left: 140, top: 360, width: 800, fontSize: 76, font: 'Playfair Display', italic: true, fill: '#2b2622', lineHeight: 1.15 },
    { type: 'rect', left: 140, top: 760, width: 120, height: 6, fill: '#c8a97e' },
    { type: 'text', text: 'Theodore Roosevelt', left: 140, top: 800, width: 800, fontSize: 40, font: 'Lora', fill: '#6b6058' },
  ] },
  { id: 'evento', name: 'Evento', format: 'story', background: { angle: 160, colors: ['#1b1446', '#5b2a86'] }, objects: [
    { type: 'circle', left: 620, top: 180, radius: 320, fill: '#ff6ec7', opacity: 0.25 },
    { type: 'circle', left: -200, top: 1260, radius: 380, fill: '#4cc9f0', opacity: 0.22 },
    { type: 'text', text: 'VOCÊ ESTÁ CONVIDADO', left: 90, top: 380, width: 900, fontSize: 52, font: 'Montserrat', bold: true, fill: '#ffd6f0', spacing: 200 },
    { type: 'text', text: 'Noite de\nJogos', left: 90, top: 480, width: 900, fontSize: 200, font: 'Lobster', fill: '#ffffff', lineHeight: 0.95 },
    { type: 'rect', left: 90, top: 1010, width: 900, height: 4, fill: '#ffffff', opacity: 0.5 },
    { type: 'text', text: 'Sábado, 14 de dezembro · 19h', left: 90, top: 1060, width: 900, fontSize: 56, font: 'Poppins', fill: '#ffffff' },
    { type: 'text', text: 'Rua das Flores, 123', left: 90, top: 1140, width: 900, fontSize: 44, font: 'Poppins', fill: '#d9c8ff' },
    { type: 'rect', left: 90, top: 1560, width: 900, height: 130, fill: '#ffffff', radius: 65 },
    { type: 'text', text: 'Confirme presença', left: 90, top: 1598, width: 900, fontSize: 50, font: 'Poppins', bold: true, fill: '#3a1d6e', align: 'center' },
  ] },
  { id: 'minimalista', name: 'Wallpaper minimalista', format: 'wallpaper', background: { angle: 120, colors: ['#0f2027', '#2c5364'] }, objects: [
    { type: 'circle', left: 1240, top: 220, radius: 300, fill: { angle: 90, colors: ['#f7b267', '#f25f5c'] } },
    { type: 'rect', left: 0, top: 760, width: 1920, height: 320, fill: '#0b161b', opacity: 0.55 },
    { type: 'text', text: 'foco.', left: 140, top: 420, width: 1000, fontSize: 220, font: 'Inter', bold: true, fill: '#e8f1f2' },
    { type: 'text', text: 'um passo de cada vez', left: 150, top: 670, width: 1000, fontSize: 48, font: 'Inter', fill: '#a9c4cb', spacing: 100 },
  ] },
  { id: 'cartaz', name: 'Cartaz A4', format: 'a4', background: '#ffffff', objects: [
    { type: 'rect', left: 0, top: 0, width: 794, height: 330, fill: { angle: 90, colors: ['#2a9d8f', '#264653'] } },
    { type: 'text', text: 'Feira de\nTrocas', left: 60, top: 70, width: 680, fontSize: 92, font: 'Oswald', bold: true, fill: '#ffffff', lineHeight: 0.95 },
    { type: 'text', text: 'Traga o que não usa e leve algo novo', left: 60, top: 380, width: 680, fontSize: 34, font: 'Poppins', bold: true, fill: '#264653', lineHeight: 1.1 },
    { type: 'text', text: 'Livros, roupas, jogos, plantas e o que mais você quiser trocar. Entrada gratuita para toda a família.', left: 60, top: 520, width: 640, fontSize: 24, font: 'Lora', fill: '#3d3d3d', lineHeight: 1.35 },
    { type: 'rect', left: 60, top: 760, width: 674, height: 2, fill: '#2a9d8f' },
    { type: 'text', text: 'Domingo · 9h às 14h\nPraça Central', left: 60, top: 790, width: 674, fontSize: 32, font: 'Poppins', fill: '#264653', lineHeight: 1.3 },
    { type: 'rect', left: 0, top: 1043, width: 794, height: 80, fill: '#e9c46a' },
    { type: 'text', text: 'Organização: Associação do Bairro', left: 60, top: 1070, width: 674, fontSize: 22, font: 'Poppins', fill: '#264653' },
  ] },
  { id: 'capa', name: 'Capa de vídeo', format: 'capa', background: { angle: 45, colors: ['#ffd60a', '#ffc300'] }, objects: [
    { type: 'rect', left: 640, top: -60, width: 760, height: 860, fill: '#111111', angle: 12 },
    { type: 'text', text: 'COMO FAZER EM', left: 60, top: 160, width: 640, fontSize: 64, font: 'Anton', fill: '#111111' },
    { type: 'text', text: '5 MIN', left: 60, top: 240, width: 640, fontSize: 230, font: 'Anton', fill: '#111111', lineHeight: 0.9 },
    { type: 'rect', left: 60, top: 520, width: 380, height: 84, fill: '#e63946', radius: 12 },
    { type: 'text', text: 'PASSO A PASSO', left: 60, top: 538, width: 380, fontSize: 44, font: 'Bebas Neue', fill: '#ffffff', align: 'center', spacing: 80 },
  ] },
];

export const formatOf = (id: string) => formats.find(f => f.id === id);

/** Apresentação (slides): todas as páginas deitadas em 16:9 ou 4:3. Ganha a faixa de miniaturas. */
export function isPresentation(pages: { largura: number; altura: number }[]) {
  return pages.length > 0 && pages.every(p => p.largura > p.altura && [16 / 9, 4 / 3].some(r => Math.abs(p.largura / p.altura - r) < 0.03));
}

/** Páginas de um design guardado (JSON do Fabric de cada uma). Com uma página só, o design é o
 * próprio JSON do Fabric (como os antigos); com várias, `{"ayla_paginas": [...]}`. */
export function designPages(data: string): string[] {
  try {
    const value = JSON.parse(data) as { ayla_paginas?: unknown };
    if (Array.isArray(value.ayla_paginas) && value.ayla_paginas.length && value.ayla_paginas.every(p => typeof p === 'string')) return value.ayla_paginas as string[];
  } catch { /* fica como uma página */ }
  return [data];
}
export const designData = (pages: string[]) => pages.length === 1 ? pages[0] : JSON.stringify({ ayla_paginas: pages });
/** Nome de um design guardado: o digitado ou, vazio, "Sem nome". */
export const designName = (name: string) => name.trim() || 'Sem nome';
/** Nome padrão de um design novo: o formato (ou o modelo) e a data. */
export function defaultName(label: string, now = new Date()) {
  return `${label} · ${now.toLocaleDateString('pt-BR', { day: '2-digit', month: 'short' })}`;
}

/** Rascunho que a Ayla cria pela conversa (`{"ayla_draft": …}` no design guardado): os mesmos
 * elementos dos modelos, mais ícones do pacote (SVG) e fotos (data URL com crédito). */
export type DraftObject = Spec
  | { type: 'icon'; svg: string; left: number; top: number; size: number; color: string }
  | { type: 'image'; url: string; credit: string; left: number; top: number; width: number; height: number; radius: number; opacity: number };
export type Draft = { format: string; width: number; height: number; background: Fill; objects: DraftObject[] };
export type { DesignCard } from './Desktop';

export function draftOf(data: string): Draft | null {
  try {
    const value = JSON.parse(data) as { ayla_draft?: Draft };
    const draft = value.ayla_draft;
    return draft && Array.isArray(draft.objects) && draft.width > 0 && draft.height > 0 ? draft : null;
  } catch { return null; }
}

/** Abrir um design no editor da Iris a partir de outro lugar (a conversa). A Iris pode ainda não
 * existir na tela: o pedido fica guardado até ela abrir. */
let pendingDesign: string | null = null;
export const OPEN_DESIGN_EVENT = 'ayla-open-design';
export function openDesign(id: string) {
  pendingDesign = id;
  window.dispatchEvent(new CustomEvent(OPEN_DESIGN_EVENT));
}
export const hasPendingDesign = () => pendingDesign !== null;
export function takePendingDesign() { const id = pendingDesign; pendingDesign = null; return id; }
