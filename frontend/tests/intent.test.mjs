import test from 'node:test';
import assert from 'node:assert/strict';
import { asksForImage } from '../src/services/Intent.ts';

test('pedidos explícitos de imagem são reconhecidos', () => {
  for (const text of ['gere uma imagem de um gato astronauta', 'Crie uma ilustração do Recife', 'desenhe um logo para a padaria', 'Por favor, faça uma imagem do pôr do sol', 'gerar foto de um bolo']) {
    assert.equal(asksForImage(text), true, text);
  }
});

test('conversas comuns não viram pedido de imagem', () => {
  for (const text of ['o que é uma imagem vetorial?', 'me explica como gerar renda extra', 'crie uma lista de compras', 'faça um resumo da reunião', 'imagem de perfil do whatsapp não carrega']) {
    assert.equal(asksForImage(text), false, text);
  }
});
