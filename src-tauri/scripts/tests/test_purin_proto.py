import io
import json
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gpu_diagnostics
import purin_proto


class FakeStdout:
    def __init__(self):
        self.buffer = io.BytesIO()

    def lines(self):
        return self.buffer.getvalue().decode('utf-8').splitlines()


def captured(fn, *args, **kwargs):
    out = FakeStdout()
    with mock.patch('sys.stdout', out):
        fn(*args, **kwargs)
    return out


class EmitTests(unittest.TestCase):
    def emitted(self, fn, *args, **kwargs):
        lines = captured(fn, *args, **kwargs).lines()
        self.assertEqual(len(lines), 1)
        return json.loads(lines[0])

    def test_emit_writes_one_utf8_line_without_escaping(self):
        out = captured(purin_proto.emit, {'type': 'log', 'message': '✓ 完成'})
        self.assertEqual(out.buffer.getvalue(), '{"type": "log", "message": "✓ 完成"}\n'.encode('utf-8'))

    def test_message_shapes_and_key_order(self):
        cases = [
            ((purin_proto.log, '加载中'), {'type': 'log', 'message': '加载中'}),
            ((purin_proto.log_i18n, 'gpu.usingCpu'),
             {'type': 'log', 'i18n_key': 'gpu.usingCpu', 'message': 'gpu.usingCpu'}),
            ((purin_proto.log_i18n, 'gpu.detected', {'name': 'RTX'}),
             {'type': 'log', 'i18n_key': 'gpu.detected', 'message': 'gpu.detected', 'i18n_params': {'name': 'RTX'}}),
            ((purin_proto.log_i18n, 'gpu.detected', {}),
             {'type': 'log', 'i18n_key': 'gpu.detected', 'message': 'gpu.detected'}),
            ((purin_proto.error, '坏了'), {'type': 'error', 'message': '坏了'}),
            ((purin_proto.progress, 2, 5, 'a.png', 'success', '[2/5] ✓ a.png'),
             {'type': 'progress', 'current': 2, 'total': 5, 'filename': 'a.png',
              'status': 'success', 'message': '[2/5] ✓ a.png'}),
            ((purin_proto.progress, 1, 3, 'a.png'),
             {'type': 'progress', 'current': 1, 'total': 3, 'filename': 'a.png'}),
            ((purin_proto.progress, 1, 3, '', 'processing', ''),
             {'type': 'progress', 'current': 1, 'total': 3, 'filename': '', 'status': 'processing', 'message': ''}),
        ]
        for (fn, *args), expected in cases:
            with self.subTest(fn=fn.__name__, args=args):
                got = self.emitted(fn, *args)
                self.assertEqual(got, expected)
                self.assertEqual(list(got), list(expected))

    def test_fields_follow_type(self):
        got = self.emitted(purin_proto.error, '推理失败', image_path='/a.png')
        self.assertEqual(list(got.items()), [('type', 'error'), ('image_path', '/a.png'), ('message', '推理失败')])
        got = self.emitted(purin_proto.result, image_path='/a.png', tag_count=0, skipped=True)
        self.assertEqual(list(got.items()),
                         [('type', 'result'), ('image_path', '/a.png'), ('tag_count', 0), ('skipped', True)])
        got = self.emitted(purin_proto.done, success=1, fail=0, total=1)
        self.assertEqual(list(got.items()), [('type', 'done'), ('success', 1), ('fail', 0), ('total', 1)])


class StdinTests(unittest.TestCase):
    def test_decodes_utf8_and_replaces_invalid_bytes(self):
        fake = types.SimpleNamespace(buffer=io.BytesIO('{"p": "中文"}\n'.encode('utf-8') + b'\xff\n'))
        with mock.patch('sys.stdin', fake):
            reader = purin_proto.utf8_stdin()
            self.assertEqual(reader.readline(), '{"p": "中文"}\n')
            self.assertEqual(list(reader), ['\ufffd\n'])


