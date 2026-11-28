/** Busca dentro de uma nota (Ctrl+F na Athena): sem diferenciar maiúsculas nem acentos. */

/** Texto dobrado com o mesmo tamanho do original (cada unidade vira uma), para os índices servirem
 * no texto de verdade: "Ação" → "acao". */
export function fold(text: string) {
  let out = '';
  for (const unit of text) {
    const base = unit.normalize('NFD')[0].toLowerCase();
    out += base.length === unit.length ? base : unit.toLowerCase().length === unit.length ? unit.toLowerCase() : unit;
  }
  return out;
}

/** Posições [início, fim) de cada ocorrência de `query` em `text`, sem sobreposição; no máximo `limit`. */
export function findAll(text: string, query: string, limit = 2000): [number, number][] {
  const needle = fold(query.trim() ? query : '');
  if (!needle) return [];
  const hay = fold(text);
  const out: [number, number][] = [];
  for (let at = hay.indexOf(needle); at >= 0 && out.length < limit; at = hay.indexOf(needle, at + needle.length)) out.push([at, at + needle.length]);
  return out;
}
