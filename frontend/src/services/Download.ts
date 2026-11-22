/** Tamanho de cada trecho pedido ao núcleo. */
const PART = 16 * 1024 * 1024;

/** Lê um arquivo em trechos (`Range`) pelos endereços da Ayla (athena-file://, iris-file://) e
 * monta um Blob. Usado pelo vídeo (o player do WebKit no Linux não toca esses endereços) e pelo
 * leitor de PDF. Falha com "grande" acima de `max` e "indisponível" se o núcleo não entregar. */
export async function download(src: string, signal: AbortSignal, max: number, fallbackType: string): Promise<Blob> {
  const probe = await fetch(src, { headers: { Range: 'bytes=0-0' }, signal });
  const total = Number(probe.headers.get('Content-Range')?.split('/')[1] ?? 0);
  const type = probe.headers.get('Content-Type') ?? fallbackType;
  if (!probe.ok || !total) throw new Error('indisponível');
  if (total > max) throw new Error('grande');
  const parts: Blob[] = [];
  for (let start = 0; start < total; start += PART) {
    const response = await fetch(src, { headers: { Range: `bytes=${start}-${Math.min(total, start + PART) - 1}` }, signal });
    if (!response.ok) throw new Error('indisponível');
    parts.push(await response.blob());
  }
  return new Blob(parts, { type });
}
