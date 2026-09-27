import io
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import image_save
import person_crop


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
        options = {'person_enabled': True, 'upper_enabled': False,
                   'head_enabled': False, 'eyes_enabled': False}
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


if __name__ == '__main__':
    unittest.main()
