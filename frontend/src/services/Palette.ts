import type { Preferences } from './Desktop';

/** Aparência escolhida em Configurações (guardada nas preferências pelo núcleo). */
export type Appearance = Preferences['appearance'];
export type Mode = 'escuro' | 'claro';

export const DEFAULT_APPEARANCE: Appearance = {
  mode: 'escuro', accent: '#ff9f43', dark_background: '#111214', light_background: '#f6f6f4',
  roundness: 100, opacity: 100, icon_ayla: 1, icon_athena: 1,
};

export const ACCENTS = ['#ff9f43', '#f2c14e', '#5ac18e', '#4fb3d9', '#6f8cff', '#a77bf3', '#ec6fa6', '#e8604c'];
export const BACKGROUNDS: Record<Mode, { name: string; color: string }[]> = {
  escuro: [
    { name: 'Grafite', color: '#111214' }, { name: 'Preto', color: '#000000' }, { name: 'Quente', color: '#16120f' },
    { name: 'Azulado', color: '#0f131a' }, { name: 'Verde', color: '#0f1512' }, { name: 'Ameixa', color: '#151018' },
  ],
  claro: [
    { name: 'Papel', color: '#f6f6f4' }, { name: 'Branco', color: '#ffffff' }, { name: 'Areia', color: '#f5f0e8' },
    { name: 'Névoa', color: '#eef2f6' }, { name: 'Sálvia', color: '#eff3ee' }, { name: 'Lavanda', color: '#f3f0f7' },
  ],
};
export const ROUNDNESS = [{ label: 'Retas', value: 0 }, { label: 'Suaves', value: 50 }, { label: 'Padrão', value: 100 }, { label: 'Arredondadas', value: 150 }];

// ---- Cores ----
type Rgb = [number, number, number];
const rgb = (hex: string): Rgb => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
const hex = (c: Rgb) => '#' + c.map(v => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0')).join('');
/** Mistura `a` com `b` (t = 0 → a; t = 1 → b). */
export const mix = (a: string, b: string, t: number) => { const [x, y] = [rgb(a), rgb(b)]; return hex([0, 1, 2].map(i => x[i] + (y[i] - x[i]) * t) as Rgb); };
export function luminance(color: string) {
  const [r, g, b] = rgb(color).map(v => { const c = v / 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
export const contrast = (a: string, b: string) => { const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
/** Aproxima `color` de `toward` até atingir o contraste mínimo sobre `over`. */
function readable(color: string, toward: string, over: string, min = 4.5) {
  for (let t = 0; t <= 1; t += 0.04) { const c = mix(color, toward, t); if (contrast(c, over) >= min) return c; }
  return toward;
}
const alpha = (color: string, opacity: number) => opacity >= 100 ? color : `rgb(${rgb(color).join(' ')} / ${opacity}%)`;
export const isHex = (value: string) => /^#[0-9a-f]{6}$/i.test(value);

/** Fundo dentro da faixa de cada modo, para o texto manter o contraste com qualquer cor livre. */
export function safeBackground(color: string, mode: Mode) {
  let c = isHex(color) ? color.toLowerCase() : DEFAULT_APPEARANCE[mode === 'escuro' ? 'dark_background' : 'light_background'];
  for (let i = 0; i < 40 && (mode === 'escuro' ? luminance(c) > 0.045 : luminance(c) < 0.78); i++) c = mix(c, mode === 'escuro' ? '#000000' : '#ffffff', 0.12);
  return c;
}

/** Paleta completa (variáveis `--ayla-*`) a partir do modo, do fundo e do destaque. */
export function palette(a: Appearance, mode: Mode): Record<string, string> {
  const dark = mode === 'escuro';
  const bg = safeBackground(dark ? a.dark_background : a.light_background, mode);
  const fg = dark ? '#ececee' : '#1b1c1f';
  const step = (t: number) => mix(bg, fg, t);
  const sidebar = dark ? step(0.023) : step(0.03);
  const accent = isHex(a.accent) ? a.accent : DEFAULT_APPEARANCE.accent;
  const onAccent = contrast('#ffffff', accent) >= contrast('#1d1206', accent) ? '#ffffff' : '#1d1206';
  return {
    'color-scheme': dark ? 'dark' : 'light',
    '--ayla-bg': bg,
    // Só o corpo da janela pinta este fundo; com transparência, ele fica translúcido.
    '--ayla-window': alpha(bg, a.opacity),
    '--ayla-rail': alpha(dark ? mix(bg, '#000000', 0.25) : step(0.055), a.opacity),
    '--ayla-sidebar': alpha(sidebar, a.opacity),
    '--ayla-panel': dark ? step(0.041) : step(0.045),
    '--ayla-control': dark ? step(0.05) : step(0.06),
    '--ayla-hover': dark ? step(0.087) : step(0.085),
    '--ayla-hover-strong': dark ? step(0.12) : step(0.12),
    '--ayla-selected': dark ? step(0.087) : step(0.085),
    '--ayla-selected-strong': step(0.12),
    '--ayla-border': dark ? step(0.12) : step(0.16),
    '--ayla-border-soft': dark ? step(0.087) : step(0.1),
    '--ayla-border-strong': dark ? step(0.16) : step(0.24),
    '--ayla-code': dark ? mix(bg, '#000000', 0.3) : step(0.04),
    '--ayla-text': fg,
    '--ayla-text-soft': readable(step(dark ? 0.62 : 0.7), fg, sidebar),
    '--ayla-text-faint': readable(step(dark ? 0.54 : 0.6), fg, sidebar),
    '--ayla-disabled': step(dark ? 0.5 : 0.48),
    '--ayla-accent': accent,
    '--ayla-accent-soft': dark ? readable(mix(accent, '#ffffff', 0.25), '#ffffff', bg) : readable(accent, '#000000', bg),
    '--ayla-accent-ring': mix(bg, accent, dark ? 0.14 : 0.18),
    '--ayla-on-accent': onAccent,
    '--ayla-error': dark ? '#ffb4a2' : '#b3261e',
    '--ayla-round': String(a.roundness / 100),
  };
}
