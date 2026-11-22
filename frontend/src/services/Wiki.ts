/** Prefixos dos links internos (nunca são endereços da web): notas/anexos e tags. */
export const WIKI = 'athena-link:';
export const TAG = 'athena-tag:';
export const IMAGE = /\.(png|jpe?g|jfif|gif|webp|bmp|svg|avif)$/i;
export const PDF = /\.pdf$/i;
/** Anexos que a Athena mostra: imagens e PDF (o resto continua link). */
export const isAttachment = (target: string) => IMAGE.test(target.split('#')[0].trim()) || PDF.test(target.split('#')[0].trim());
/** Endereço de um anexo de uma raiz (servido pelo núcleo, lido da pasta da nota). */
export const attachmentUrl = (root: string, target: string) => `${typeof navigator !== 'undefined' && navigator.userAgent.includes('Windows') ? 'http://athena-file.localhost/' : 'athena-file://localhost/'}${encodeURIComponent(root)}/${encodeURIComponent(target)}`;

/** Sintaxe do Obsidian em Markdown comum: [[nota]], [[nota|apelido]], [[nota#seção]], ![[foto.png]]
 * e ![[foto.png|300]] (imagem, com largura), ![[livro.pdf]] (PDF na nota) e #tag. Código em bloco ou em linha fica como está. */
export function wikiToMarkdown(text: string) {
  return text.split(/(```[\s\S]*?```|`[^`\n]*`)/g).map((part, i) => i % 2 ? part : part
    .replace(/(!?)\[\[([^\]\n|#]+)(#[^\]\n|]*)?(?:\|([^\]\n]+))?\]\]/g, (_match: string, bang: string, target: string, section: string | undefined, alias: string | undefined) => {
      const clean = target.trim();
      // ![[foto.png|300]] ou |300x200: tamanho, como no Obsidian (não é apelido).
      const size = bang && alias && /^\d+(x\d+)?$/.test(alias.trim()) ? alias.trim() : '';
      const label = (size ? clean : alias || `${clean}${section || ''}`).replace(/[[\]"]/g, '');
      return bang && isAttachment(clean) ? `![${label}](${WIKI}${encodeURIComponent(clean)}${size ? ` "${size}"` : ''})` : `[${label}](${WIKI}${encodeURIComponent(clean)})`;
    })
    .replace(/(^|[ \t])#([\p{L}\p{N}_/-]*\p{L}[\p{L}\p{N}_/-]*)/gmu, (_match: string, space: string, tag: string) => `${space}[#${tag}](${TAG}${encodeURIComponent(tag.toLowerCase())})`)
  ).join('');
}

export type Property = { key: string; values: string[] };
export type Banner = { target: string; y: number };
export type Frontmatter = { properties: Property[]; banner: Banner | null; body: string };

const unquote = (value: string) => value.trim().replace(/^(["'])(.*)\1$/, '$2').trim();

/** Propriedades do Obsidian (frontmatter YAML no início da nota): `chave: valor`, listas
 * (`- item` ou `[a, b]`). O banner segue o make-md/Banners: `banner` (caminho, [[imagem]] ou
 * ![[imagem]]) e `banner_y` (0–100, ou 0–1). Endereços da web não viram banner. */
export function frontmatter(text: string): Frontmatter {
  const match = /^---[ \t]*\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  if (!match) return { properties: [], banner: null, body: text };
  const properties: Property[] = [];
  for (const line of match[1].split(/\r?\n/)) {
    const item = /^\s+-\s*(.*)$/.exec(line);
    if (item && properties.length) { const value = unquote(item[1]); if (value) properties[properties.length - 1].values.push(value); continue; }
    const pair = /^([^\s:#][^:]*):\s*(.*)$/.exec(line);
    if (!pair) continue;
    const raw = pair[2].trim();
    const values = /^\[.*\]$/.test(raw) && !raw.startsWith('[[') ? raw.slice(1, -1).split(',').map(unquote).filter(Boolean) : raw ? [unquote(raw)] : [];
    properties.push({ key: pair[1].trim(), values });
  }
  const value = (key: string) => properties.find(p => p.key === key)?.values[0] ?? '';
  const target = value('banner').replace(/^!?\[\[([^\]|#]+)(?:[|#][^\]]*)?\]\]$/, '$1').trim();
  let y = Number.parseFloat(value('banner_y'));
  if (!Number.isFinite(y)) y = 50; else if (y <= 1) y *= 100;
  const banner = target && !/^[a-z]+:\/\//i.test(target) ? { target, y: Math.min(100, Math.max(0, y)) } : null;
  return { properties: properties.filter(p => !['banner', 'banner_y', 'banner_x', 'banner_icon', 'banner_lock'].includes(p.key)), banner, body: text.slice(match[0].length) };
}
