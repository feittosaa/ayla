/** Operações de texto do menu da Athena (como no Text Editor do GNOME). Funções puras. */

export type CaseMode = 'upper' | 'lower' | 'invert' | 'title';

export function changeCase(text: string, mode: CaseMode): string {
  if (mode === 'upper') return text.toLocaleUpperCase('pt-BR');
  if (mode === 'lower') return text.toLocaleLowerCase('pt-BR');
  if (mode === 'invert') return [...text].map(c => c === c.toLocaleUpperCase('pt-BR') ? c.toLocaleLowerCase('pt-BR') : c.toLocaleUpperCase('pt-BR')).join('');
  return text.toLocaleLowerCase('pt-BR').replace(/(^|[^\p{L}\p{N}'])(\p{L})/gu, (_, before: string, letter: string) => before + letter.toLocaleUpperCase('pt-BR'));
}

/** Ordena linhas em ordem alfabética (português, sem diferenciar acentos de maiúsculas na comparação). */
export function sortLines(text: string, options: { descending?: boolean; unique?: boolean; ignoreCase?: boolean } = {}): string {
  const collator = new Intl.Collator('pt-BR', { sensitivity: options.ignoreCase === false ? 'variant' : 'accent', numeric: true });
  let lines = text.split('\n');
  const trailing = lines.length > 1 && lines[lines.length - 1] === '';
  if (trailing) lines = lines.slice(0, -1);
  lines.sort(collator.compare);
  if (options.descending) lines.reverse();
  if (options.unique) lines = lines.filter((line, i) => i === 0 || collator.compare(line, lines[i - 1]) !== 0);
  return lines.join('\n') + (trailing ? '\n' : '');
}

/** Junta as linhas com um espaço, tirando o recuo do início de cada uma. */
export function joinLines(text: string): string {
  return text.split('\n').map((line, i) => i === 0 ? line.trimEnd() : line.trim()).filter((line, i) => i === 0 || line !== '').join(' ');
}

/** Título da aba: nome do arquivo, ou "Novo documento", "Novo documento 2"… sem repetir. */
export function untitledName(taken: string[]): string {
  for (let n = 1; ; n++) {
    const name = n === 1 ? 'Novo documento' : `Novo documento ${n}`;
    if (!taken.includes(name)) return name;
  }
}

/** Linha e coluna (contadas a partir de 1, com tabs valendo `tabSize` colunas), como na barra do editor. */
export function position(lineText: string, column: number, tabSize: number): number {
  let visual = 0;
  for (const char of lineText.slice(0, column)) visual = char === '\t' ? visual + tabSize - (visual % tabSize) : visual + 1;
  return visual + 1;
}

/** Recentes: o mais novo na frente, sem repetir, no máximo `limit`. */
export function remember(recent: string[], path: string, limit = 10): string[] {
  return [path, ...recent.filter(p => p !== path)].slice(0, limit);
}
