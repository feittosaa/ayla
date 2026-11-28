import test from 'node:test';
import assert from 'node:assert/strict';
import { TAG, WIKI, attachmentUrl, changeBanner, setMediaVersions, frontmatter, noteAliases, removeBanner, setAliases, embedResolver, imageWidth, isAttachment, resizedImage, storedUrl, wikiToMarkdown } from '../src/services/Wiki.ts';

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
  assert.deepEqual(note.properties, [{ key: 'tags', values: ['lugar', 'mapa'] }]);
  assert.deepEqual(note.aliases, ['Mata', 'Bosque'], 'apelidos à parte, editáveis');
  assert.deepEqual(note.banner, { target: 'imagens/capa.png', y: 40 });
  assert.equal(note.body, '\n# Floresta\n');
});

test('banner aceita [[imagem]], posição em porcentagem como o make-md e a fração antiga da Ayla; endereço da web e nota sem frontmatter não viram banner', () => {
  assert.deepEqual(frontmatter('---\nbanner: "![[capa.jpg|300]]"\nbanner_y: 0.25\n---\nx').banner, { target: 'capa.jpg', y: 25 });
  assert.deepEqual(frontmatter('---\nbanner: capa.jpg\nbanner_y: "0.5"\n---\nx').banner, { target: 'capa.jpg', y: 0.5 }, 'make-md: "0.5" é 0,5%');
  assert.deepEqual(frontmatter('---\nbanner: capa.jpg\nbanner_y: "1"\n---\nx').banner, { target: 'capa.jpg', y: 1 });
  assert.deepEqual(frontmatter('---\nbanner: capa.jpg\nbanner_y: 1\n---\nx').banner, { target: 'capa.jpg', y: 1 });
  assert.equal(frontmatter('---\nbanner: https://exemplo.com/a.png\n---\nx').banner, null);
  assert.deepEqual(frontmatter('# Só texto\n---\nbanner: a.png\n---'), { properties: [], aliases: [], banner: null, body: '# Só texto\n---\nbanner: a.png\n---' });
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

test('largura da imagem é salva na sintaxe do anexo ou no título Markdown', () => {
  assert.equal(resizedImage('![[mapa.png|200]]', 'mapa.png', 'mapa.png', '200', 320), '![[mapa.png|320]]');
  const plain = '![Vista](fotos/vista.png "Serra")';
  assert.equal(resizedImage(plain, 'fotos/vista.png', 'Vista', 'Serra', 310), '![Vista](<fotos/vista.png> "Serra [largura=310]")');
  assert.equal(imageWidth('Serra [largura=310]'), '310');
});

test('ajustes do banner preservam o corpo e as outras propriedades do cofre', () => {
  const text = '---\r\nbanner: Imagens/Coruja.jpg\r\nbanner_y: "45"\r\ntags: [fera]\r\n---\r\n[[Feras|Fera]]';
  const resized = changeBanner(changeBanner(text, 'banner_height', 320), 'banner_y', 1);
  assert.equal(resized, '---\r\nbanner: Imagens/Coruja.jpg\r\nbanner_y: "1"\r\ntags: [fera]\r\nbanner_height: 320\r\n---\r\n[[Feras|Fera]]');
  assert.equal(changeBanner(text, 'banner_y', 37.25).split('\r\n')[2], 'banner_y: "37.3"');
  assert.deepEqual(frontmatter(resized), { banner: { target: 'Imagens/Coruja.jpg', y: 1, height: 320 }, properties: [{ key: 'tags', values: ['fera'] }], aliases: [], body: '[[Feras|Fera]]' });
});

test('texto que começa com --- (separador) não vira propriedades nem esconde o começo da nota', () => {
  const text = '---\njogo de luta rogue like\ncartas de habilidades\n\nmain game (modo historia) - rogue like\n---\n10 - composite\n';
  assert.deepEqual(frontmatter(text), { properties: [], aliases: [], banner: null, body: text });
  assert.equal(changeBanner(text, 'banner_y', 30), text);
  const valid = frontmatter('---\n# comentário\ntags:\n- rpg\naliases: [a, b]\n---\nCorpo');
  assert.deepEqual(valid.properties, [{ key: 'tags', values: ['rpg'] }]);
  assert.deepEqual(valid.aliases, ['a', 'b']);
  assert.equal(valid.body, 'Corpo');
});

test('![[nome]] sem extensão acha o anexo da raiz, imagem primeiro, e só se for um', () => {
  const embed = embedResolver([{ path: 'Imagens/Casco de Cinzas.png' }, { path: 'Fichas/Casco de Cinzas.pdf' }, { path: 'Notas/Casco de Cinzas.md' }, { path: 'a/Mapa.jpg' }, { path: 'b/Mapa.webp' }]);
  assert.equal(embed('Casco de Cinzas'), 'Imagens/Casco de Cinzas.png');
  assert.equal(embed('casco de cinzas'), 'Imagens/Casco de Cinzas.png');
  assert.equal(embed('Mapa'), undefined, 'dois com o mesmo nome: não adivinha');
  assert.equal(embed('Casco de Cinzas.png'), undefined, 'com extensão segue o caminho normal');
  assert.equal(embed('Bohr'), undefined);
  assert.match(wikiToMarkdown('![[Casco de Cinzas|300]] e [[Casco de Cinzas]]', embed), /^!\[Casco de Cinzas\]\(athena-link:Imagens%2FCasco%20de%20Cinzas\.png "300"\) e \[Casco de Cinzas\]\(athena-link:Casco%20de%20Cinzas\)$/);
});

test('anexo trocado no lugar ganha endereço novo (data de alteração), pelo caminho ou pelo nome', () => {
  const before = attachmentUrl('cofre', 'Imagens/Mapas/Krakatoa.png');
  setMediaVersions('cofre', [{ path: 'Imagens/Mapas/Krakatoa.png', modified: 100 }]);
  const v1 = attachmentUrl('cofre', 'Imagens/Mapas/Krakatoa.png');
  assert.notEqual(v1, before);
  assert.ok(v1.endsWith('?v=100'));
  assert.ok(attachmentUrl('cofre', 'krakatoa.png', true).endsWith('?miniatura&v=100'), 'pelo nome, como o ![[Krakatoa.png]]');
  setMediaVersions('cofre', [{ path: 'Imagens/Mapas/Krakatoa.png', modified: 200 }]);
  assert.ok(attachmentUrl('cofre', 'Imagens/Mapas/Krakatoa.png').endsWith('?v=200'));
  assert.ok(!attachmentUrl('outro', 'Imagens/Mapas/Krakatoa.png').includes('v='), 'cada raiz com as suas datas');
});

test('apelidos: lidos de aliases/alias, criados, editados e removidos no topo da nota', () => {
  const nota = '---\ntags: [npc]\naliases:\n  - Kael\n  - "Kûarasy"\ncriado: 2026\n---\n# Kael\ntexto';
  assert.deepEqual(noteAliases(nota), ['Kael', 'Kûarasy']);
  assert.deepEqual(frontmatter(nota).properties.map(p => p.key), ['tags', 'criado'], 'apelidos saem da lista comum');
  const editada = setAliases(nota, ['Kael', 'O Caçador', 'Kael']);
  assert.equal(editada, '---\ntags: [npc]\naliases:\n  - Kael\n  - O Caçador\ncriado: 2026\n---\n# Kael\ntexto');
  assert.equal(setAliases(nota, []), '---\ntags: [npc]\ncriado: 2026\n---\n# Kael\ntexto');
  // Sem propriedades: cria o bloco; com só o apelido, removê-lo tira o bloco.
  const nova = setAliases('# Nota\r\ncorpo', ['Pescador: o velho']);
  assert.equal(nova, '---\r\naliases:\r\n  - "Pescador: o velho"\r\n---\r\n# Nota\r\ncorpo');
  assert.deepEqual(noteAliases(nova), ['Pescador: o velho']);
  assert.equal(setAliases(nova, []), '# Nota\r\ncorpo');
  // `alias` antigo em linha vira `aliases`.
  assert.equal(setAliases('---\nalias: Velho\n---\nx', ['Velho', 'Tomás']), '---\naliases:\n  - Velho\n  - Tomás\n---\nx');
});

test('banner removido: só as chaves do banner saem; sem outras propriedades, o bloco sai', () => {
  const nota = '---\nbanner: Imagens/capa.png\nbanner_y: "40"\ntags: [npc]\nbanner_height: 320\n---\n\n# Kael';
  assert.equal(removeBanner(nota), '---\ntags: [npc]\n---\n\n# Kael');
  assert.equal(removeBanner('---\r\nbanner: a.png\r\nbanner_y: "10"\r\n---\r\n\r\n# Só'), '# Só', 'desfaz o bloco criado pelo banner');
  // Trocar a imagem: a posição antiga sai e a altura fica.
  assert.equal(removeBanner(nota, ['banner_y']), '---\nbanner: Imagens/capa.png\ntags: [npc]\nbanner_height: 320\n---\n\n# Kael');
  assert.equal(removeBanner('# sem propriedades'), '# sem propriedades');
});
