import test from 'node:test';
import assert from 'node:assert/strict';
import { NoteGraphSimulation } from '../src/services/NoteGraphLayout.ts';

const keys = count => Array.from({ length: count }, (_, i) => `n${i}`);
const settle = sim => { while (sim.running) sim.tick(); return sim; };
const distance = (sim, a, b) => Math.hypot(sim.x[a] - sim.x[b], sim.y[a] - sim.y[b]);

test('simulação assenta com notas ligadas mais próximas que as soltas', () => {
  const edges = [[0, 1], [0, 2], [0, 3], [0, 4], [4, 5], [4, 6]];
  const sim = settle(new NoteGraphSimulation(keys(9), edges));
  const again = settle(new NoteGraphSimulation(keys(9), edges));
  assert.deepEqual([...sim.x], [...again.x], 'mesmos dados, mesmo arranjo');
  assert.deepEqual(sim.neighbors[0], [1, 2, 3, 4]);
  assert.ok([...sim.x, ...sim.y].every(Number.isFinite));
  const linked = edges.map(([a, b]) => distance(sim, a, b));
  assert.ok(linked.every(d => d > 8 && d < 90), `links: ${linked}`);
  const average = linked.reduce((a, b) => a + b) / linked.length;
  assert.ok(distance(sim, 0, 7) > average && distance(sim, 0, 8) > average, 'notas soltas ficam em volta');
  for (let a = 0; a < 9; a++) for (let b = a + 1; b < 9; b++) assert.ok(distance(sim, a, b) > 4, `${a} e ${b} sobrepostos`);
  const { minX, maxX } = sim.bounds();
  assert.ok(maxX - minX < 400, 'notas soltas não fogem do conjunto');
});

test('posições conhecidas são preservadas e notas novas nascem perto da vizinha', () => {
  const first = settle(new NoteGraphSimulation(keys(4), [[0, 1], [1, 2]]));
  const saved = first.positions(keys(4));
  const next = new NoteGraphSimulation(keys(5), [[0, 1], [1, 2], [2, 4]], saved);
  assert.ok(next.alpha < 0.5, 'atualização só reajusta de leve');
  assert.deepEqual(next.point(1), saved.get('n1'));
  assert.ok(!new NoteGraphSimulation(keys(4), [[0, 1], [1, 2]], saved).running, 'mesmas notas: nada se move');
  assert.ok(distance(next, 2, 4) < 20);
});

test('nota arrastada fica onde o ponteiro está e puxa as ligadas', () => {
  const sim = settle(new NoteGraphSimulation(keys(3), [[0, 1], [1, 2]]));
  const before = distance(sim, 0, 1);
  sim.heat(0.3);
  sim.pin(0, { x: 200, y: 0 });
  for (let i = 0; i < 120; i++) sim.tick();
  assert.deepEqual(sim.point(0), { x: 200, y: 0 });
  assert.ok(Math.abs(distance(sim, 0, 1) - before) < 30, 'a vizinha acompanha');
  sim.release(0); sim.heat(0);
  settle(sim);
  assert.ok(!sim.running);
});

test('grafo vazio, uma nota e pontos coincidentes não quebram', () => {
  assert.equal(settle(new NoteGraphSimulation([], [])).count, 0);
  const single = settle(new NoteGraphSimulation(['a'], []));
  assert.ok(Math.abs(single.x[0]) < 20 && Math.abs(single.y[0]) < 20);
  const same = new Map(keys(4).map(k => [k, { x: 3, y: 3 }]));
  const stacked = settle(new NoteGraphSimulation(keys(5), [], same));
  assert.ok([...stacked.x, ...stacked.y].every(Number.isFinite));
  assert.ok(new Set([...stacked.x].map(v => v.toFixed(3))).size > 1, 'pontos empilhados se separam');
});
