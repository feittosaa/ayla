/** Fontes do pacote opcional dos designs, registradas no documento uma vez por sessão. */
import { invoke } from '@tauri-apps/api/core';
import { native } from './Desktop';
import type { PackFont } from './Design';

// Faixas do Fontsource: cada família tem um arquivo para o latim básico e outro para o estendido.
const RANGES: Record<string, string> = {
  latin: 'U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD',
  'latin-ext': 'U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, U+0304, U+0308, U+0329, U+1D00-1DBF, U+1E00-1E9F, U+1EF2-1EFF, U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF',
};

let loading: Promise<PackFont[]> | null = null;

/** Registra as fontes instaladas (sem o pacote, devolve lista vazia). Repetir não baixa de novo. */
export function loadPackFonts(): Promise<PackFont[]> {
  loading ??= (async () => {
    const fonts = await native<PackFont[]>('design_fonts').catch(() => []);
    const faces: Promise<unknown>[] = [];
    for (const font of fonts) {
      for (const face of font.faces) {
        for (const file of face.files) {
          const range = file.includes('-latin-ext-') ? RANGES['latin-ext'] : RANGES.latin;
          faces.push(invoke<ArrayBuffer>('design_font', { file }).then(bytes => {
            const loaded = new FontFace(font.family, bytes, { weight: String(face.weight), style: face.style, unicodeRange: range });
            document.fonts.add(loaded);
            return loaded.load();
          }).catch(() => undefined));
        }
      }
    }
    await Promise.all(faces);
    return fonts;
  })();
  const current = loading;
  // Lista vazia (pacote ainda não baixado): tenta de novo na próxima vez.
  void current.then(fonts => { if (!fonts.length && loading === current) loading = null; });
  return current;
}

/** Depois de instalar o pacote: esquece a tentativa anterior para carregar as fontes novas. */
export function resetPackFonts() { loading = null; }
