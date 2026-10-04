import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import image_save
from imagefixtures import fake_cv2, png_bytes, read_png, stub_module


def noisy(mode='RGB', size=(48, 48), seed=0):
    rng = np.random.default_rng(seed)
    channels = {'RGB': 3, 'RGBA': 4}[mode]
    return Image.fromarray((rng.random((size[1], size[0], channels)) * 255).astype('uint8'))


def fake_icc(space):
    return bytes(16) + space + bytes(108)


class TempDirTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)


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


class SaveLikeSourceTests(TempDirTest):
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

    def test_mpo_is_written_as_jpeg_with_first_frame_quantization(self):
        src = self.root / 'a.jpg'
        noisy(seed=1).save(src, 'MPO', save_all=True, append_images=[noisy(seed=2)], quality=98)
        out = self.root / 'out.jpg'
        with Image.open(src) as im:
            self.assertEqual(im.format, 'MPO')
            first_frame_tables = dict(im.quantization)
            info = image_save.SourceInfo(src)
            image_save.save_like_source(im.crop((3, 5, 40, 40)), out, info)
        self.assertEqual(info.format, 'JPEG')
        with Image.open(out) as im:
            self.assertEqual(im.format, 'JPEG')
            self.assertEqual(dict(im.quantization), first_frame_tables)
            self.assertEqual(im.layer[0][1:3], (1, 1))

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

    def test_16_bit_gray_keeps_depth_and_gray_profile(self):
        src = self.root / 'gray16.png'
        src.write_bytes(image_save.png_with_icc(png_bytes(np.full((6, 5), 40000, np.uint16)),
                                                fake_icc(b'GRAY')))
        out = self.root / 'out.png'
        with Image.open(src) as im:
            self.assertEqual(im.mode, 'I;16')
            image_save.save_like_source(im.crop((0, 0, 4, 4)), out, image_save.SourceInfo(src))
        with Image.open(out) as im:
            self.assertEqual((im.mode, im.size, im.getpixel((1, 1))), ('I;16', (4, 4), 40000))
            self.assertEqual(im.info.get('icc_profile'), fake_icc(b'GRAY'))

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


class SixteenBitArrayTests(TempDirTest):
    def save16(self, src, arr):
        out = self.root / 'out.png'
        with stub_module('cv2', fake_cv2()):
            image_save.save_array_like_source(arr, out, image_save.SourceInfo(src))
        return out

    def test_png_output_keeps_16_bits_and_source_profile(self):
        src = self.root / 'a.png'
        noisy().save(src, icc_profile=fake_icc(b'RGB '))
        bgr = np.zeros((4, 6, 3), np.uint16)
        bgr[:, :, 2] = 65000
        out = self.save16(src, bgr)
        depth, rgb = read_png(out.read_bytes())
        self.assertEqual(depth, 16)
        self.assertEqual((rgb.shape, int(rgb[0, 0, 0])), ((4, 6, 3), 65000))
        with Image.open(out) as im:
            self.assertEqual(im.info.get('icc_profile'), fake_icc(b'RGB '))
        self.assertEqual([p.name for p in self.root.iterdir() if p.suffix == '.tmp'], [])

    def test_gray_profile_is_not_attached_to_color_output(self):
        src = self.root / 'a.png'
        noisy().convert('L').save(src, icc_profile=fake_icc(b'GRAY'))
        out = self.save16(src, np.zeros((4, 6, 3), np.uint16))
        with Image.open(out) as im:
            self.assertIsNone(im.info.get('icc_profile'))