class IsUnderTests(unittest.TestCase):
    def test_paths(self):
        base = os.path.join(tempfile.gettempdir(), 'purin_base')
        self.assertTrue(purin_proto.is_under(base, base))
        self.assertTrue(purin_proto.is_under(os.path.join(base, 'sub', 'a.png'), base))
        self.assertTrue(purin_proto.is_under(os.path.join(base, 'sub', '..', 'a.png'), base))
        self.assertFalse(purin_proto.is_under(base + 'x', base))
        self.assertFalse(purin_proto.is_under(os.path.dirname(base), base))
        self.assertFalse(purin_proto.is_under(base, ''))
        self.assertFalse(purin_proto.is_under(base, None))

    def test_relative_paths_resolve_against_cwd(self):
        self.assertTrue(purin_proto.is_under('a/b.png', os.getcwd()))
        self.assertTrue(purin_proto.is_under(os.path.join(os.getcwd(), 'x'), '.'))

    def test_compares_case_insensitively_where_platform_does(self):
        with mock.patch('os.path.normcase', side_effect=str.lower):
            self.assertTrue(purin_proto.is_under('/Data/Out/a.png', '/data/out'))

    def test_paths_without_common_root_are_not_under(self):
        # Windows 上不同盘符时 commonpath 抛 ValueError
        with mock.patch('os.path.commonpath', side_effect=ValueError):
            self.assertFalse(purin_proto.is_under('D:\\a.png', 'C:\\out'))


class TempDirTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)

    def leftovers(self):
        return sorted(p.name for p in self.root.iterdir() if p.name.endswith('.tmp'))


class ReadTextCompatTests(TempDirTest):
    def read(self, data):
        path = self.root / 'tags.txt'
        path.write_bytes(data)
        return purin_proto.read_text_compat(path)

    def test_utf8_with_and_without_bom(self):
        self.assertEqual(self.read('长发, blue eyes'.encode('utf-8')), '长发, blue eyes')
        self.assertEqual(self.read(b'\xef\xbb\xbf' + 'long hair, 长发'.encode('utf-8')), 'long hair, 长发')

    def test_falls_back_to_gbk(self):
        self.assertEqual(self.read('长发, 蓝色眼睛'.encode('gbk')), '长发, 蓝色眼睛')

    def test_returns_none_when_neither_encoding_fits(self):
        # 0xFF 在 UTF-8 和 GBK 里都不合法：不按 replace 硬解成乱码
        self.assertIsNone(self.read(b'long hair, \xff\xfe'))

    def test_newlines_match_text_mode_open(self):
        path = self.root / 'n.txt'
        path.write_bytes(b'a, b\r\nc\rd\n')
        with open(path, encoding='utf-8') as f:
            expected = f.read()
        self.assertEqual(purin_proto.read_text_compat(path), expected)
        self.assertEqual(expected, 'a, b\nc\nd\n')

    def test_missing_file_raises(self):
        with self.assertRaises(FileNotFoundError):
            purin_proto.read_text_compat(self.root / 'missing.txt')


class ReplaceAtomicallyTests(TempDirTest):
    def test_success_leaves_no_temp_file(self):
        out = self.root / 'a.txt'
        purin_proto.replace_atomically(out, lambda tmp: Path(tmp).write_text('new'))
        self.assertEqual(out.read_text(), 'new')
        self.assertEqual(self.leftovers(), [])

    def test_failed_write_keeps_original_and_removes_temp(self):
        out = self.root / 'a.txt'
        out.write_text('old')

        def half_write(tmp):
            Path(tmp).write_text('ne')
            raise OSError('disk full')

        with self.assertRaises(OSError):
            purin_proto.replace_atomically(out, half_write)
        self.assertEqual(out.read_text(), 'old')
        self.assertEqual(self.leftovers(), [])

    def test_failed_replace_removes_temp(self):
        target = self.root / 'a.png'
        target.mkdir()
        with self.assertRaises(OSError):
            purin_proto.replace_atomically(target, lambda tmp: Path(tmp).write_bytes(b'x'))
        self.assertTrue(target.is_dir())
        self.assertEqual(self.leftovers(), [])

    def test_interrupt_also_removes_temp(self):
        def interrupted(tmp):
            Path(tmp).write_bytes(b'x')
            raise KeyboardInterrupt

        with self.assertRaises(KeyboardInterrupt):
            purin_proto.replace_atomically(self.root / 'a.bin', interrupted)
        self.assertEqual(self.leftovers(), [])

    def test_failure_before_temp_exists_reraises_original_error(self):
        def fail(tmp):
            raise ValueError('编码图片失败')

        with self.assertRaises(ValueError):
            purin_proto.replace_atomically(self.root / 'a.png', fail)


