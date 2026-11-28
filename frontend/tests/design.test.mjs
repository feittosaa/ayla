import test from 'node:test';
import assert from 'node:assert/strict';
import { History, clampSide, designData, designName, designPages, exportScale, fontName, fontValue, formatOf, formats, gradientCoords, pdfPage, previewScale, templates } from '../src/services/Design.ts';

test('A4 do design vira a folha A4 do PDF e outras páginas seguem 96 px por polegada', () => {
  const a4 = formatOf('a4');
  assert.deepEqual(pdfPage(a4.width, a4.height), [595.5, 842.25]);
  assert.deepEqual(pdfPage(1080, 1080), [810, 810]);
});

test('exportação: PDF em ~300 dpi, imagens com 1920 px no lado maior, sem passar de 8000 px', () => {
  assert.equal(exportScale(794, 1123, 'pdf'), 3);
  assert.equal(exportScale(794, 1123, 'png'), 2);
  assert.equal(exportScale(1080, 1080, 'jpg'), 2);
  assert.equal(exportScale(1920, 1080, 'png'), 1);
  assert.equal(exportScale(4000, 3000, 'pdf'), 2);
  assert.equal(exportScale(8000, 100, 'pdf'), 1);
  assert.ok(previewScale(1080, 1920) * 1920 <= 400 && previewScale(100, 100) === 1);
});

test('lados ficam entre 16 e 8000 px', () => {
  assert.equal(clampSide(3), 16);
  assert.equal(clampSide(99999), 8000);
  assert.equal(clampSide(Number.NaN), 16);
  assert.equal(clampSide(1080.4), 1080);
});

test('gradiente cobre o retângulo inteiro no ângulo pedido', () => {
  assert.deepEqual(gradientCoords(0, 200, 100), { x1: 0, y1: 50, x2: 200, y2: 50 });
  assert.deepEqual(gradientCoords(90, 200, 100), { x1: 100, y1: 0, x2: 100, y2: 100 });
  const diagonal = gradientCoords(45, 100, 100);
  assert.ok(Math.abs(diagonal.x1) < 0.01 && Math.abs(diagonal.y2 - 100) < 0.01);
});

test('histórico: desfazer, refazer, nova alteração apaga o futuro e limite de fotos', () => {
  const h = new History('a', 3);
  assert.equal(h.push('a'), false);
  h.push('b'); h.push('c');
  assert.equal(h.undo(), 'b');
  assert.equal(h.undo(), 'a');
  assert.equal(h.undo(), null);
  assert.equal(h.redo(), 'b');
  h.push('x');
  assert.equal(h.canRedo, false);
  for (const v of ['1', '2', '3', '4']) h.push(v);
  let steps = 0; while (h.undo() !== null) steps++;
  assert.equal(steps, 3);
});

test('fontes: valor com reserva genérica e nome de volta', () => {
  assert.equal(fontValue('Playfair Display'), 'Playfair Display, serif');
  assert.equal(fontValue('monospace'), 'monospace');
  assert.equal(fontName('Playfair Display, serif'), 'Playfair Display');
  assert.equal(fontName("'Inter', sans-serif"), 'Inter');
});

test('modelos usam formatos existentes, fontes conhecidas e ficam dentro da página', () => {
  const ids = new Set();
  for (const t of templates) {
    assert.ok(!ids.has(t.id)); ids.add(t.id);
    const format = formatOf(t.format);
    assert.ok(format, t.id);
    for (const o of t.objects) {
      if (o.type === 'text') {
        assert.ok(fontValue(o.font).includes(','), `${t.id}: ${o.font}`);
        assert.ok(o.left >= 0 && o.left + o.width <= format.width && o.top < format.height, `${t.id}: ${o.text}`);
      }
    }
  }
  assert.ok(formats.every(f => f.width >= 16 && f.height >= 16));
});

test('guias: encaixa no centro, na margem e na borda de outro elemento, e mostra as linhas', async () => {
  const { marginOf, snap, snapLines } = await import('../src/services/DesignGuides.ts');
  assert.equal(marginOf(1080, 1080), 54);
  const lines = snapLines(1000, 800, [{ left: 600, top: 100, width: 200, height: 100 }]);
  // Centro horizontal da página: 1000/2 = 500; o meio do elemento (100 + 400/2... ) cai a 3 px.
  const centered = snap({ left: 303, top: 300, width: 400, height: 150 }, lines, 6);
  assert.equal(centered.dx, -3);
  assert.ok(centered.guides.x.includes(500));
  // Margem (40 px) na vertical e nada longe do limite.
  const margin = snap({ left: 120, top: 44, width: 50, height: 50 }, lines, 6);
  assert.equal(margin.dy, -4);
  assert.deepEqual(margin.guides.y, [40]);
  assert.equal(margin.dx, 0);
  // Borda esquerda de outro elemento.
  const edge = snap({ left: 598, top: 500, width: 40, height: 40 }, lines, 6);
  assert.equal(edge.dx, 2);
  assert.ok(edge.guides.x.includes(600));
  assert.deepEqual(snap({ left: 120, top: 300, width: 10, height: 10 }, lines, 6), { dx: 0, dy: 0, guides: { x: [], y: [] } });
});

