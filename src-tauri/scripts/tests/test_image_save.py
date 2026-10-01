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
import image_save
import person_crop
import purin_proto
import aesthetic_inference
import image_cluster


def noisy(mode='RGB', size=(48, 48), seed=0):
    rng = np.random.default_rng(seed)
    channels = {'RGB': 3, 'RGBA': 4}[mode]
    return Image.fromarray((rng.random((size[1], size[0], channels)) * 255).astype('uint8'))


def fake_icc(space):
    return bytes(16) + space + bytes(108)


class JpegQualityTests(unittest.TestCase):
    def quality_of_source(self, quality):
        buf = io.BytesIO()
        noisy().save(buf, 'JPEG', quality=quality)
        buf.seek(0)
        with Image.open(buf) as im:
            return image_save.jpeg_quality(dict(im.quantization))

    def test_quality_never_below_source_or_floor(self):
        for src, expected in [(100, 100), (98, 98), (96, 96), (95, 95), (80, 95), (40, 95)]:
            self.assertEqual(self.quality_of_source(src), expected, f'源质量 {src}')
        self.assertEqual(image_save.jpeg_quality(None), 100)


class SaveLikeSourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_keeps_actual_format_when_extension_differs(self):
        src = self.root / 'png_inside.jpg'
        noisy().save(src, format='PNG')
        out = self.root / 'out.jpg'
        with Image.open(src) as im:
            image_save.save_like_source(im.crop((0, 0, 16, 16)), out, image_save.SourceInfo(src))
        with Image.open(out) as im:
            self.assertEqual(im.format, 'PNG')
            self.assertEqual(im.size, (16, 16))

    def test_jpeg_keeps_source_quantization(self):
        src = self.root / 'a.jpg'
        noisy().save(src, quality=98)
        out = self.root / 'out.jpg'
        with Image.open(src) as im:
            image_save.save_like_source(im.crop((3, 5, 40, 40)), out, image_save.SourceInfo(src))
        with Image.open(src) as a, Image.open(out) as b:
            self.assertEqual(dict(b.quantization), dict(a.quantization))
            # 不做色度抽样
            self.assertEqual(b.layer[0][1:3], (1, 1))

    def test_webp_is_rewritten_lossless(self):
        src = self.root / 'a.webp'
        noisy().save(src, quality=80)
        out = self.root / 'out.webp'
        with Image.open(src) as im:
            decoded = im.convert('RGB')
        image_save.save_like_source(decoded, out, image_save.SourceInfo(src))
        self.assertEqual(out.read_bytes()[12:16], b'VP8L')
        with Image.open(out) as im:
            self.assertEqual(im.convert('RGB').tobytes(), decoded.tobytes())

    def test_transparent_pixels_flatten_to_white_for_jpeg(self):
        src = self.root / 'a.jpg'
        noisy().save(src, quality=95)
        out = self.root / 'out.jpg'
        image_save.save_like_source(Image.new('RGBA', (8, 8), (0, 0, 0, 0)), out,
                                    image_save.SourceInfo(src))
        with Image.open(out) as im:
            self.assertTrue(all(c > 240 for c in im.convert('RGB').getpixel((4, 4))))

    def test_icc_profile_kept_only_when_color_space_matches(self):
        src = self.root / 'a.png'
        noisy().save(src, icc_profile=fake_icc(b'RGB '))
        info = image_save.SourceInfo(src)
        out, gray = self.root / 'out.png', self.root / 'gray.png'
        image_save.save_like_source(noisy(), out, info)
        image_save.save_like_source(noisy().convert('L'), gray, info)
        with Image.open(out) as im:
            self.assertEqual(im.info.get('icc_profile'), fake_icc(b'RGB '))
        with Image.open(gray) as im:
            self.assertIsNone(im.info.get('icc_profile'))

    def test_array_output_follows_source_format(self):
        src = self.root / 'a.jpg'
        noisy().save(src, quality=97)
        bgr = np.zeros((10, 12, 3), dtype=np.uint8)
        bgr[:, :, 2] = 200  # BGR 的红色通道
        out = self.root / 'out.jpg'
        image_save.save_array_like_source(bgr, out, image_save.SourceInfo(src))
        with Image.open(out) as im:
            self.assertEqual(im.format, 'JPEG')
            self.assertEqual(im.size, (12, 10))
            r, g, b = im.convert('RGB').getpixel((5, 5))
            self.assertGreater(r, 190)
            self.assertLess(max(g, b), 10)


class PersonCropOutputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / 'out'
        self.out.mkdir()
        original = person_crop.detect_with_model
        person_crop.detect_with_model = lambda model, path, conf: [(4, 4, 36, 36, 0.9)]
        self.addCleanup(setattr, person_crop, 'detect_with_model', original)

    def crop(self, src):
        # 与 Rust 在 init 配置里下发的裁切参数同形；models 只含启用的类型
        options = {'person_conf': 0.3, 'upper_conf': 0.5, 'upper_tag': 'upper body',
                   'head_conf': 0.4, 'head_tag': 'head view', 'head_scale': 1.5,
                   'eyes_conf': 0.3, 'eyes_tag': 'eyes view', 'eyes_scale': 2.4,
                   'keep_original_tags': False}
        result = person_crop.process_image({'person': None}, str(src), options, str(self.out))
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

    def crop_tags(self, tag_bytes, kind):
        """带原标签裁一次，返回裁切结果旁 .txt 的内容（没写出时为 None）和 stderr"""
        for p in self.out.iterdir():
            p.unlink()
        src = self.root / 'c.png'
        noisy().save(src)
        src.with_suffix('.txt').write_bytes(tag_bytes)
        options = {'person_conf': 0.3, 'upper_conf': 0.5, 'upper_tag': 'upper body',
                   'keep_original_tags': True}
        with mock.patch('sys.stderr', io.StringIO()) as err:
            result = person_crop.process_image({kind: None}, str(src), options, str(self.out))
        self.assertEqual(result['status'], 'success')
        suffix = {'person': 'full', 'halfbody': 'halfbody'}[kind]
        tag_out = self.out / f'c_{suffix}.txt'
        self.assertEqual([p.name for p in self.out.iterdir() if p.suffix == '.tmp'], [])
        return (tag_out.read_text(encoding='utf-8') if tag_out.exists() else None), err.getvalue()

    def test_original_tags_in_gbk_or_bom_utf8_are_copied_as_utf8(self):
        for data in ('长发, 蓝色眼睛'.encode('gbk'), b'\xef\xbb\xbf' + '长发, 蓝色眼睛'.encode('utf-8')):
            with self.subTest(data=data):
                self.assertEqual(self.crop_tags(data, 'halfbody')[0], 'upper body, 长发, 蓝色眼睛')

    def test_undecodable_original_tags_are_not_copied(self):
        tags, err = self.crop_tags(b'long hair, \xff\xfe', 'person')
        self.assertIsNone(tags)
        self.assertIn('c.txt', err)
        self.assertEqual(self.crop_tags(b'long hair, \xff\xfe', 'halfbody')[0], 'upper body')


