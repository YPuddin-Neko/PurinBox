import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import person_crop
from imagefixtures import fake_cv2, png_bytes, read_png, stub_module

CROP_OPTIONS = {'person_conf': 0.3, 'upper_conf': 0.5, 'upper_tag': 'upper body',
                'head_conf': 0.4, 'head_tag': 'head view', 'head_scale': 1.5,
                'eyes_conf': 0.3, 'eyes_tag': 'eyes view', 'eyes_scale': 2.4,
                'keep_original_tags': False}


def noisy(mode='RGB', size=(48, 48), seed=0):
    rng = np.random.default_rng(seed)
    channels = {'RGB': 3, 'RGBA': 4}[mode]
    return Image.fromarray((rng.random((size[1], size[0], channels)) * 255).astype('uint8'))


class PersonCropOutputTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.out = self.root / 'out'
        self.out.mkdir()
        self.detected = []
        patcher = mock.patch.object(person_crop, 'detect_with_model', side_effect=self.detect)
        patcher.start()
        self.addCleanup(patcher.stop)

    def detect(self, model, letterbox, conf):
        # 检测用的图：letterbox 第一个参数就是喂给模型前的 8 位 RGB 图
        self.detected.append(letterbox.__wrapped__.args[0])
        return [(4, 4, 36, 36, 0.9)]

    def crop(self, src):
        # 与 Rust 在 init 命令里下发的裁切参数同形；models 只含启用的类型
        result = person_crop.process_image({'person': None}, str(src), CROP_OPTIONS, str(self.out))
        self.assertEqual(result['status'], 'success')
        return self.out / f'{src.stem}_full{src.suffix}'

    def test_transparent_png_keeps_alpha(self):
        src = self.root / 'a.png'
        noisy('RGBA').save(src)
        with Image.open(self.crop(src)) as im:
            self.assertEqual((im.format, im.mode), ('PNG', 'RGBA'))

    def test_jpeg_keeps_source_quality(self):
        src = self.root / 'b.jpg'
        noisy().save(src, quality=98)
        with Image.open(src) as a, Image.open(self.crop(src)) as b:
            self.assertEqual(b.format, 'JPEG')
            self.assertEqual(dict(b.quantization), dict(a.quantization))

    def test_16_bit_gray_is_detected_on_scaled_pixels_and_cropped_at_16_bits(self):
        src = self.root / 'gray16.png'
        src.write_bytes(png_bytes(np.full((48, 48), 40000, np.uint16)))
        out = self.crop(src)
        self.assertEqual(self.detected[0].mode, 'RGB')
        self.assertEqual(self.detected[0].getpixel((10, 10)), (156, 156, 156))
        with Image.open(out) as im:
            self.assertEqual((im.mode, im.size, im.getpixel((5, 5))), ('I;16', (37, 37), 40000))

    def test_16_bit_color_is_cropped_at_16_bits(self):
        src = self.root / 'rgb16.png'
        rgb = np.zeros((48, 48, 3), np.uint16)
        rgb[:, :, 0] = 65000
        rgb[:, :, 2] = 300
        src.write_bytes(png_bytes(rgb))
        with stub_module('cv2', fake_cv2()):
            out = self.crop(src)
        depth, pixels = read_png(out.read_bytes())
        self.assertEqual(depth, 16)
        # 框 (4,4)-(36,36) 外扩 8% 后为 (1,1)-(38,38)
        self.assertEqual(pixels.shape, (37, 37, 3))
        self.assertEqual(pixels[3, 3].tolist(), [65000, 0, 300])
        self.assertEqual(self.detected[0].getpixel((10, 10)), (253, 0, 1))

    def crop_tags(self, tag_bytes, kind):
        """带原标签裁一次，返回裁切结果旁 .txt 的内容（没写出时为 None）和日志"""
        for p in self.out.iterdir():
            p.unlink()
        src = self.root / 'c.png'
        noisy().save(src)
        src.with_suffix('.txt').write_bytes(tag_bytes)
        options = dict(CROP_OPTIONS, keep_original_tags=True)
        with mock.patch.object(person_crop, 'log') as log:
            result = person_crop.process_image({kind: None}, str(src), options, str(self.out))
        self.assertEqual(result['status'], 'success')
        suffix = {'person': 'full', 'halfbody': 'halfbody'}[kind]
        tag_out = self.out / f'c_{suffix}.txt'
        self.assertEqual([p.name for p in self.out.iterdir() if p.suffix == '.tmp'], [])
        logged = ' '.join(str(call.args[0]) for call in log.call_args_list)
        return (tag_out.read_text(encoding='utf-8') if tag_out.exists() else None), logged

    def test_original_tags_in_gbk_or_bom_utf8_are_copied_as_utf8(self):
        for data in ('长发, 蓝色眼睛'.encode('gbk'), b'\xef\xbb\xbf' + '长发, 蓝色眼睛'.encode('utf-8')):
            with self.subTest(data=data):
                self.assertEqual(self.crop_tags(data, 'halfbody')[0], 'upper body, 长发, 蓝色眼睛')

    def test_undecodable_original_tags_are_not_copied(self):
        tags, logged = self.crop_tags(b'long hair, \xff\xfe', 'person')
        self.assertIsNone(tags)
        self.assertIn('c.txt', logged)
        self.assertEqual(self.crop_tags(b'long hair, \xff\xfe', 'halfbody')[0], 'upper body')

    def test_same_name_outputs_get_a_counter(self):
        src = self.root / 'd.png'
        noisy().save(src)
        first = self.crop(src)
        person_crop.process_image({'person': None}, str(src), CROP_OPTIONS, str(self.out))
        self.assertTrue(first.exists())
        self.assertTrue((self.out / 'd_full_1.png').exists())


