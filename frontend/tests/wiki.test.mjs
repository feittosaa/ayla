import test from 'node:test';
import assert from 'node:assert/strict';
import { TAG, WIKI, attachmentUrl, frontmatter, isAttachment, storedUrl, wikiToMarkdown } from '../src/services/Wiki.ts';

test('links do Obsidian viram links internos, com apelido e seção', () => {
  assert.equal(wikiToMarkdown('Ver [[Personagens]].'), `Ver [Personagens](${WIKI}Personagens).`);
  assert.equal(wikiToMarkdown('[[RPG/Mapa|o mapa]]'), `[o mapa](${WIKI}RPG%2FMapa)`);
  assert.equal(wikiToMarkdown('[[Mapa#Norte]]'), `[Mapa#Norte](${WIKI}Mapa)`);
});

test('imagens anexadas viram imagem; outros anexos, link', () => {
  assert.equal(wikiToMarkdown('![[foto 1.png]]'), `![foto 1.png](${WIKI}foto%201.png)`);
  assert.equal(wikiToMarkdown('![[Outra nota]]'), `[Outra nota](${WIKI}Outra%20nota)`);
});

test('tags viram links; títulos, âncoras e números não', () => {
  assert.equal(wikiToMarkdown('Treino #Academia hoje'), `Treino [#Academia](${TAG}academia) hoje`);
  assert.equal(wikiToMarkdown('# Título\n## Sub'), '# Título\n## Sub');
  assert.equal(wikiToMarkdown('[x](https://a.b/#topo) e #1'), '[x](https://a.b/#topo) e #1');
});

test('código fica como está e texto sem links não muda', () => {
  assert.equal(wikiToMarkdown('`[[nao]]` e [[sim]]'), `\`[[nao]]\` e [sim](${WIKI}sim)`);
  assert.equal(wikiToMarkdown('```\n[[bloco]] #tag\n```'), '```\n[[bloco]] #tag\n```');
  assert.equal(wikiToMarkdown('nada aqui [x](https://a.b)'), 'nada aqui [x](https://a.b)');
});

test('frontmatter vira propriedades e banner, fora do corpo da nota', () => {
  const note = frontmatter('---\naliases:\n  - Mata\n  - "Bosque"\ntags: [lugar, mapa]\nbanner: imagens/capa.png\nbanner_y: "40"\n---\n\n# Floresta\n');
  assert.deepEqual(note.properties, [{ key: 'aliases', values: ['Mata', 'Bosque'] }, { key: 'tags', values: ['lugar', 'mapa'] }]);
  assert.deepEqual(note.banner, { target: 'imagens/capa.png', y: 40 });
  assert.equal(note.body, '\n# Floresta\n');
});

test('banner aceita [[imagem]] e posição de 0 a 1; endereço da web e nota sem frontmatter não viram banner', () => {
  assert.deepEqual(frontmatter('---\nbanner: "![[capa.jpg|300]]"\nbanner_y: 0.25\n---\nx').banner, { target: 'capa.jpg', y: 25 });
  assert.equal(frontmatter('---\nbanner: https://exemplo.com/a.png\n---\nx').banner, null);
  assert.deepEqual(frontmatter('# Só texto\n---\nbanner: a.png\n---'), { properties: [], banner: null, body: '# Só texto\n---\nbanner: a.png\n---' });
});

test('anexos do Obsidian: largura em ![[foto|300]], .jfif e PDF embutido; link para PDF não vira nota', () => {
  assert.equal(wikiToMarkdown('![[capa.jpg|300]]'), `![capa.jpg](${WIKI}capa.jpg "300")`);
  assert.equal(wikiToMarkdown('![[mapa.jfif]]'), `![mapa.jfif](${WIKI}mapa.jfif)`);
  assert.equal(wikiToMarkdown('![[Livro do Mestre.pdf]]'), `![Livro do Mestre.pdf](${WIKI}Livro%20do%20Mestre.pdf)`);
  assert.equal(wikiToMarkdown('![[foto.png|legenda]]'), `![legenda](${WIKI}foto.png)`);
  assert.ok(isAttachment('regras.pdf') && isAttachment('a/b.JPEG') && !isAttachment('Nota') && !isAttachment('nota.md'));
  assert.equal(attachmentUrl('vault:/home/x/Cofre', 'imagens/a b.png'), 'athena-file://localhost/vault%3A%2Fhome%2Fx%2FCofre/imagens%2Fa%20b.png');
});

test('vídeos são anexos e arquivos guardados têm endereço próprio, com miniatura opcional', () => {
  assert.ok(isAttachment('sessao.MP4') && isAttachment('clipe.webm#t=3') && !isAttachment('roteiro.txt'));
  assert.equal(wikiToMarkdown('![[clipe.webm]]'), `![clipe.webm](${WIKI}clipe.webm)`);
  assert.equal(storedUrl('18ea-0'), 'iris-file://localhost/18ea-0');
  assert.equal(storedUrl('18ea-0', true), 'iris-file://localhost/18ea-0?miniatura');
  assert.equal(attachmentUrl('ayla', 'a.png', true), 'athena-file://localhost/ayla/a.png?miniatura');
});
