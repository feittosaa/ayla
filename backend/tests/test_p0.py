"""Regressões do P0. Apenas bancos temporários e provedores simulados."""

import asyncio
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from app.core.conversation_memory import ConversationMemory
from app.core.persistent_memory import PersistentMemory
from app.core import chat_engine
from app.main import app
from app.services.llm_service import LLMService
from app.services.inference_router import InferenceRouter, ProviderUnavailable
from backup import backup_database
from app.services.ollama_client import OllamaClient
from app.services.cloud_client import CloudReadOnlyClient


async def request(path, token=None, payload=None, method='POST'):
    messages = []
    sent = False
    async def receive():
        nonlocal sent
        if not sent:
            sent = True
            return {'type': 'http.request', 'body': json.dumps(payload or {}).encode(), 'more_body': False}
        await asyncio.Future()
    async def send(message):
        messages.append(message)
    headers = [(b'content-type', b'application/json')]
    if token is not None:
        headers.append((b'x-ayla-token', token.encode()))
    await app({
        'type': 'http', 'asgi': {'version': '3.0', 'spec_version': '2.4'},
        'http_version': '1.1', 'method': method, 'scheme': 'http',
        'path': path, 'raw_path': path.encode(), 'query_string': b'',
        'root_path': '', 'headers': headers, 'server': ('127.0.0.1', 8000),
        'client': ('127.0.0.1', 1234),
    }, receive, send)
    status = next(m['status'] for m in messages if m['type'] == 'http.response.start')
    body = b''.join(m.get('body', b'') for m in messages if m['type'] == 'http.response.body')
    return status, body


class MemoryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / 'memory.db'
        self.memory = PersistentMemory(self.path)
        self.short = ConversationMemory()
        self.llm = Mock()
        for name, value in [('persistent_memory', self.memory), ('conversation_memory', self.short), ('llm_service', self.llm)]:
            patcher = patch.object(chat_engine, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch.object(chat_engine, 'maybe_compress_memory')
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_import_and_constructor_do_not_create_database(self):
        self.assertFalse(self.path.exists())

    def test_complete_answer_saved_and_current_message_occurs_once(self):
        self.short.add_user_message('anterior')
        self.llm.stream.return_value = iter(['Olá ', '🧡\n  fim'])
        self.assertEqual(''.join(chat_engine.chat_stream('mensagem única')), 'Olá 🧡\n  fim')
        self.assertEqual(self.memory.get_last_messages()[-1], {'role': 'assistant', 'content': 'Olá 🧡\n  fim'})
        prompt = self.llm.stream.call_args.kwargs['local_prompt']
        self.assertEqual(prompt.count('mensagem única'), 1)
        self.assertIn('anterior', prompt)

    def test_interrupted_response_is_not_saved_as_complete(self):
        def fail():
            yield 'parcial'
            raise RuntimeError('offline')
        self.llm.stream.return_value = fail()
        with self.assertRaises(RuntimeError):
            list(chat_engine.chat_stream('oi'))
        self.assertEqual([m['role'] for m in self.memory.get_last_messages()], ['user'])

    def test_cancelled_generator_is_not_saved_as_complete(self):
        self.llm.stream.return_value = iter(['parcial', 'fim'])
        stream = chat_engine.chat_stream('oi')
        self.assertEqual(next(stream), 'parcial')
        stream.close()
        self.assertEqual([m['role'] for m in self.memory.get_last_messages()], ['user'])

    def test_existing_schema_and_backup_remain_readable(self):
        self.memory.add_message('user', 'dado de teste')
        self.memory.add_summary('resumo de teste')
        destination = Path(self.tmp.name) / 'backup.db'
        backup_database(self.path, destination)
        restored = PersistentMemory(destination)
        self.assertEqual(restored.get_last_messages(), self.memory.get_last_messages())
        self.assertEqual(restored.get_all_summaries(), ['resumo de teste'])
        with self.assertRaises(FileExistsError):
            backup_database(self.path, destination)

    def test_database_path_is_independent_of_working_directory(self):
        with patch.dict(os.environ, {}, clear=True):
            before = PersistentMemory().path
            previous = Path.cwd()
            try:
                os.chdir(self.tmp.name)
                self.assertEqual(before, PersistentMemory().path)
            finally:
                os.chdir(previous)
        self.assertEqual(before, Path(__file__).resolve().parents[1] / 'ayla_memory.db')


class ProviderTests(unittest.TestCase):
    def test_optional_models_and_cloud_disabled_by_default(self):
        with patch.dict(os.environ, {}, clear=True):
            service = LLMService()
            self.assertEqual(service.settings.local_model, '')
            self.assertIsNone(service.router)
            router = service._get_router()
            self.assertIsNone(router.local)
            self.assertIsNone(router.cloud)
            with self.assertRaises(ProviderUnavailable):
                list(service.stream('histórico privado', 'oi', False))

    def test_local_first(self):
        local, cloud = Mock(), Mock()
        local.stream_local.return_value = iter(['local'])
        self.assertEqual(list(InferenceRouter(local, cloud).stream('contexto', 'entrada', False)), ['local'])
        cloud.stream_read_only.assert_not_called()

    def test_private_input_never_falls_back_to_cloud(self):
        local, cloud = Mock(), Mock()
        local.stream_local.side_effect = RuntimeError('offline')
        with self.assertRaises(ProviderUnavailable):
            list(InferenceRouter(local, cloud).stream('contexto', 'meu dado', True))
        cloud.stream_read_only.assert_not_called()

    def test_public_input_can_fall_back_with_only_current_message(self):
        local, cloud = Mock(), Mock()
        local.stream_local.side_effect = RuntimeError('offline')
        cloud.stream_read_only.return_value = iter(['cloud'])
        self.assertEqual(list(InferenceRouter(local, cloud).stream('histórico', 'entrada', False)), ['cloud'])
        cloud.stream_read_only.assert_called_once_with('entrada')

    def test_partial_failure_does_not_mix_providers(self):
        def fail():
            yield 'parcial'
            raise RuntimeError('offline')
        local, cloud = Mock(), Mock()
        local.stream_local.return_value = fail()
        stream = InferenceRouter(local, cloud).stream('contexto', 'entrada', False)
        self.assertEqual(next(stream), 'parcial')
        with self.assertRaises(ProviderUnavailable):
            next(stream)
        cloud.stream_read_only.assert_not_called()

    def test_cloud_requires_flag_and_key(self):
        for env in [{'OPENAI_API_KEY': 'teste'}, {'AYLA_CLOUD_ENABLED': 'true'}]:
            with self.subTest(env=env), patch.dict(os.environ, env, clear=True):
                self.assertIsNone(LLMService()._get_router().cloud)


class TransportTests(unittest.TestCase):
    def call(self, *args, **kwargs):
        return asyncio.run(request(*args, **kwargs))

    def test_fail_closed_and_wrong_tokens(self):
        for env, token in [({}, None), ({'AYLA_TOKEN': 'correto'}, None), ({'AYLA_TOKEN': 'correto'}, 'errado')]:
            with self.subTest(env=bool(env), token=bool(token)), patch.dict(os.environ, env, clear=True):
                self.assertEqual(self.call('/chat', token, {'message': 'oi'})[0], 401)

    def test_health_requires_token_and_does_not_call_models(self):
        with patch.dict(os.environ, {'AYLA_TOKEN': 'teste'}), patch.object(chat_engine.llm_service, 'stream', side_effect=AssertionError('Rede proibida')):
            self.assertEqual(self.call('/health', method='GET')[0], 401)
            status, body = self.call('/health', 'teste', method='GET')
            self.assertEqual(status, 200)
            self.assertEqual(json.loads(body)['status'], 'ok')

    def test_invalid_payload_is_http_error(self):
        with patch.dict(os.environ, {'AYLA_TOKEN': 'teste'}):
            for payload in [{}, {'message': ''}, {'message': '   '}, {'message': 123}]:
                self.assertEqual(self.call('/chat', 'teste', payload)[0], 422)

    def test_sse_preserves_content_and_signals_completion(self):
        with patch.dict(os.environ, {'AYLA_TOKEN': 'teste'}), patch('app.controllers.chat_controller.chat_http', return_value=iter([' Olá\n\n', '🧡  '])):
            status, body = self.call('/chat', 'teste', {'message': 'oi'})
        self.assertEqual(status, 200)
        frames = body.decode().split('\n\n')
        self.assertEqual(json.loads(frames[0][6:])['token'], ' Olá\n\n')
        self.assertEqual(json.loads(frames[1][6:])['token'], '🧡  ')
        self.assertIn('event: done', frames[2])

    def test_stream_failure_is_error_not_done(self):
        def fail(_):
            yield 'parcial'
            raise RuntimeError('informação interna que não deve aparecer')
        with patch.dict(os.environ, {'AYLA_TOKEN': 'teste'}), patch('app.controllers.chat_controller.chat_http', fail):
            _, body = self.call('/chat', 'teste', {'message': 'oi'})
        self.assertIn(b'event: error', body)
        self.assertNotIn(b'event: done', body)
        self.assertNotIn(b'interna', body)


class AdapterTests(unittest.TestCase):
    def response(self, lines):
        response = Mock()
        response.__enter__ = Mock(return_value=response)
        response.__exit__ = Mock(return_value=False)
        response.iter_lines.return_value = iter(lines)
        return response

    def test_ollama_closes_http_response_when_consumer_stops(self):
        response = self.response([b'{"response":"primeiro"}', b'{"response":"fim","done":true}'])
        with patch('app.services.ollama_client.requests.post', return_value=response):
            stream = OllamaClient('modelo-de-teste').stream_local('oi')
            self.assertEqual(next(stream), 'primeiro')
            stream.close()
        response.__exit__.assert_called_once()

    def test_ollama_unfinished_stream_is_not_success(self):
        response = self.response([b'{"response":"parcial"}'])
        with patch('app.services.ollama_client.requests.post', return_value=response):
            with self.assertRaisesRegex(RuntimeError, 'sem conclusão'):
                list(OllamaClient('modelo-de-teste').stream_local('oi'))

    def test_cloud_accepts_done_and_closes_response(self):
        response = self.response([b'data: {"choices":[{"delta":{"content":"teste"}}]}', b'data: [DONE]'])
        with patch('app.services.cloud_client.requests.post', return_value=response) as post:
            tokens = list(CloudReadOnlyClient('chave-de-teste').stream_read_only('entrada'))
        self.assertEqual(tokens, ['teste'])
        self.assertEqual(post.call_args.kwargs['json']['messages'], [{'role': 'user', 'content': 'entrada'}])
        response.__exit__.assert_called_once()

    def test_cloud_unfinished_stream_is_not_success(self):
        response = self.response([b'data: {"choices":[{"delta":{"content":"parcial"}}]}'])
        with patch('app.services.cloud_client.requests.post', return_value=response):
            with self.assertRaisesRegex(RuntimeError, 'sem conclusão'):
                list(CloudReadOnlyClient('chave-de-teste').stream_read_only('oi'))


if __name__ == '__main__':
    unittest.main()
