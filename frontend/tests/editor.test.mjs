import test from 'node:test';
import assert from 'node:assert/strict';
import { clock, cropPixels, dragCrop, fitCrop, fractionRatio, isFull, rotated } from '../src/services/Editor.ts';

const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} ≠ ${b}`);

test('recorte na proporção, em pixels, no quadro girado', () => {
  assert.deepEqual(rotated(400, 300, 90), { width: 300, height: 400 });
  assert.deepEqual(fitCrop('livre', 400, 300), { x: 0, y: 0, width: 1, height: 1 });
  // 1:1 numa imagem 400×300: 300×300 centralizado.
  const square = fitCrop('1:1', 400, 300);
  const px = cropPixels(square, 400, 300);
  assert.deepEqual(px, { width: 300, height: 300 });
  near(square.x, 50 / 400);
  // 16:9 numa imagem em pé vira 9:16.
  const tall = cropPixels(fitCrop('16:9', 900, 1600), 900, 1600);
  assert.deepEqual(tall, { width: 900, height: 1600 });
  assert.equal(fractionRatio('livre', 1, 1), null);
});

test('arrastar mantém o recorte dentro e a proporção', () => {
  const start = { x: 0.25, y: 0.25, width: 0.5, height: 0.5 };
  assert.deepEqual(dragCrop(start, 'move', 0.9, -0.9, null), { x: 0.5, y: 0, width: 0.5, height: 0.5 });
  const free = dragCrop(start, 'se', 0.1, 0.2, null);
  near(free.width, 0.6); near(free.height, 0.7); near(free.x, 0.25);
  // Canto noroeste com proporção 2: a largura manda e o canto sudeste fica parado.
  const ratio = dragCrop(start, 'nw', -0.1, 0, 2);
  near(ratio.width, 0.6); near(ratio.height, 0.3); near(ratio.x + ratio.width, 0.75); near(ratio.y + ratio.height, 0.75);
  // Não passa da borda.
  const big = dragCrop(start, 'se', 5, 5, 1);
  assert.ok(big.x + big.width <= 1 + 1e-9 && big.y + big.height <= 1 + 1e-9);
  near(big.width, big.height);
  assert.ok(isFull(null) && isFull({ x: 0, y: 0, width: 1, height: 1 }) && !isFull(start));
});

test('tempo do vídeo', () => {
  assert.equal(clock(65.34), '1:05.3');
  assert.equal(clock(3725), '1:02:05.0');
  assert.equal(clock(-1), '0:00.0');
});

test('bordas: um lado só no Livre, os dois na proporção', () => {
  const start = { x: 0.25, y: 0.25, width: 0.5, height: 0.5 };
  const east = dragCrop(start, 'e', 0.1, 0.3, null);
  near(east.width, 0.6); near(east.height, 0.5); near(east.x, 0.25); near(east.y, 0.25);
  const north = dragCrop(start, 'n', 0.3, -0.5, null);
  near(north.y, 0); near(north.height, 0.75); near(north.x, 0.25); near(north.width, 0.5);
  const west = dragCrop(start, 'w', 5, 0, null);
  near(west.width, 0.02); near(west.x + west.width, 0.75);
  // Proporção 1: puxar a borda direita cresce a altura em volta do centro, até a borda da imagem.
  const square = dragCrop(start, 'e', 0.2, 0, 1);
  near(square.width, 0.7); near(square.height, 0.7); near(square.y, 0.15);
  const capped = dragCrop(start, 's', 0, 0.25, 1);
  near(capped.height, 0.75); near(capped.width, 0.75); near(capped.x, 0.125);
});
