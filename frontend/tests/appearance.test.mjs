import test from 'node:test';
import assert from 'node:assert/strict';
import { ACCENTS, BACKGROUNDS, DEFAULT_APPEARANCE, contrast, luminance, palette, safeBackground } from '../src/services/Palette.ts';

const surfaces = ['--ayla-bg', '--ayla-sidebar', '--ayla-panel', '--ayla-control'];
const solid = (value) => value.startsWith('#') ? value : null;

test('o padrão reproduz o tema escuro original', () => {
  const p = palette(DEFAULT_APPEARANCE, 'escuro');
  assert.equal(p['--ayla-bg'], '#111214');
  assert.equal(p['--ayla-accent'], '#ff9f43');
  assert.equal(p['--ayla-text'], '#ececee');
  assert.equal(p['--ayla-round'], '1');
  assert.equal(p['color-scheme'], 'dark');
});

test('texto legível (≥ 4,5:1) em todos os fundos prontos e destaques, nos dois modos', () => {
  for (const mode of ['escuro', 'claro']) for (const bg of BACKGROUNDS[mode]) for (const accent of ACCENTS) {
    const key = mode === 'escuro' ? 'dark_background' : 'light_background';
    const p = palette({ ...DEFAULT_APPEARANCE, accent, [key]: bg.color }, mode);
    for (const surface of surfaces) {
      const over = solid(p[surface]);
      assert.ok(contrast(p['--ayla-text'], over) >= 4.5, `${mode} ${bg.name} texto em ${surface}`);
    }
    assert.ok(contrast(p['--ayla-text-soft'], solid(p['--ayla-sidebar'])) >= 4.5, `${mode} ${bg.name} texto secundário`);
    assert.ok(contrast(p['--ayla-text-faint'], solid(p['--ayla-sidebar'])) >= 4.5, `${mode} ${bg.name} texto fraco`);
    assert.ok(contrast(p['--ayla-accent-soft'], p['--ayla-bg']) >= 4.5, `${mode} ${bg.name} destaque como texto (${accent})`);
    assert.ok(contrast(p['--ayla-on-accent'], p['--ayla-accent']) >= 3, `${mode} texto sobre o destaque ${accent}`);
  }
});

test('cor livre de fundo é puxada para a faixa do modo, mantendo o tom', () => {
  const dark = safeBackground('#4080ff', 'escuro');
  assert.ok(luminance(dark) <= 0.045, dark);
  const [r, , b] = [1, 3, 5].map(i => parseInt(dark.slice(i, i + 2), 16));
  assert.ok(b > r, 'continua azulado');
  assert.ok(luminance(safeBackground('#202020', 'claro')) >= 0.78);
  assert.equal(safeBackground('lixo', 'escuro'), DEFAULT_APPEARANCE.dark_background);
});

test('transparência só afeta o fundo da janela e as laterais; quinas seguem a escala', () => {
  const p = palette({ ...DEFAULT_APPEARANCE, opacity: 70, roundness: 0 }, 'escuro');
  assert.equal(p['--ayla-window'], 'rgb(17 18 20 / 70%)');
  assert.match(p['--ayla-sidebar'], /\/ 70%\)$/);
  assert.equal(p['--ayla-bg'], '#111214', 'cabeçalhos e abas continuam sólidos');
  assert.ok(p['--ayla-panel'].startsWith('#'));
  assert.equal(p['--ayla-round'], '0');
});
