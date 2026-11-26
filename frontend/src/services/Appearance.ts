import { createContext, useContext, useEffect, useState } from 'react';
import { native } from './Desktop';
import { DEFAULT_APPEARANCE, isHex, palette, safeBackground, type Appearance, type Mode } from './Palette';
import ayla1 from '../assets/marcas/ayla_1.svg?raw';
import ayla2 from '../assets/marcas/ayla_2.svg?raw';
import ayla3 from '../assets/marcas/ayla_3.svg?raw';
import athena3 from '../assets/marcas/athena_3.svg?raw';
import athena4 from '../assets/marcas/athena_4.svg?raw';
import iris from '../assets/marcas/iris.svg?raw';
import hand from '../assets/marcas/hand.svg?raw';

/** Marcas (SVG monocromáticos, em `currentColor`). O Hand é do futuro órgão. As da Athena são as
 * variantes 3 e 4 (as 1 e 2 saíram). */
export const FIRST_VARIANT: Partial<Record<'ayla' | 'athena' | 'iris' | 'hand', number>> = { athena: 3 };
export const MARKS = {
  ayla: [ayla1, ayla2, ayla3], athena: [athena3, athena4], iris: [iris], hand: [hand],
} as const;
export type MarkName = keyof typeof MARKS;
export const markUrl = (svg: string) => `data:image/svg+xml,${encodeURIComponent(svg)}`;

export * from './Palette';

/** Modo efetivo: "sistema" segue o modo claro/escuro do sistema. */
export function useMode(a: Appearance): Mode {
  const query = '(prefers-color-scheme: dark)';
  const [systemDark, setSystemDark] = useState(() => typeof matchMedia !== 'function' || matchMedia(query).matches);
  useEffect(() => {
    if (typeof matchMedia !== 'function') return;
    const list = matchMedia(query);
    const change = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    list.addEventListener('change', change);
    return () => list.removeEventListener('change', change);
  }, []);
  return a.mode === 'sistema' ? (systemDark ? 'escuro' : 'claro') : a.mode === 'claro' ? 'claro' : 'escuro';
}

/** Aplica a paleta na raiz do documento (vale para todas as telas e órgãos abertos na Ayla). */
export function applyAppearance(a: Appearance, mode: Mode) {
  const root = document.documentElement;
  for (const [key, value] of Object.entries(palette(a, mode))) {
    if (key === 'color-scheme') root.style.colorScheme = value; else root.style.setProperty(key, value);
  }
}

/** Ícone do sistema: a marca (da Ayla, da Athena ou da Iris) no destaque, sobre um quadrado com
 * o fundo escolhido. */
export async function drawSystemIcon(a: Appearance, mode: Mode, name: 'ayla' | 'athena' | 'iris' | 'notification' = 'ayla'): Promise<string> {
  const side = 512; const canvas = document.createElement('canvas'); canvas.width = canvas.height = side;
  const ctx = canvas.getContext('2d'); if (!ctx) throw new Error('Canvas indisponível');
  if (name !== 'notification') {
    const bg = safeBackground(mode === 'escuro' ? a.dark_background : a.light_background, mode);
    ctx.fillStyle = bg; ctx.beginPath(); ctx.roundRect(16, 16, side - 32, side - 32, 112); ctx.fill();
  }
  const brand = name === 'notification' ? 'ayla' : name;
  const marks: readonly string[] = MARKS[brand];
  const variant = brand === 'ayla' ? a.icon_ayla : name === 'athena' ? a.icon_athena : 1;
  const mark = marks[variant - (FIRST_VARIANT[brand] ?? 1)] ?? marks[0];
  const accent = isHex(a.accent) ? a.accent : DEFAULT_APPEARANCE.accent;
  const svg = mark.replaceAll('currentColor', name === 'notification' ? '#fff' : accent);
  const image = new Image(); image.src = markUrl(svg); await image.decode();
  if (name === 'notification') drawCentered(ctx, image, side, NOTIFICATION_FILL);
  else ctx.drawImage(image, 104, 104, side - 208, side - 208);
  return canvas.toDataURL('image/png').split(',')[1];
}

/** Quanto do quadrado a marca ocupa na notificação: os painéis costumam mostrar o ícone num
 * círculo, e a marca larga encostava na borda. */
const NOTIFICATION_FILL = 0.7;

/** Desenha a marca centrada pelo que ela tem de visível (não pela caixa do SVG), com o lado maior
 * ocupando `fill` do quadrado. */
function drawCentered(ctx: CanvasRenderingContext2D, image: HTMLImageElement, side: number, fill: number) {
  const probe = document.createElement('canvas'); probe.width = probe.height = side;
  const pctx = probe.getContext('2d'); if (!pctx) { ctx.drawImage(image, 0, 0, side, side); return; }
  pctx.drawImage(image, 0, 0, side, side);
  const alpha = pctx.getImageData(0, 0, side, side).data;
  let left = side, top = side, right = -1, bottom = -1;
  for (let y = 0; y < side; y++) for (let x = 0; x < side; x++) {
    if (alpha[(y * side + x) * 4 + 3] > 8) { if (x < left) left = x; if (x > right) right = x; if (y < top) top = y; if (y > bottom) bottom = y; }
  }
  if (right < 0) return;
  const w = right - left + 1, h = bottom - top + 1, scale = side * fill / Math.max(w, h);
  // Redesenha o vetor (e não o recorte) para continuar nítido em qualquer escala.
  ctx.drawImage(image, (side - w * scale) / 2 - left * scale, (side - h * scale) / 2 - top * scale, side * scale, side * scale);
}

/** Mantém o ícone da janela, do menu e do "Abrir com" (Ayla, Athena e Iris) igual ao escolhido
 * (o núcleo só regrava o que mudar). */
export function useSystemIcon(a: Appearance, mode: Mode, ready: boolean) {
  const key = [ready, a.icon_ayla, a.icon_athena, a.accent, a.dark_background, a.light_background, mode].join('|');
  useEffect(() => {
    if (!ready) return;
    const timer = setTimeout(() => {
      void Promise.all((['ayla', 'athena', 'iris', 'notification'] as const).map(name => drawSystemIcon(a, mode, name)))
        .then(([png, athena, iris, notification]) => native('appearance_icon', { png, athena, iris, notification }))
        .catch(e => console.warn('Ícone da Ayla:', e));
    }, 600);
    return () => clearTimeout(timer);
    // `key` resume o que muda nos ícones.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
}

export const AppearanceContext = createContext<Appearance>(DEFAULT_APPEARANCE);
export const useAppearance = () => useContext(AppearanceContext);
