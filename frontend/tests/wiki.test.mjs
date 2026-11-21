import test from 'node:test';
import assert from 'node:assert/strict';
import { WIKI, wikiToMarkdown } from '../src/services/Wiki.ts';

test('links do Obsidian viram links internos, com apelido e seção', () => {
  assert.equal(wikiToMarkdown('Ver [[Personagens]].'), `Ver [Personagens](${WIKI}Personagens).`);
  assert.equal(wikiToMarkdown('[[RPG/Mapa|o mapa]]'), `[o mapa](${WIKI}RPG%2FMapa)`);
  assert.equal(wikiToMarkdown('[[Mapa#Norte]]'), `[Mapa#Norte](${WIKI}Mapa)`);
  assert.equal(wikiToMarkdown('![[foto.png]]'), `[foto.png](${WIKI}foto.png)`);
});

test('código fica como está e texto sem links não muda', () => {
  assert.equal(wikiToMarkdown('`[[nao]]` e [[sim]]'), `\`[[nao]]\` e [sim](${WIKI}sim)`);
  assert.equal(wikiToMarkdown('```\n[[bloco]]\n```'), '```\n[[bloco]]\n```');
  assert.equal(wikiToMarkdown('nada aqui [x](https://a.b)'), 'nada aqui [x](https://a.b)');
});
