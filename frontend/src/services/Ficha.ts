// Fichas de RPG (Athena + Iris): mesmos tipos do núcleo (`core/fichas.rs`).
export type Tipo = { tipo: 'linha' } | { tipo: 'numero' } | { tipo: 'bolinhas'; maximo: number } | { tipo: 'bloco' } | { tipo: 'tracos' };
export type Campo = { id: string; rotulo: string; pagina: number; caixa: [number, number, number, number] } & Tipo;
export type Mapa = { nome: string; paginas: number; campos: Campo[] };
export type Traco = { titulo: string; descricao: string };
export type Valor = number | string | Traco[];
export type Valores = Record<string, Valor>;
/** Fichas de um PDF (`ficha_status`), detectadas pela Ayla. */
export type FichaStatus = { paginas: number; porFicha: number; fichas: { primeira: number; nome: string }[]; origem: string; editada: boolean };
/** Ficha aberta (`ficha_abrir`). */
export type FichaAberta = { mapa: Mapa; valores: Valores; primeira: number; nome: string };
/** Cartão da conversa: a mudança proposta, para confirmar. A ficha é o PDF do cofre (`root`/`path`) a partir de `primeira`. */
export type FichaCard = { root: string; path: string; primeira: number; pdf: string; nome: string; valores: Valores; mudancas: { rotulo: string; antes: string; depois: string }[] };

export const tipos: { id: Tipo['tipo']; label: string }[] = [
  { id: 'linha', label: 'Texto curto' }, { id: 'numero', label: 'Número' }, { id: 'bolinhas', label: 'Pontos (bolinhas)' },
  { id: 'bloco', label: 'Texto longo' }, { id: 'tracos', label: 'Traços (título e descrição)' },
];

/** "Vigor · Atual" → "vigor-atual" (igual ao núcleo). */
export function slug(text: string) {
  return text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().split(/[^a-z0-9]+/).filter(Boolean).join('-');
}
