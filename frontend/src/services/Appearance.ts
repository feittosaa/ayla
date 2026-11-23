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

/** Ícone do sistema: a marca da Ayla no destaque, sobre um quadrado com o fundo escolhido. */
export async function drawSystemIcon(a: Appearance, mode: Mode): Promise<string> {
  const side = 512; const canvas = document.createElement('canvas'); canvas.width = canvas.height = side;
  const ctx = canvas.getContext('2d'); if (!ctx) throw new Error('Canvas indisponível');
  const bg = safeBackground(mode === 'escuro' ? a.dark_background : a.light_background, mode);
  ctx.fillStyle = bg; ctx.beginPath(); ctx.roundRect(16, 16, side - 32, side - 32, 112); ctx.fill();
  const svg = MARKS.ayla[a.icon_ayla - 1].replace('currentColor', isHex(a.accent) ? a.accent : DEFAULT_APPEARANCE.accent);
  const image = new Image(); image.src = markUrl(svg); await image.decode();
  ctx.drawImage(image, 104, 104, side - 208, side - 208);
  return canvas.toDataURL('image/png').split(',')[1];
}

/** Mantém o ícone da janela e do menu igual ao escolhido (o núcleo só regrava se mudar). */
export function useSystemIcon(a: Appearance, mode: Mode, ready: boolean) {
  const key = [ready, a.icon_ayla, a.accent, a.dark_background, a.light_background, mode].join('|');
  useEffect(() => {
    if (!ready) return;
    const timer = setTimeout(() => {
      void drawSystemIcon(a, mode).then(png => native('appearance_icon', { png })).catch(e => console.warn('Ícone da Ayla:', e));
    }, 600);
    return () => clearTimeout(timer);
    // `key` resume o que muda no ícone.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
}

export const AppearanceContext = createContext<Appearance>(DEFAULT_APPEARANCE);
export const useAppearance = () => useContext(AppearanceContext);
