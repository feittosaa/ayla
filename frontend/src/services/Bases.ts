import { native } from './Desktop';

export type Base = {
  id: string; name: string; publisher: string; description: string; license: string; repository: string; source: string; commit: string; download: number;
  origin: 'ayla' | 'voce'; current: boolean; installed: { commit: string; version: string | null; bytes: number; installedAt: number } | null;
};
export type Validation = {
  id: string; name: string; repository: string; commit: string; license: string; licenseOpen: boolean; format: string;
  download: number; size: number; tables: string[]; warnings: string[]; description: string;
};
const CHANGED = 'ayla-bases-changed';
export const listBases = () => native<Base[]>('bases_list');
/** Avisa as telas abertas (barra lateral e Configurações) de que as bases mudaram. */
export const basesChanged = () => window.dispatchEvent(new Event(CHANGED));
export function onBasesChanged(listener: () => void) {
  window.addEventListener(CHANGED, listener);
  return () => window.removeEventListener(CHANGED, listener);
}
