// Sem dependências: usado pela fala e pelos testes do cliente.
/** Divide texto em frases completas; devolve as frases e o resto ainda incompleto. */
export function splitSentences(text: string): { sentences: string[]; rest: string } {
  const sentences: string[] = [];
  const pattern = /[^.!?…\n]*(?:[.!?…]+(?=\s|$)|\n)/g;
  let consumed = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index !== consumed) break;
    consumed += match[0].length;
    if (match[0].trim().length > 1) sentences.push(match[0].trim());
  }
  return { sentences, rest: text.slice(consumed) };
}
