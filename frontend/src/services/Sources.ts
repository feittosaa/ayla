/** Fontes que a Ayla consulta, além das bases de dados: imagens e pesquisa acadêmica.
 * `key`: precisa de chave (configurada em Configurações › Imagens e pesquisa).
 * `link`: sem API pública; a Ayla devolve o link da busca para abrir no navegador. */
export type Source = { id: string; name: string; hint: string; key?: 'unsplash' | 'pexels' | 'pixabay'; link?: boolean };
export const IMAGE_SOURCES: Source[] = [
  { id: 'imagens:unsplash', name: 'Unsplash', hint: 'Fotos de licença livre (apresentações, mockups, uso comercial)', key: 'unsplash' },
  { id: 'imagens:pexels', name: 'Pexels', hint: 'Fotos de licença livre', key: 'pexels' },
  { id: 'imagens:pixabay', name: 'Pixabay', hint: 'Fotos e ilustrações de licença livre', key: 'pixabay' },
  { id: 'imagens:wallhaven', name: 'Wallhaven', hint: 'Wallpapers e referências (direitos dos autores)' },
  { id: 'imagens:pinterest', name: 'Pinterest', hint: 'Sem API pública: a Ayla devolve o link da busca', link: true },
  { id: 'imagens:cosmos', name: 'Cosmos', hint: 'Sem API pública: a Ayla devolve o link da busca', link: true },
  { id: 'imagens:same-energy', name: 'same.energy', hint: 'Busca visual, sem busca por texto: a Ayla devolve o link do site', link: true },
  { id: 'imagens:arena', name: 'Are.na', hint: 'Busca pela API só no plano pago: a Ayla devolve o link do site', link: true },
];
export const ACADEMIC_SOURCES: Source[] = [
  { id: 'academico:openalex', name: 'OpenAlex', hint: 'Artigos acadêmicos (inclui SciELO e revistas brasileiras)' },
  { id: 'academico:crossref', name: 'Crossref', hint: 'Artigos com DOI' },
  { id: 'academico:arxiv', name: 'arXiv', hint: 'Pré-publicações (exatas, computação)' },
  { id: 'academico:scholar', name: 'Google Acadêmico', hint: 'Sem API: a Ayla devolve o link da busca', link: true },
];
export type KeyStatus = Record<NonNullable<Source['key']>, boolean>;