class SquareBoxTests(unittest.TestCase):
    def test_square_around_center_clamped_to_image(self):
        self.assertEqual(person_crop.square_box(100, 80, 10, 10, 30, 50, 0.05), (0, 8, 42, 52))
        self.assertEqual(person_crop.square_box(100, 80, 0, 0, 90, 70, 0.1), (0, 0, 99, 80))


class ProtocolTests(unittest.TestCase):
    def run_main(self, commands, **patches):
        stream = io.StringIO(''.join(c if isinstance(c, str) else json.dumps(c) + '\n' for c in commands))
        messages = []
        patches = {'bootstrap': mock.DEFAULT, 'utf8_stdin': mock.Mock(return_value=stream),
                   'load_model': mock.Mock(return_value=object()), **patches}
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.multiple(person_crop, **patches))
            stack.enter_context(mock.patch('purin_proto.emit', side_effect=messages.append))
            stack.enter_context(mock.patch('gpu_diagnostics.resolve_ort_providers',
                                           return_value=['CPUExecutionProvider']))
            person_crop.main()
        return messages

    def test_ready_results_and_errors_identify_image(self):
        init = {'cmd': 'init', 'model_paths': {'person': 'model.onnx'}, 'options': {}}
        messages = self.run_main(
            [init, {'cmd': 'process', 'image_path': 'a.png', 'output_dir': 'out'},
             {'cmd': 'process', 'image_path': 'b.png', 'output_dir': 'out'},
             {'cmd': 'quit'}, {'cmd': 'process', 'image_path': 'c.png', 'output_dir': 'out'}],
            process_image=mock.Mock(side_effect=[{'status': 'skip', 'message': 'empty'}, ValueError('broken')]))
        kinds = [m for m in messages if m['type'] != 'log']
        self.assertEqual(kinds, [
            {'type': 'ready'},
            {'type': 'result', 'image_path': 'a.png', 'status': 'skip', 'message': 'empty'},
            {'type': 'error', 'image_path': 'b.png', 'message': 'broken'},
        ])
        # 加载过程与单张失败的详情走 log，不混进 stderr
        logs = [m['message'] for m in messages if m['type'] == 'log']
        self.assertTrue(any('加载 person 模型' in m for m in logs))
        self.assertTrue(any('处理失败 b.png' in m and 'ValueError' in m for m in logs))

    def test_init_errors(self):
        cases = [
            ([''], '未收到初始化配置'),
            (['{"model_paths": {"person": "m"}}\n'], '未收到初始化配置'),
            (['{not json\n'], 'JSON 解析失败'),
            ([{'cmd': 'init', 'model_paths': {}}], '未指定模型路径'),
        ]
        for commands, expected in cases:
            with self.subTest(expected=expected):
                messages = self.run_main(commands)
                self.assertEqual(len(messages), 1)
                self.assertEqual(messages[0]['type'], 'error')
                self.assertIn(expected, messages[0]['message'])

    def test_model_load_failure_is_reported_before_ready(self):
        messages = self.run_main([{'cmd': 'init', 'model_paths': {'person': 'm.onnx'}}],
                                 load_model=mock.Mock(side_effect=RuntimeError('bad model')))
        self.assertEqual(messages[-1], {'type': 'error', 'message': '模型加载失败: bad model'})

    def test_16_bit_color_without_opencv_fails_alone_instead_of_dropping_to_8_bits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            deep, plain, out = root / 'deep.png', root / 'plain.png', root / 'out'
            deep.write_bytes(png_bytes(np.full((48, 48, 3), 300, np.uint16)))
            noisy().save(plain)
            out.mkdir()
            init = {'cmd': 'init', 'model_paths': {'person': 'm.onnx'}, 'options': CROP_OPTIONS}
            commands = [init] + [{'cmd': 'process', 'image_path': str(p), 'output_dir': str(out)}
                                 for p in (deep, plain)]
            with stub_module('cv2', None):
                messages = self.run_main(commands, detect_with_model=mock.Mock(return_value=[(4, 4, 36, 36, 0.9)]))
            replies = [m for m in messages if m['type'] in ('result', 'error')]
            self.assertEqual([(m['type'], m['image_path']) for m in replies],
                             [('error', str(deep)), ('result', str(plain))])
            self.assertIn('OpenCV', replies[0]['message'])
            self.assertEqual([p.name for p in out.iterdir()], ['plain_full.png'])

    def test_unknown_and_malformed_commands_are_global_errors(self):
        init = {'cmd': 'init', 'model_paths': {'person': 'm.onnx'}}
        messages = self.run_main([init, '{broken\n', ['x'], {'cmd': 'other'}, '\n'])
        errors = [m for m in messages if m['type'] == 'error']
        self.assertEqual(len(errors), 3)
        self.assertTrue(all('image_path' not in m for m in errors))
        self.assertIn('未知命令: other', errors[2]['message'])


if __name__ == '__main__':
    unittest.main()