class WriteTextAtomicTests(TempDirTest):
    def test_matches_text_mode_write(self):
        text = '{\n  "tags": ["长发"]\n}'
        out, ref = self.root / 'a.json', self.root / 'ref.json'
        purin_proto.write_text_atomic(out, text)
        with open(ref, 'w', encoding='utf-8') as f:
            f.write(text)
        self.assertEqual(out.read_bytes(), ref.read_bytes())
        self.assertEqual(self.leftovers(), [])

    def test_unencodable_text_keeps_original(self):
        out = self.root / 'a.txt'
        out.write_text('old')
        with self.assertRaises(UnicodeEncodeError):
            purin_proto.write_text_atomic(out, 'bad \udcff')
        self.assertEqual(out.read_text(), 'old')
        self.assertEqual(self.leftovers(), [])


class BootstrapTests(unittest.TestCase):
    def test_puts_script_dir_first_and_registers_cuda_dlls(self):
        import cuda_dll_helper
        saved = list(sys.path)
        self.addCleanup(setattr, sys, 'path', saved)
        with mock.patch.object(cuda_dll_helper, 'register_cuda_dlls') as register:
            purin_proto.bootstrap()
        self.assertEqual(sys.path[0], purin_proto.SCRIPT_DIR)
        self.assertTrue(os.path.samefile(purin_proto.SCRIPT_DIR, Path(__file__).resolve().parents[1]))
        register.assert_called_once_with()


class FakeOrt:
    """onnxruntime 替身：providers 首项在 failing 里时建会话失败"""

    def __init__(self, failing=()):
        self.failing = set(failing)
        self.calls = []

    def InferenceSession(self, path, sess_options, providers):
        self.calls.append((path, sess_options, list(providers)))
        first = providers[0][0] if isinstance(providers[0], tuple) else providers[0]
        if first in self.failing:
            raise RuntimeError(f'{first} 不可用 #{len(self.calls)}')
        return ('session', first)


class CpuFallbackTests(unittest.TestCase):
    def run_with(self, ort, providers, on_fallback=None):
        with mock.patch.dict(sys.modules, {'onnxruntime': ort}):
            return gpu_diagnostics.create_session_with_cpu_fallback('m.onnx', providers, 'opts', on_fallback)

    def test_success_does_not_fall_back(self):
        ort, seen = FakeOrt(), []
        session = self.run_with(ort, ['CUDAExecutionProvider', 'CPUExecutionProvider'],
                                lambda *a: seen.append(a))
        self.assertEqual(session, ('session', 'CUDAExecutionProvider'))
        self.assertEqual(len(ort.calls), 1)
        self.assertEqual(seen, [])

    def test_gpu_failure_reports_then_rebuilds_on_cpu(self):
        ort, seen = FakeOrt(failing={'CUDAExecutionProvider'}), []
        cuda = ('CUDAExecutionProvider', {'cudnn_conv_algo_search': 'HEURISTIC'})
        session = self.run_with(ort, [cuda, 'CPUExecutionProvider'], lambda p, e: seen.append((p, str(e))))
        self.assertEqual(session, ('session', 'CPUExecutionProvider'))
        self.assertEqual(seen, [(cuda, 'CUDAExecutionProvider 不可用 #1')])
        self.assertEqual(ort.calls[1], ('m.onnx', 'opts', ['CPUExecutionProvider']))

    def test_fallback_without_callback(self):
        ort = FakeOrt(failing={'CoreMLExecutionProvider'})
        self.assertEqual(self.run_with(ort, ['CoreMLExecutionProvider', 'CPUExecutionProvider']),
                         ('session', 'CPUExecutionProvider'))

    def test_cpu_only_failure_is_raised_without_retry(self):
        ort, seen = FakeOrt(failing={'CPUExecutionProvider'}), []
        with self.assertRaisesRegex(RuntimeError, '#1'):
            self.run_with(ort, ['CPUExecutionProvider'], lambda *a: seen.append(a))
        self.assertEqual(len(ort.calls), 1)
        self.assertEqual(seen, [])

    def test_cpu_retry_failure_propagates(self):
        ort = FakeOrt(failing={'CUDAExecutionProvider', 'CPUExecutionProvider'})
        with self.assertRaisesRegex(RuntimeError, 'CPUExecutionProvider 不可用 #2'):
            self.run_with(ort, ['CUDAExecutionProvider', 'CPUExecutionProvider'])


if __name__ == '__main__':
    unittest.main()
