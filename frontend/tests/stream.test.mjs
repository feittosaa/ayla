import test from 'node:test';
import assert from 'node:assert/strict';
import { readChatStream, streamChat } from '../src/services/Api.ts';

function stream(data, size = 1) {
  const bytes = new TextEncoder().encode(data);
  let index = 0;
  return new ReadableStream({ pull(controller) {
    if (index >= bytes.length) return controller.close();
    controller.enqueue(bytes.slice(index, index += size));
  }});
}

test('SSE mantém Unicode, espaços e linhas mesmo com um byte por chunk', async () => {
  const tokens = [' Olá 🧡\n', '  segunda linha\n\nfim '];
  const data = ': heartbeat\r\n\r\n' + tokens.map(token => `data: ${JSON.stringify({token})}\r\n\r\n`).join('') + 'event: done\r\ndata: {}\r\n\r\n';
  const actual = [];
  await readChatStream(stream(data), token => actual.push(token));
  assert.deepEqual(actual, tokens);
});

test('falha parcial preserva tokens e não parece conclusão', async () => {
  const actual = [];
  await assert.rejects(readChatStream(stream('data: {"token":"parte"}\n\nevent: error\ndata: {"message":"Falhou"}\n\n'), x => actual.push(x)), /Falhou/);
  assert.deepEqual(actual, ['parte']);
});

test('EOF sem evento done e JSON inválido são erros', async () => {
  await assert.rejects(readChatStream(stream('data: {"token":"parte"}\n\n'), () => {}), /antes de concluir/);
  await assert.rejects(readChatStream(stream('data: INVALIDO\n\n'), () => {}), SyntaxError);
});

test('cliente envia token e recusa erro HTTP antes de consumir corpo', async () => {
  const original = globalThis.fetch;
  try {
    globalThis.fetch = async (url, options) => {
      assert.equal(url, 'http://127.0.0.1:1234/chat');
      assert.equal(options.headers['X-Ayla-Token'], 'teste');
      return new Response('{}', {status: 401, headers: {'content-type': 'application/json'}});
    };
    await assert.rejects(streamChat({url: 'http://127.0.0.1:1234', token: 'teste'}, 'oi', () => {}), /reiniciada/);
  } finally { globalThis.fetch = original; }
});

test('cancelamento fecha leitor e propaga AbortError', async () => {
  const body = new ReadableStream({ start(controller) {
    controller.error(new DOMException('Interrompido', 'AbortError'));
  }});
  await assert.rejects(readChatStream(body, () => {}), { name: 'AbortError' });
  assert.equal(body.locked, false);
});