class AiProtocolTests(unittest.TestCase):
    def test_emit_replaces_lone_surrogates(self):
        output = io.BytesIO()
        with mock.patch('sys.stdout', types.SimpleNamespace(buffer=output)):
            purin_proto.emit({'type': 'log', 'message': 'path\udcff.png'})
        self.assertEqual(json.loads(output.getvalue()), {'type': 'log', 'message': 'path?.png'})

    def test_crop_ready_and_result_are_typed_and_identify_image(self):
        commands = [{'model_paths': {'person': 'model.onnx'}, 'options': {}},
                    {'action': 'process', 'image_path': 'a.png', 'output_dir': 'out'},
                    {'action': 'process', 'image_path': 'b.png', 'output_dir': 'out'}]
        stream = io.StringIO('\n'.join(map(json.dumps, commands)) + '\nEXIT\n')
        messages = []
        with mock.patch.object(person_crop, 'bootstrap'), \
             mock.patch.object(person_crop, 'utf8_stdin', return_value=stream), \
             mock.patch.object(person_crop, 'emit', side_effect=messages.append), \
             mock.patch.object(person_crop, '_diag'), \
             mock.patch('gpu_diagnostics.resolve_ort_providers', return_value=['CPUExecutionProvider']), \
             mock.patch.object(person_crop, 'load_model', return_value=object()), \
             mock.patch.object(person_crop, 'process_image', side_effect=[
                 {'status': 'skip', 'message': 'empty'}, ValueError('broken')]):
            person_crop.main()
        self.assertEqual(messages, [
            {'type': 'ready'},
            {'type': 'result', 'image_path': 'a.png', 'status': 'skip', 'message': 'empty'},
            {'type': 'error', 'image_path': 'b.png', 'message': 'broken'},
        ])

    def test_crop_init_error_uses_type(self):
        messages = []
        with mock.patch.object(person_crop, 'bootstrap'), \
             mock.patch.object(person_crop, 'utf8_stdin', return_value=io.StringIO('{}\n')), \
             mock.patch.object(person_crop, 'emit', side_effect=messages.append):
            person_crop.main()
        self.assertEqual(messages, [{'type': 'error', 'message': '未指定模型路径'}])

    def test_aesthetic_single_image_uses_batch_and_preserves_sidecar(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            src = root / 'a.png'
            noisy().save(src)
            src.with_suffix('.txt').write_text('tag', encoding='utf-8')
            (root / 'meta.json').write_text(json.dumps({'labels': ['good', 'low'], 'img_size': 8}))
            session = mock.Mock()
            session.get_inputs.return_value = [types.SimpleNamespace(name='input', shape=[None, 3, 8, 8])]
            session.run.return_value = [np.array([[3., 0.]])]
            ort = types.SimpleNamespace(InferenceSession=mock.Mock(return_value=session),
                                        GraphOptimizationLevel=types.SimpleNamespace(ORT_ENABLE_ALL=1))
            commands = [{'cmd': 'init', 'model_path': str(root / 'model.onnx')},
                        {'cmd': 'score_batch', 'images': [{'image_path': str(src), 'copy_files': True,
                                                          'output_path': str(root / 'out')}]},
                        {'cmd': 'quit'}]
            messages = []
            with mock.patch.dict(sys.modules, {'onnxruntime': ort}), \
                 mock.patch.object(aesthetic_inference, 'bootstrap'), \
                 mock.patch.object(aesthetic_inference, 'utf8_stdin', return_value=io.StringIO('\n'.join(map(json.dumps, commands)))), \
                 mock.patch('gpu_diagnostics.resolve_ort_providers', return_value=['CPUExecutionProvider']), \
                 mock.patch('gpu_diagnostics.quiet_session_options', return_value=types.SimpleNamespace()), \
                 mock.patch.object(purin_proto, 'emit', side_effect=messages.append), \
                 mock.patch.object(aesthetic_inference, 'emit', side_effect=messages.append):
                aesthetic_inference.main()
            self.assertEqual([m['type'] for m in messages], ['ready', 'result'])
            self.assertEqual(messages[1]['image_path'], str(src))
            self.assertEqual(messages[1]['label'], 'good')
            self.assertTrue(src.exists())
            self.assertEqual((root / 'out/good/a.png').read_bytes(), src.read_bytes())
            self.assertEqual((root / 'out/good/a.txt').read_text(), 'tag')
            self.assertEqual(session.run.call_args.args[1]['input'].shape, (1, 3, 8, 8))


class ClusterPcaTests(unittest.TestCase):
    def test_pca_caps_dimension_by_samples_and_features(self):
        with mock.patch.object(image_cluster, 'log'):
            for n in (1, 2, 5, 49, 60):
                for width in (1, 8, 128):
                    with self.subTest(samples=n, width=width):
                        values = np.random.default_rng(42).normal(size=(n, width))
                        reduced = image_cluster._pca_reduce(values)
                        expected = min(50, width, n - 1) if n > 1 else width
                        self.assertEqual(reduced.shape, (n, expected))
                        self.assertTrue(np.isfinite(reduced).all())

    def test_hdbscan_without_umap_accepts_five_samples(self):
        import sklearn.cluster
        import sklearn.decomposition
        features = np.random.default_rng(42).normal(size=(5, 100))
        with mock.patch.dict(sys.modules, {'umap': None}), mock.patch.object(image_cluster, 'log'):
            labels = image_cluster.cluster_hdbscan(features, min_cluster_size=2)
        self.assertEqual(labels.shape, (5,))


if __name__ == '__main__':
    unittest.main()
