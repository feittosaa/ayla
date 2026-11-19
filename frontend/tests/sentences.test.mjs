import test from 'node:test';
import assert from 'node:assert/strict';
import { splitSentences } from '../src/services/Sentences.ts';

test('frases completas saem durante o streaming e o resto espera', () => {
  assert.deepEqual(splitSentences('Oi! Tudo bem? Hoje vamos'), { sentences: ['Oi!', 'Tudo bem?'], rest: ' Hoje vamos' });
  assert.deepEqual(splitSentences('Custa R$ 87,90. Pronto'), { sentences: ['Custa R$ 87,90.'], rest: ' Pronto' });
  assert.deepEqual(splitSentences('Versão 3.5 saiu'), { sentences: [], rest: 'Versão 3.5 saiu' });
  assert.deepEqual(splitSentences('- item um\n- item dois\n'), { sentences: ['- item um', '- item dois'], rest: '' });
  assert.deepEqual(splitSentences('Espere... ok.'), { sentences: ['Espere...', 'ok.'], rest: '' });
});
