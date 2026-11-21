/** Prefixo dos links internos entre notas (nunca é um endereço da web). */
export const WIKI = 'athena-link:';

/** Links wiki do Obsidian viram links internos: [[nota]], [[nota|apelido]], [[nota#seção]] e
 * ![[anexo]] (anexos aparecem como link). Código em bloco ou em linha fica como está. */
export function wikiToMarkdown(text: string) {
  return text.split(/(```[\s\S]*?```|`[^`\n]*`)/g).map((part, i) => i % 2 ? part : part.replace(/!?\[\[([^\]\n|#]+)(#[^\]\n|]*)?(?:\|([^\]\n]+))?\]\]/g, (_match: string, target: string, section: string | undefined, alias: string | undefined) => {
    const label = (alias || `${target}${section || ''}`).replace(/[[\]]/g, '');
    return `[${label}](${WIKI}${encodeURIComponent(target.trim())})`;
  })).join('');
}
