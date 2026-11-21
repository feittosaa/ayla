/** Prefixos dos links internos (nunca são endereços da web): notas/anexos e tags. */
export const WIKI = 'athena-link:';
export const TAG = 'athena-tag:';
const IMAGE = /\.(png|jpe?g|gif|webp|bmp|svg|avif)$/i;

/** Sintaxe do Obsidian em Markdown comum: [[nota]], [[nota|apelido]], [[nota#seção]], ![[foto.png]]
 * (imagem) e #tag. Código em bloco ou em linha fica como está. */
export function wikiToMarkdown(text: string) {
  return text.split(/(```[\s\S]*?```|`[^`\n]*`)/g).map((part, i) => i % 2 ? part : part
    .replace(/(!?)\[\[([^\]\n|#]+)(#[^\]\n|]*)?(?:\|([^\]\n]+))?\]\]/g, (_match: string, bang: string, target: string, section: string | undefined, alias: string | undefined) => {
      const clean = target.trim();
      const label = (alias || `${clean}${section || ''}`).replace(/[[\]]/g, '');
      return bang && IMAGE.test(clean) ? `![${label}](${WIKI}${encodeURIComponent(clean)})` : `[${label}](${WIKI}${encodeURIComponent(clean)})`;
    })
    .replace(/(^|[ \t])#([\p{L}\p{N}_/-]*\p{L}[\p{L}\p{N}_/-]*)/gmu, (_match: string, space: string, tag: string) => `${space}[#${tag}](${TAG}${encodeURIComponent(tag.toLowerCase())})`)
  ).join('');
}
