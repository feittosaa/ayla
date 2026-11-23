/** Segurança dos livros EPUB abertos na Athena.
 *
 * O foliate-js desenha cada capítulo num iframe da mesma origem da Ayla. Nenhum script do livro
 * pode rodar ali (alcançaria a ponte com o núcleo): arquivos de script nem carregam, cada página
 * (X)HTML recebe uma política que bloqueia scripts e rede, e SVGs perdem scripts e eventos.
 */
const BOOK_CSP = "default-src 'none'; img-src blob: data:; style-src blob: data: 'unsafe-inline'; font-src blob: data:; media-src blob: data:";
export function withPolicy(html: string) {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${BOOK_CSP}"/>`;
  if (/<head(\s[^>]*)?>/i.test(html)) return html.replace(/<head(\s[^>]*)?>/i, head => head + meta);
  if (/<html(\s[^>]*)?>/i.test(html)) return html.replace(/<html(\s[^>]*)?>/i, root => `${root}<head>${meta}</head>`);
  return meta + html;
}
export function cleanSvg(svg: string) {
  const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
  doc.querySelectorAll('script, foreignObject').forEach(node => node.remove());
  for (const node of doc.querySelectorAll('*')) {
    for (const attribute of [...node.attributes]) {
      const value = attribute.value.trim().toLowerCase();
      if (attribute.name.toLowerCase().startsWith('on') || value.startsWith('javascript:')) node.removeAttribute(attribute.name);
    }
  }
  return new XMLSerializer().serializeToString(doc);
}
