import test from 'node:test';
import assert from 'node:assert/strict';
import { withPolicy } from '../src/services/BookSafety.ts';

test('toda página de EPUB recebe a política que bloqueia scripts e rede', () => {
  const policy = /<meta http-equiv="Content-Security-Policy" content="default-src 'none';[^"]*"\/>/;
  const xhtml = withPolicy('<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title><script>x()</script></head><body onload="x()"></body></html>');
  assert.match(xhtml, policy);
  assert.ok(xhtml.indexOf('Content-Security-Policy') < xhtml.indexOf('<script>'), 'antes de qualquer script');
  assert.ok(!/script-src/.test(xhtml.match(policy)[0]), 'sem exceção para scripts (vale o default-src none)');
  assert.ok(!/https?:|'self'/.test(xhtml.match(policy)[0]), 'nada da rede nem da origem da Ayla');
  assert.match(withPolicy('<html><body>sem head</body></html>'), /<html><head><meta http-equiv="Content-Security-Policy"/);
  assert.match(withPolicy('<HEAD lang="pt">x'), /^<HEAD lang="pt"><meta/);
  assert.match(withPolicy('<p>fragmento</p>'), /^<meta http-equiv="Content-Security-Policy"/);
});
