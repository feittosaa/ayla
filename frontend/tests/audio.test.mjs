import test from 'node:test';
import assert from 'node:assert/strict';
import { encodeWav, resample } from '../src/services/Audio.ts';

test('WAV PCM16 mono 16 kHz com cabeçalho e amostras corretos', () => {
  const wav = encodeWav(new Float32Array([0, 1, -1, 2]), 16000);
  const view = new DataView(wav.buffer);
  assert.equal(new TextDecoder().decode(wav.slice(0, 4)), 'RIFF');
  assert.equal(view.getUint32(24, true), 16000);
  assert.equal(view.getUint16(22, true), 1);
  assert.equal(view.getUint32(40, true), 8);
  assert.deepEqual([0, 1, 2, 3].map(i => view.getInt16(44 + i * 2, true)), [0, 32767, -32767, 32767]);
});

test('reamostragem de 48 kHz para 16 kHz preserva duração e forma', () => {
  const input = new Float32Array(48000).map((_, i) => Math.sin(i / 10));
  const output = resample(input, 48000);
  assert.equal(output.length, 16000);
  assert.ok(Math.abs(output[100] - input[300]) < 1e-6);
  assert.equal(resample(input, 16000), input);
});
