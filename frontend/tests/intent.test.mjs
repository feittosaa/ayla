import test from 'node:test';
import assert from 'node:assert/strict';
import { asksForImage, asksAboutScreen } from '../src/services/Intent.ts';

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

test('pedidos para olhar a tela são reconhecidos', () => {
  for (const text of ['qual o modelo desse carro na tela?', 'veja minha tela e me diga o erro', 'o que está aparecendo na tela?', 'Olha a tela: quem é esse ator?', 'descreva o que tem na minha tela']) {
    assert.equal(asksAboutScreen(text), true, text);
  }
});

test('conversas comuns não acionam a captura de tela', () => {
  for (const text of ['como mudar o brilho do monitor?', 'crie uma tela de login em React', 'qual o melhor carro de 2025?', 'tela azul no windows, o que fazer?']) {
    assert.equal(asksAboutScreen(text), false, text);
  }
});
