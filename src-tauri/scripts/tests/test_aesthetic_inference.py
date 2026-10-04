import contextlib
import io
import json
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aesthetic_inference
from imagefixtures import png_bytes, stub_module


def noisy(size=(48, 48), seed=0):
    rng = np.random.default_rng(seed)
    return Image.fromarray((rng.random((size[1], size[0], 3)) * 255).astype('uint8'))


class FakeSession:
    """onnxruntime 会话替身：按 providers 记录，failing 里的 provider 推理时抛错"""

    def __init__(self, providers, failing=()):
        self.providers = [p[0] if isinstance(p, tuple) else p for p in providers]
        self.failing = failing
        self.batches = []

    def get_inputs(self):
        return [types.SimpleNamespace(name='input', shape=[None, 3, 8, 8])]

    def get_providers(self):
        return self.providers

    def run(self, _outputs, feeds):
        if self.providers[0] in self.failing:
            raise RuntimeError(f'{self.providers[0]} 推理失败')
        batch = feeds['input']
        self.batches.append(batch.shape)
        return [np.tile(np.array([[3., 0.]]), (batch.shape[0], 1))]


class AestheticRunTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / 'meta.json').write_text(json.dumps({'labels': ['good', 'low'], 'img_size': 8}))

    def run_main(self, commands, providers=('CPUExecutionProvider',), failing=()):
        sessions = []

        def make_session(path, options, providers):
            sessions.append(FakeSession(providers, failing))
            return sessions[-1]

        ort = types.SimpleNamespace(InferenceSession=make_session,
                                    GraphOptimizationLevel=types.SimpleNamespace(ORT_ENABLE_ALL=1))
        stream = io.StringIO('\n'.join(map(json.dumps, commands)))
        messages = []
        with contextlib.ExitStack() as stack:
            stack.enter_context(stub_module('onnxruntime', ort))
            stack.enter_context(mock.patch.object(aesthetic_inference, 'bootstrap'))
            stack.enter_context(mock.patch.object(aesthetic_inference, 'utf8_stdin', return_value=stream))
            stack.enter_context(mock.patch('gpu_diagnostics.resolve_ort_providers', return_value=list(providers)))
            stack.enter_context(mock.patch('gpu_diagnostics.quiet_session_options',
                                           return_value=types.SimpleNamespace()))
            stack.enter_context(mock.patch('purin_proto.emit', side_effect=messages.append))
            aesthetic_inference.main()
        return messages, sessions

    def init(self):
        return {'cmd': 'init', 'model_path': str(self.root / 'model.onnx')}

    def test_single_image_uses_batch_and_preserves_sidecar(self):
        src = self.root / 'a.png'
        noisy().save(src)
        src.with_suffix('.txt').write_text('tag', encoding='utf-8')
        messages, sessions = self.run_main([
            self.init(),
            {'cmd': 'score_batch', 'images': [{'image_path': str(src), 'copy_files': True,
                                              'output_path': str(self.root / 'out')}]},
            {'cmd': 'quit'},
        ])
        self.assertEqual([m['type'] for m in messages], ['ready', 'result'])
        self.assertEqual(messages[1]['image_path'], str(src))
        self.assertEqual(messages[1]['label'], 'good')
        self.assertTrue(src.exists())
        self.assertEqual((self.root / 'out/good/a.png').read_bytes(), src.read_bytes())
        self.assertEqual((self.root / 'out/good/a.txt').read_text(), 'tag')
        self.assertEqual(sessions[0].batches, [(1, 3, 8, 8)])

    def test_gpu_batch_failure_retries_on_cpu(self):
        src = self.root / 'a.png'
        noisy().save(src)
        messages, sessions = self.run_main(
            [self.init(), {'cmd': 'score_batch', 'images': [{'image_path': str(src), 'copy_files': True,
                                                            'output_path': str(self.root / 'out')}]}],
            providers=('CUDAExecutionProvider', 'CPUExecutionProvider'), failing=('CUDAExecutionProvider',))
        self.assertEqual([s.providers for s in sessions],
                         [['CUDAExecutionProvider', 'CPUExecutionProvider'], ['CPUExecutionProvider']])
        self.assertEqual([m['type'] for m in messages if m['type'] != 'log'], ['ready', 'result'])

    def test_cpu_batch_failure_falls_back_to_single_images_without_rebuilding(self):
        src = self.root / 'a.png'
        noisy().save(src)
        messages, sessions = self.run_main(
            [self.init(), {'cmd': 'score_batch', 'images': [{'image_path': str(src)}]}],
            failing=('CPUExecutionProvider',))
        self.assertEqual(len(sessions), 1)
        errors = [m for m in messages if m['type'] == 'error']
        self.assertEqual(len(errors), 1)
        self.assertEqual(errors[0]['image_path'], str(src))

    def test_init_failure_reports_reason_without_traceback(self):
        messages, _ = self.run_main([{'cmd': 'init', 'model_path': str(self.root / 'missing' / 'model.onnx')}])
        self.assertEqual(messages[-1]['type'], 'error')
        self.assertTrue(messages[-1]['message'].startswith('初始化失败: '))
        self.assertNotIn('Traceback', messages[-1]['message'])
        self.assertTrue(any(m['type'] == 'log' and 'Traceback' in m['message'] for m in messages))


class PreprocessTests(unittest.TestCase):
    def test_16_bit_gray_is_scaled_not_saturated(self):
        with tempfile.TemporaryDirectory() as directory:
            src = Path(directory) / 'gray16.png'
            src.write_bytes(png_bytes(np.full((16, 16), 40000, np.uint16)))
            tensor = aesthetic_inference.preprocess_image(src, 8)
        expected = (156 / 255.0 - 0.5) / 0.5
        self.assertEqual(tensor.shape, (1, 3, 8, 8))
        self.assertTrue(np.allclose(tensor, expected, atol=1e-6))


if __name__ == '__main__':
    unittest.main()
