import test from 'node:test';
import assert from 'node:assert/strict';
import { changeCase, joinLines, position, remember, sortLines, untitledName } from '../src/pad/textOps.ts';

test('maiúsculas, minúsculas, inverter e primeira maiúscula respeitam acentos', () => {
  assert.equal(changeCase('ação é ótima', 'upper'), 'AÇÃO É ÓTIMA');
  assert.equal(changeCase('AÇÃO', 'lower'), 'ação');
  assert.equal(changeCase('Ação', 'invert'), 'aÇÃO');
  assert.equal(changeCase('olá mundo-novo d\'água', 'title'), 'Olá Mundo-Novo D\'água');
});

test('ordenar linhas mantém a quebra final e ordena números e acentos como gente', () => {
  assert.equal(sortLines('pera\nÁgua\nbanana\n'), 'Água\nbanana\npera\n');
  assert.equal(sortLines('item 10\nitem 2'), 'item 2\nitem 10');
  assert.equal(sortLines('b\na\nb', { unique: true }), 'a\nb');
  assert.equal(sortLines('a\nc\nb', { descending: true }), 'c\nb\na');
});

test('juntar linhas tira recuos e linhas vazias', () => {
  assert.equal(joinLines('primeira  \n    segunda\n\n  terceira'), 'primeira segunda terceira');
});

test('nomes de documentos novos não se repetem', () => {
  assert.equal(untitledName([]), 'Novo documento');
  assert.equal(untitledName(['Novo documento', 'Novo documento 2']), 'Novo documento 3');
});

test('coluna conta tabs como o editor e recentes não repetem', () => {
  assert.equal(position('\tab', 2, 8), 10);
  assert.equal(position('abc', 0, 8), 1);
  assert.deepEqual(remember(['/a', '/b'], '/b'), ['/b', '/a']);
  assert.equal(remember(Array.from({ length: 12 }, (_, i) => `/${i}`), '/novo').length, 10);
});
