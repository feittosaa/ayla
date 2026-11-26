/** Geometria do editor da Iris. O recorte é guardado em frações (0 a 1) do quadro já girado, igual ao
 * que o núcleo recebe; as proporções são em pixels. */
export type Crop = { x: number; y: number; width: number; height: number };
export type Aspect = 'livre' | '1:1' | '16:9' | '4:3';
export type Handle = 'move' | 'nw' | 'ne' | 'sw' | 'se' | 'n' | 's' | 'e' | 'w';

export const aspects: { id: Aspect; label: string }[] = [
  { id: 'livre', label: 'Livre' }, { id: '1:1', label: '1:1' }, { id: '16:9', label: '16:9' }, { id: '4:3', label: '4:3' },
];
const MIN = 0.02;
const ratioOf = (aspect: Aspect) => aspect === '1:1' ? 1 : aspect === '16:9' ? 16 / 9 : aspect === '4:3' ? 4 / 3 : null;

/** Quadro depois de girar (90° e 270° trocam largura e altura). */
export function rotated(width: number, height: number, rotate: number) {
  return rotate % 180 ? { width: height, height: width } : { width, height };
}

/** Proporção em frações: largura/altura do recorte em frações para dar `aspect` em pixels. Na
 * imagem em pé, a proporção deitada (16:9, 4:3) vira em pé (9:16, 3:4). */
export function fractionRatio(aspect: Aspect, width: number, height: number) {
  const ratio = ratioOf(aspect);
  if (ratio === null) return null;
  const pixels = height > width ? 1 / ratio : ratio;
  return pixels * height / width;
}

/** Maior recorte centralizado na proporção (ou o quadro inteiro, em Livre). */
export function fitCrop(aspect: Aspect, width: number, height: number): Crop {
  const r = fractionRatio(aspect, width, height);
  if (r === null) return { x: 0, y: 0, width: 1, height: 1 };
  const w = r >= 1 ? 1 : r;
  const h = r >= 1 ? 1 / r : 1;
  return { x: (1 - w) / 2, y: (1 - h) / 2, width: w, height: h };
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Arrasta o recorte (mover, um canto ou uma borda) por `dx`, `dy` em frações; com proporção, a
 * largura manda nos cantos, e a borda puxa o outro lado junto, centralizado. */
export function dragCrop(start: Crop, handle: Handle, dx: number, dy: number, ratio: number | null): Crop {
  if (handle === 'move') {
    return { ...start, x: clamp(start.x + dx, 0, 1 - start.width), y: clamp(start.y + dy, 0, 1 - start.height) };
  }
  if (handle === 'e' || handle === 'w') {
    const west = handle === 'w';
    const ax = west ? start.x + start.width : start.x;
    let w = clamp(start.width + (west ? -dx : dx), MIN, west ? ax : 1 - ax);
    if (ratio === null) return { ...start, x: west ? ax - w : ax, width: w };
    // A altura acompanha em volta do mesmo centro, sem passar da imagem.
    const cy = start.y + start.height / 2;
    const maxH = 2 * Math.min(cy, 1 - cy);
    let h = w / ratio;
    if (h > maxH) { h = maxH; w = h * ratio; }
    return { x: west ? ax - w : ax, y: cy - h / 2, width: w, height: h };
  }
  if (handle === 'n' || handle === 's') {
    const north = handle === 'n';
    const ay = north ? start.y + start.height : start.y;
    let h = clamp(start.height + (north ? -dy : dy), MIN, north ? ay : 1 - ay);
    if (ratio === null) return { ...start, y: north ? ay - h : ay, height: h };
    const cx = start.x + start.width / 2;
    const maxW = 2 * Math.min(cx, 1 - cx);
    let w = h * ratio;
    if (w > maxW) { w = maxW; h = w / ratio; }
    return { x: cx - w / 2, y: north ? ay - h : ay, width: w, height: h };
  }
  const west = handle === 'nw' || handle === 'sw';
  const north = handle === 'nw' || handle === 'ne';
  // Canto oposto fica parado.
  const ax = west ? start.x + start.width : start.x;
  const ay = north ? start.y + start.height : start.y;
  const maxW = west ? ax : 1 - ax;
  const maxH = north ? ay : 1 - ay;
  let w = clamp(start.width + (west ? -dx : dx), MIN, maxW);
  let h = clamp(start.height + (north ? -dy : dy), MIN, maxH);
  if (ratio !== null) {
    h = w / ratio;
    if (h > maxH) { h = maxH; w = h * ratio; }
    if (h < MIN) { h = MIN; w = Math.min(maxW, h * ratio); }
  }
  return { x: west ? ax - w : ax, y: north ? ay - h : ay, width: w, height: h };
}

/** Recorte que não muda nada. */
export const isFull = (crop: Crop | null) => !crop || (crop.x <= 1e-4 && crop.y <= 1e-4 && crop.width >= 1 - 1e-4 && crop.height >= 1 - 1e-4);

/** Tamanho final do recorte em pixels. */
export function cropPixels(crop: Crop | null, width: number, height: number) {
  const c = crop ?? { x: 0, y: 0, width: 1, height: 1 };
  return { width: Math.max(1, Math.round(c.width * width)), height: Math.max(1, Math.round(c.height * height)) };
}

/** Segundos como 1:05.3 (ou 1:02:05.3). */
export function clock(seconds: number) {
  const s = Math.max(0, seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const rest = (s % 60).toFixed(1).padStart(4, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${rest}` : `${m}:${rest}`;
}
