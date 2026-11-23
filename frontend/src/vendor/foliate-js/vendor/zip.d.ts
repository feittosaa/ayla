// Tipos mínimos do zip.js (embutido no foliate-js) usados pelo leitor de CBZ.
export function configure(options: { useWebWorkers?: boolean }): void;
export class BlobReader { constructor(blob: Blob); }
export class BlobWriter { constructor(type?: string); }
export class ZipReader { constructor(reader: BlobReader); getEntries(): Promise<unknown[]>; close(): Promise<void>; }
