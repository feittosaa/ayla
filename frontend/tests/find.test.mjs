import test from 'node:test';
import assert from 'node:assert/strict';
import { findAll, fold } from '../src/services/Find.ts';

test('busca na nota ignora maiúsculas e acentos e mantém as posições do texto', () => {
  assert.equal(fold('Ação ÚNICA'), 'acao unica');
  assert.equal(fold('Ação').length, 'Ação'.length);
  const text = 'O Reino do Sol e o reino das serpentes; REINO, réino.';
  assert.deepEqual(findAll(text, 'reino').map(([a, b]) => text.slice(a, b)), ['Reino', 'reino', 'REINO', 'réino']);
  assert.deepEqual(findAll(text, '   '), []);
  assert.deepEqual(findAll('aaaa', 'aa'), [[0, 2], [2, 4]], 'sem sobreposição');
  assert.equal(findAll('x'.repeat(10), 'x', 3).length, 3);
  assert.deepEqual(findAll('😀 ação', 'acao'), [[3, 7]], 'emoji (duas unidades) não desloca');
});
