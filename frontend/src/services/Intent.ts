/** Pedido explícito para criar uma imagem ("gere uma imagem de…", "desenhe um logo…"). */
export function asksForImage(text: string): boolean {
  return /^\s*(por favor[,\s]*)?(gere|gera|gerar|crie|cria|criar|desenhe|desenha|desenhar|fa[cç]a|faz|fazer|produza|ilustre)\s+(uma?|a|o)?\s*(imagem|foto|figura|ilustra[cç][aã]o|desenho|logo|logotipo|arte|pintura|papel de parede)\b/i.test(text);
}
