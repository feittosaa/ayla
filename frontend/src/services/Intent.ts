/** Pedido explícito para criar uma imagem ("gere uma imagem de…", "desenhe um logo…"). */
export function asksForImage(text: string): boolean {
  return /^\s*(por favor[,\s]*)?(gere|gera|gerar|crie|cria|criar|desenhe|desenha|desenhar|fa[cç]a|faz|fazer|produza|ilustre)\s+(uma?|a|o)?\s*(imagem|foto|figura|ilustra[cç][aã]o|desenho|logo|logotipo|arte|pintura|papel de parede)\b/i.test(text);
}

/** Pedido explícito para olhar a tela ("o que é esse carro na tela?", "veja minha tela"). */
export function asksAboutScreen(text: string): boolean {
  return /\b(veja|olhe|olha|ver|vê|analise|descreva|leia|l[eê])\b[^.?!]{0,30}\b(tela|screen)\b/i.test(text)
    || /\b(n[ao]|nest[ae]|ness[ae]|aquela|minha|toda a)\s+(minha\s+)?tela\b/i.test(text) && /\b(isso|esse|essa|este|esta|aqui|aparec\w+|mostr\w+|qual|quem|o que|que)\b/i.test(text);
}
