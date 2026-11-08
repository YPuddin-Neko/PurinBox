import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import httpx
from gbc_worker import AnalysisReplies, BatchStages, analyze_graph
from kohakugbc.engines.vlm.api import ChatAdapter
from kohakugbc.nodes.specs import parse
from kohakugbc.nodes.root import image_root
from kohakugbc.graph import to_text


class AnalysisTests(unittest.TestCase):
    def setUp(self):
        self.options = dict(max_tokens=-1, max_requests=3, image_detail='original')
        self.adapter = ChatAdapter('openai', 'test-model', flavour='openai', structured='prompt')
        self.guard = AnalysisReplies(self.adapter, self.options)
        self.inputs = {'request': dict(key='image-a', schema='image.json', system=None,
            query='Describe the image', images=[], temperature=.1, max_tokens=1024)}

    def response(self, text, **extra):
        prepared = self.guard.prepare(self.inputs, 'http')
        return httpx.Response(200, request=httpx.Request('POST', 'https://example.invalid/v1/chat/completions', headers=prepared.headers),
            json={'choices': [{'message': {'content': text}, **extra}]})

    def test_upstream_unstructured_reply_reproduces_empty_graph(self):
        reply = 'A blue cup is sitting on a desk.'
        self.assertEqual(parse.image(reply)['descs'], [])
        root = image_root({'key':'test'}, reply)
        self.assertIsNone(to_text([root], 'test.png', [16,16])[0])
        with self.assertRaisesRegex(ValueError, 'JSON 格式'):
            self.guard.decode(self.response(reply), 'http')

    def test_unlimited_tokens_does_not_inherit_query_cap(self):
        self.assertNotIn('max_tokens', self.guard.prepare(self.inputs, 'http').json)
        adapter = ChatAdapter('openai', 'test-model', flavour='openai', sampling={'max_tokens':2048}, structured='prompt')
        guard = AnalysisReplies(adapter, {**self.options, 'max_tokens':2048})
        self.assertEqual(guard.prepare(self.inputs, 'http').json['max_tokens'], 2048)

    def test_empty_truncated_and_missing_description_replies(self):
        for body, extra, reason in [(None, {}, '正文为空'), ('{', {'finish_reason':'length'}, '截断'),
                (json.dumps({'detailed_caption':'','concise_caption':'','elements':[]}), {}, '整图描述')]:
            self.guard.reset()
            with self.subTest(body=body), self.assertRaisesRegex(ValueError, reason):
                self.guard.decode(self.response(body, **extra), 'http')

    def test_valid_reply_is_not_rewritten(self):
        reply = json.dumps({'detailed_caption':'A blue cup.', 'concise_caption':'A cup.', 'elements':[]})
        self.assertEqual(self.guard.decode(self.response(reply), 'http'), reply)

    def test_budget_is_per_image_and_includes_retries(self):
        for _ in range(2):
            self.guard.reset()
            for _ in range(3):
                self.guard.prepare(self.inputs, 'http')
            with self.assertRaisesRegex(ValueError, '请求上限'):
                self.guard.prepare(self.inputs, 'http')

    def test_invalid_response_retries_then_produces_real_graph(self):
        from types import SimpleNamespace
        from unittest.mock import Mock
        good = json.dumps({'detailed_caption':'A blue cup.', 'concise_caption':'A cup.', 'elements':[]})
        answers = iter(['plain prose without JSON', good, good])
        def run(sources, sink):
            try:
                text = self.guard.decode(self.response(next(answers)), 'http')
                graph, stats = to_text([image_root({'key':'test'}, text)], 'test.png', [16,16])
                sink.write(SimpleNamespace(ok=True, outputs={'graph':graph, 'stats':stats}))
            except ValueError as error:
                sink.write(SimpleNamespace(ok=False, failure=str(error)))
        machine = Mock(run=Mock(side_effect=run))
        with patch('gbc_worker.emit') as emit:
            result = analyze_graph(machine, object(), self.guard, 'test.png')
            self.assertTrue(result['vertices'])
            self.assertEqual(machine.run.call_count, 2)
            self.assertEqual(self.guard.count, 2)
            self.assertEqual(emit.call_count, 1)
            analyze_graph(machine, object(), self.guard, 'next.png')
            self.assertEqual(self.guard.count, 1)

    def test_invalid_response_stops_after_one_retry(self):
        from types import SimpleNamespace
        from unittest.mock import Mock
        def run(sources, sink):
            try:
                self.guard.decode(self.response(''), 'http')
            except ValueError as error:
                sink.write(SimpleNamespace(ok=False, failure=str(error)))
        machine = Mock(run=Mock(side_effect=run))
        with patch('gbc_worker.emit'), self.assertRaisesRegex(ValueError, '正文为空'):
            analyze_graph(machine, object(), self.guard, 'test.png')
        self.assertEqual(machine.run.call_count, 2)

    def test_stage_messages_are_once_per_batch(self):
        with patch('gbc_worker.emit') as emit:
            stages = BatchStages()
            for _ in range(3):
                stages.log('正在分析全图、区域和对象关系')
                stages.log('正在整理描述与导出内容')
            self.assertEqual(emit.call_count, 2)
            BatchStages().log('正在分析全图、区域和对象关系')
            self.assertEqual(emit.call_count, 3)

if __name__ == '__main__':
    unittest.main()