class IccEmbeddingTests(unittest.TestCase):
    def test_png_profile_follows_header_and_replaces_srgb(self):
        buf = io.BytesIO()
        img = noisy(size=(9, 7))
        img.save(buf, 'PNG')
        data = buf.getvalue()
        # 在 IHDR 后面塞一个 sRGB 块：插入 iCCP 时要去掉它
        srgb = b'\0\0\0\x01sRGB\0' + (0xAECE1CE9).to_bytes(4, 'big')
        data = data[:33] + srgb + data[33:]
        with_icc = image_save.png_with_icc(data, fake_icc(b'RGB '))
        self.assertEqual(with_icc[37:41], b'iCCP')
        self.assertNotIn(b'sRGB', with_icc)
        with Image.open(io.BytesIO(with_icc)) as im:
            self.assertEqual(im.info['icc_profile'], fake_icc(b'RGB '))
            self.assertEqual(im.convert('RGB').tobytes(), img.tobytes())

    def test_tiff_profile_is_added_to_first_ifd(self):
        for profile in (fake_icc(b'RGB '), fake_icc(b'RGB ') + b'\x01'):
            with self.subTest(odd_length=len(profile) % 2 == 1):
                buf = io.BytesIO()
                img = noisy(size=(5, 3))
                img.save(buf, 'TIFF')
                with_icc = image_save.tiff_with_icc(buf.getvalue(), profile)
                with Image.open(io.BytesIO(with_icc)) as im:
                    self.assertEqual(im.info['icc_profile'], profile)
                    self.assertEqual(im.convert('RGB').tobytes(), img.tobytes())
                # 已有配置文件时替换而不是重复
                twice = image_save.tiff_with_icc(with_icc, fake_icc(b'RGB ') + b'\x02\x03')
                with Image.open(io.BytesIO(twice)) as im:
                    self.assertEqual(im.info['icc_profile'], fake_icc(b'RGB ') + b'\x02\x03')

    def test_non_tiff_input_is_returned_unchanged(self):
        self.assertEqual(image_save.tiff_with_icc(b'not a tiff', fake_icc(b'RGB ')), b'not a tiff')


class BitDepthTests(TempDirTest):
    def open(self, name, data):
        path = self.root / name
        path.write_bytes(data)
        return Image.open(path), image_save.SourceInfo(path)

    def test_16_bit_color_is_flagged_but_16_bit_gray_is_not(self):
        cases = [
            ('rgb16.png', np.full((4, 4, 3), 300, np.uint16), 16, True),
            ('rgba16.png', np.full((4, 4, 4), 300, np.uint16), 16, True),
            ('gray16.png', np.full((4, 4), 300, np.uint16), 16, False),
            ('rgb8.png', np.full((4, 4, 3), 30, np.uint8), 8, False),
        ]
        for name, arr, bits, drops in cases:
            with self.subTest(name=name):
                img, source = self.open(name, png_bytes(arr))
                with img:
                    self.assertEqual(source.bits, bits)
                    self.assertEqual(image_save.pillow_drops_depth(img, source), drops)
        src = self.root / 'a.jpg'
        noisy().save(src)
        self.assertEqual(image_save.SourceInfo(src).bits, 8)

    def test_tiff_bits_come_from_bits_per_sample(self):
        src = self.root / 'a.tif'
        noisy().save(src, 'TIFF')
        self.assertEqual(image_save.SourceInfo(src).bits, 8)

    def test_load_array_rejects_undecodable_files(self):
        broken = self.root / 'broken.png'
        broken.write_bytes(b'not an image')
        cv2 = mock.Mock(IMREAD_UNCHANGED=-1)
        cv2.imdecode.return_value = None
        with stub_module('cv2', cv2), self.assertRaisesRegex(ValueError, '无法读取'):
            image_save.load_array(broken)


def pixels(img):
    return np.asarray(img).ravel().tolist()


class To8BitTests(unittest.TestCase):
    def test_16_bit_gray_is_scaled_instead_of_saturated(self):
        img = Image.open(io.BytesIO(png_bytes(np.array([[0, 257, 40000, 65535]], np.uint16))))
        self.assertEqual(img.convert('RGB').getpixel((2, 0)), (255, 255, 255))
        eight = image_save.to_8bit(img)
        self.assertEqual(eight.mode, 'L')
        self.assertEqual(pixels(eight), [0, 1, 156, 255])
        self.assertEqual(eight.convert('RGB').getpixel((2, 0)), (156, 156, 156))

    def test_int_and_float_modes(self):
        self.assertEqual(pixels(image_save.to_8bit(Image.fromarray(np.array([[0, 65535]], np.int32)))), [0, 255])
        self.assertEqual(pixels(image_save.to_8bit(Image.fromarray(np.array([[0.0, 0.5, 1.0]], np.float32)))),
                         [0, 128, 255])

    def test_other_modes_are_returned_as_is(self):
        for img in (noisy(), noisy('RGBA'), noisy().convert('L'), noisy().convert('P')):
            self.assertIs(image_save.to_8bit(img), img)


if __name__ == '__main__':
    unittest.main()
