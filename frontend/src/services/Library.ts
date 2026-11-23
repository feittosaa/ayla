import { useCallback, useState } from 'react';
import { native } from './Desktop';

/** Biblioteca da Athena: livros (EPUB) e mangás (CBZ) das fontes (importados e pastas conectadas). */
export type Book = { path: string; title: string; series: string; kind: 'epub' | 'cbz'; bytes: number; modified: number; protected?: boolean };
/** Pasta ou livro protegido com senha nos importados. */
export type BookLock = { path: string; kind: 'folder' | 'file'; unlocked: boolean; since: number | null; recovery: boolean };
export type Source = { id: string; name: string; path: string; imported: boolean; available: boolean; books: Book[]; folders?: string[]; locks?: BookLock[] };
export type Progress = { position: string; fraction: number; done: boolean; at: number };
export type ReadingMode = 'rtl' | 'ltr' | 'double' | 'vertical';
export type Library = { sources: Source[]; reading: { books: Record<string, Progress>; modes: Record<string, ReadingMode> } };

/** Abas de livros usam a raiz `biblioteca:<fonte>` e o caminho do livro na fonte. */
export const BOOK_ROOT = 'biblioteca:';
export const isBookRoot = (root: string) => root.startsWith(BOOK_ROOT);
export const bookRoot = (source: string) => `${BOOK_ROOT}${source}`;
export const sourceOf = (root: string) => root.slice(BOOK_ROOT.length);
export const readingKey = (source: string, path: string) => `${source}|${path}`;
const windows = () => navigator.userAgent.includes('Windows');
export const bookUrl = (source: string, path: string) =>
  `${windows() ? 'http://biblioteca-file.localhost/' : 'biblioteca-file://localhost/'}${encodeURIComponent(source)}/${encodeURIComponent(path)}`;

/** Ordem natural (Capitulo 9 antes de Capitulo 10), a mesma do núcleo. */
const collator = new Intl.Collator('pt-BR', { numeric: true, sensitivity: 'base' });
export const natural = (a: string, b: string) => collator.compare(a, b);

export function useLibrary() {
  const [library, setLibrary] = useState<Library | null>(null);
  const load = useCallback(async () => { setLibrary(await native<Library>('library_list')); }, []);
  /** Guarda onde parou (na hora na tela; no disco pelo núcleo). */
  const saveProgress = useCallback((source: string, path: string, progress: Omit<Progress, 'at'>) => {
    setLibrary(current => {
      if (!current) return current;
      const key = readingKey(source, path);
      const done = progress.done || Boolean(current.reading.books[key]?.done);
      return { ...current, reading: { ...current.reading, books: { ...current.reading.books, [key]: { ...progress, done, at: Date.now() / 1000 } } } };
    });
    return native('library_progress', { source, path, ...progress });
  }, []);
  const saveMode = useCallback((source: string, series: string, mode: ReadingMode) => {
    setLibrary(current => current && { ...current, reading: { ...current.reading, modes: { ...current.reading.modes, [readingKey(source, series)]: mode } } });
    return native('library_mode', { source, series, mode });
  }, []);
  /** Marca vários livros de uma fonte como lidos ou não lidos (seleção múltipla). */
  const markBooks = useCallback((source: string, paths: string[], done: boolean) => {
    setLibrary(current => {
      if (!current) return current;
      const books = { ...current.reading.books };
      for (const path of paths) {
        const key = readingKey(source, path);
        if (done) books[key] = { position: books[key]?.position ?? '0', fraction: 1, done: true, at: Date.now() / 1000 };
        else delete books[key];
      }
      return { ...current, reading: { ...current.reading, books } };
    });
    return native('library_mark', { source, paths, done });
  }, []);
  return { library, load, saveProgress, saveMode, markBooks };
}

/** Capítulos da mesma série, em ordem. */
export function seriesOf(library: Library | null, source: string, series: string): Book[] {
  return (library?.sources.find(s => s.id === source)?.books ?? []).filter(b => b.series === series).sort((a, b) => natural(a.path, b.path));
}
