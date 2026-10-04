import json
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from imagefixtures import fake_cv2, stub_module

with stub_module('cv2', fake_cv2()):
    import realesrgan_upscale


class FailureMessageTests(unittest.TestCase):
    def test_failures_leave_existing_outputs_untouched(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            src = root / 'a.png'
            Image.new('RGB', (4, 4)).save(src)
            kept, fresh = root / 'out/kept.png', root / 'out/fresh.png'
            kept.parent.mkdir()
            kept.write_bytes(b'previous')
            manifest = root / 'files.json'
            manifest.write_text(json.dumps([[str(src), str(kept)], [str(src), str(fresh)]]), encoding='utf-8')
            unreadable = fake_cv2()
            unreadable.imdecode = lambda buf, flags: None
            session = mock.Mock()
            session.get_inputs.return_value = [types.SimpleNamespace(name='input')]
            session.get_outputs.return_value = [types.SimpleNamespace(name='output')]
            messages = []
            argv = ['realesrgan_upscale.py', '--files', str(manifest), '--model-path', str(root / 'm.onnx')]
            with stub_module('cv2', unreadable), \
                    mock.patch.object(sys, 'argv', argv), \
                    mock.patch.object(realesrgan_upscale, 'bootstrap'), \
                    mock.patch.object(realesrgan_upscale, 'create_session', return_value=(session, 'cpu')), \
                    mock.patch('purin_proto.emit', side_effect=messages.append):
                realesrgan_upscale.main()
            errors = [m['message'] for m in messages if m.get('status') == 'error']
            self.assertEqual(errors, ['[1/2] ✗ a.png: 无法读取图片', '[2/2] ✗ a.png: 无法读取图片'])
            self.assertEqual(messages[-1]['type'], 'done')
            self.assertEqual(messages[-1]['fail_count'], 2)
            self.assertEqual(kept.read_bytes(), b'previous')
            self.assertFalse(fresh.exists())


if __name__ == '__main__':
    unittest.main()
