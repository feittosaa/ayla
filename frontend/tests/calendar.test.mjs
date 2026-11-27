import test from 'node:test';
import assert from 'node:assert/strict';
import { alertFrom, alertMinutes, alertsLabel, maskDate, maskTime, maskedEdit, parseDate, parseTime, showDate } from '../src/services/Calendar.ts';

test('avisos viram minutos e voltam na unidade natural', () => {
  assert.equal(alertMinutes({ unit: 'na-hora', amount: 0 }), 0);
  assert.equal(alertMinutes({ unit: 'horas', amount: 2 }), 120);
  assert.equal(alertMinutes({ unit: 'dias', amount: 3 }), 4320);
  assert.equal(alertMinutes({ unit: 'horas', amount: 0 }), null, 'zero horas antes não vale');
  assert.equal(alertMinutes({ unit: 'dias', amount: 400 }), null, 'acima de 365 dias');
  assert.equal(alertMinutes({ unit: 'horas', amount: 1.5 }), null);
  assert.deepEqual(alertFrom(120), { unit: 'horas', amount: 2 });
  assert.deepEqual(alertFrom(90), { unit: 'minutos', amount: 90 });
  assert.deepEqual(alertFrom(2880), { unit: 'dias', amount: 2 });
  assert.equal(alertsLabel([0, 60]), '1 hora antes e na hora');
  assert.equal(alertsLabel([1440, 0, 180]), '1 dia antes, 3 horas antes e na hora');
  assert.equal(alertsLabel([]), '');
});

test('hora em 24 h e data em DD/MM/AAAA', () => {
  assert.equal(parseTime('9'), '09:00');
  assert.equal(parseTime('930'), '09:30');
  assert.equal(parseTime('21:05'), '21:05');
  assert.equal(parseTime('14h30'), '14:30');
  assert.equal(parseTime('24:00'), null);
  assert.equal(parseTime('12:60'), null);
  assert.equal(maskTime('1530'), '15:30');
  assert.equal(parseDate('23/11/2026'), '2026-11-23');
  assert.equal(parseDate('5/1/2027'), '2027-01-05');
  assert.equal(parseDate('31/02/2026'), null, 'dia que não existe');
  assert.equal(showDate('2026-11-23'), '23/11/2026');
  assert.equal(maskDate('23112026'), '23/11/2026');
});

test('máscara de data e hora deixa o cursor onde a pessoa digita', () => {
  // Corrige o dia no começo: apaga o "7" de "27/11/2026" e digita "8".
  assert.deepEqual(maskedEdit('27/11/2026', '2/11/2026', 1, 8, maskDate), { value: '21/12/026', caret: 1 });
  assert.deepEqual(maskedEdit('27/11/2026', '287/11/2026', 2, 8, maskDate), { value: '28/11/2026', caret: 3 }, 'campo cheio: o dígito novo substitui o seguinte');
  assert.deepEqual(maskedEdit('14:00', '14:300', 4, 4, maskTime), { value: '14:30', caret: 4 });
  assert.deepEqual(maskedEdit('', '2', 1, 8, maskDate), { value: '2', caret: 1 });
  assert.deepEqual(maskedEdit('27', '271', 3, 8, maskDate), { value: '27/1', caret: 4 }, 'digitando no fim, passa a barra');
  assert.deepEqual(maskedEdit('27/11', '2711', 2, 8, maskDate), { value: '21/1', caret: 1 }, 'apagar a barra apaga o dígito antes');
});
