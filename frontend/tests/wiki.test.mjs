import test from 'node:test';
import assert from 'node:assert/strict';
import { TAG, WIKI, wikiToMarkdown } from '../src/services/Wiki.ts';

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