test('rascunho da Ayla: reconhece só o formato da conversa', async () => {
  const { draftOf } = await import('../src/services/Design.ts');
  const draft = { format: 'post', width: 1080, height: 1080, background: '#fff', objects: [{ type: 'icon', svg: '<svg/>', left: 0, top: 0, size: 50, color: '#000' }] };
  assert.deepEqual(draftOf(JSON.stringify({ ayla_draft: draft })), draft);
  assert.equal(draftOf(JSON.stringify({ objects: [], version: '7.0.0' })), null, 'projeto do editor não é rascunho');
  assert.equal(draftOf(JSON.stringify({ ayla_draft: { ...draft, width: 0 } })), null);
  assert.equal(draftOf('{quebrado'), null);
});

test('páginas do design: uma página é o próprio JSON (como os antigos); várias vão numa lista', () => {
  const a = JSON.stringify({ version: '7.0.0', objects: [] });
  const b = JSON.stringify({ version: '7.0.0', objects: [{ type: 'Rect' }] });
  assert.equal(designData([a]), a, 'design de uma página continua legível por quem lê o JSON direto');
  assert.deepEqual(designPages(designData([a, b])), [a, b]);
  assert.deepEqual(designPages(a), [a]);
  assert.deepEqual(designPages(JSON.stringify({ ayla_paginas: [] })), [JSON.stringify({ ayla_paginas: [] })], 'lista vazia não vira design sem páginas');
  assert.deepEqual(designPages('{quebrado'), ['{quebrado']);
  assert.equal(designName('   '), 'Sem nome');
  assert.equal(designName(' Cartaz '), 'Cartaz');
});

test('redimensionar encaixa a borda puxada na margem (sangria), na borda e em outro elemento', async () => {
  const { snapLines, scaleSnap } = await import('../src/services/DesignGuides.ts');
  const lines = snapLines(1000, 1000, [{ left: 600, top: 600, width: 100, height: 100 }]); // margem 50
  // Canto de baixo à direita (proporcional): a direita em 947 encaixa na margem 950.
  const a = scaleSnap({ left: 100, top: 100, width: 847, height: 380 }, { right: true, bottom: true }, lines, 6, true);
  assert.ok(Math.abs(a.fx - 850 / 847) < 1e-9 && a.fx === a.fy);
  assert.deepEqual(a.guides.x, [950]);
  // Lado esquerdo sozinho: a esquerda em 52 vai para a margem 50; só o eixo x muda.
  const b = scaleSnap({ left: 52, top: 300, width: 200, height: 80 }, { left: true }, lines, 6, false);
  assert.equal(b.fx, 202 / 200); assert.equal(b.fy, 1);
  // Base encosta no topo do outro elemento (600).
  const c = scaleSnap({ left: 100, top: 100, width: 300, height: 497 }, { bottom: true }, lines, 6, false);
  assert.equal(c.fy, 500 / 497); assert.deepEqual(c.guides.y, [600]);
  // Longe de tudo: nada muda.
  const d = scaleSnap({ left: 100, top: 100, width: 220, height: 220 }, { right: true, bottom: true }, lines, 6, true);
  assert.deepEqual([d.fx, d.fy, d.guides], [1, 1, { x: [], y: [] }]);
});

test('apresentação: páginas deitadas em 16:9 ou 4:3 (slides exportados em PDF também)', async () => {
  const { isPresentation, formatOf } = await import('../src/services/Design.ts');
  const f = formatOf('apresentacao');
  assert.ok(isPresentation([{ largura: f.width, altura: f.height }]));
  assert.ok(isPresentation([{ largura: 1920, altura: 1080 }, { largura: 1600, altura: 1200 }]), '4:3 também');
  assert.ok(isPresentation([{ largura: 1920, altura: 1081 }]), 'arredondamento do PDF');
  assert.ok(!isPresentation([{ largura: 1080, altura: 1080 }]));
  assert.ok(!isPresentation([{ largura: 1920, altura: 1080 }, { largura: 794, altura: 1123 }]), 'uma página em pé tira a faixa');
  assert.ok(!isPresentation([]));
});

test('tamanho livre em cm e mm: 96 px por polegada, a mesma folha A4 do formato pronto', async () => {
  const { toPx, fromPx, formatOf } = await import('../src/services/Design.ts');
  const a4 = formatOf('a4');
  assert.equal(Math.round(toPx(210, 'mm')), a4.width);
  assert.equal(Math.round(toPx(29.7, 'cm')), a4.height);
  assert.equal(fromPx(a4.width, 'mm'), 210.1, 'uma casa');
  assert.equal(fromPx(1200.4, 'px'), 1200);
  assert.equal(fromPx(toPx(10, 'cm'), 'mm'), 100);
});
