"""按源图格式保存处理结果：沿用源文件的实际编码格式，且不降低画质。

JPEG 的量化不比源图粗、质量不低于 JPEG_MIN_QUALITY，且不做色度抽样；WebP 用无损编码；
PNG/BMP/TIFF/GIF 本身无损。源图的 ICC 配置文件在色彩空间一致时一并写回。
"""
import io
import os
from functools import lru_cache

from PIL import Image

# 低质量源图若按原质量再压一次，损失会叠加一代
JPEG_MIN_QUALITY = 95

_GRAY_MODES = {'1', 'L', 'LA', 'I', 'I;16', 'F'}


class SourceInfo:
    """源图的编码信息（只读文件头，不解码像素）"""

    def __init__(self, path):
        with Image.open(path) as im:
            # 按内容识别，扩展名可能与之不符
            self.format = im.format
            self.icc_profile = im.info.get('icc_profile')
            self.qtables = dict(im.quantization) if im.format == 'JPEG' else None


@lru_cache(maxsize=None)
def _encoder_tables(quality):
    """Pillow 在指定质量下写出的 (亮度, 色度) 量化表"""
    buf = io.BytesIO()
    Image.new('RGB', (8, 8)).save(buf, 'JPEG', quality=quality, subsampling=0)
    buf.seek(0)
    with Image.open(buf) as im:
        tables = im.quantization
    return tuple(tables[0]), tuple(tables[1])


def jpeg_quality(qtables, color=True):
    """每个量化步长都不比源图粗的最低质量，再与 JPEG_MIN_QUALITY 取大"""
    luma = qtables.get(0) if qtables else None
    if luma is None:
        return 100
    others = [t for k, t in qtables.items() if k != 0]
    chroma = [min(v) for v in zip(*others)] if others else list(luma)
    for q in range(1, 101):
        enc_luma, enc_chroma = _encoder_tables(q)
        if all(e <= s for e, s in zip(enc_luma, luma)) and (
                not color or all(e <= s for e, s in zip(enc_chroma, chroma))):
            return max(q, JPEG_MIN_QUALITY)
    return 100


def _icc_matches(profile, mode):
    if mode in _GRAY_MODES:
        expected = b'GRAY'
    elif mode == 'CMYK':
        expected = b'CMYK'
    else:
        expected = b'RGB '
    return len(profile) >= 20 and profile[16:20] == expected


def _flatten_for_jpeg(img):
    """JPEG 不支持透明通道：透明区域按白底合成"""
    if img.mode in ('RGBA', 'LA') or (img.mode == 'P' and 'transparency' in img.info):
        rgba = img.convert('RGBA')
        bg = Image.new('RGB', rgba.size, (255, 255, 255))
        bg.paste(rgba, mask=rgba.getchannel('A'))
        return bg
    if img.mode not in ('RGB', 'L', 'CMYK'):
        return img.convert('RGB')
    return img


def _replace_atomically(out_path, write):
    """先写临时名再原子替换，被取消杀死时不留半截图片顶着正名"""
    tmp = f'{out_path}.tmp'
    try:
        write(tmp)
        os.replace(tmp, out_path)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def save_like_source(img, out_path, source):
    """把 PIL 图按源图格式写到 out_path"""
    fmt = source.format
    kwargs = {}
    if fmt == 'JPEG':
        img = _flatten_for_jpeg(img)
        kwargs.update(quality=jpeg_quality(source.qtables, img.mode != 'L'), subsampling=0)
    elif fmt == 'WEBP':
        kwargs.update(lossless=True, exact=True)
    icc = source.icc_profile
    if icc and fmt in ('JPEG', 'PNG', 'WEBP', 'TIFF') and _icc_matches(icc, img.mode):
        kwargs['icc_profile'] = icc
    _replace_atomically(str(out_path), lambda tmp: img.save(tmp, format=fmt, **kwargs))


def save_array_like_source(arr, out_path, source):
    """把 OpenCV 的 BGR/BGRA/灰度数组按源图格式写到 out_path。

    16 位结果（源图只可能是 PNG/TIFF）交给 OpenCV 编码，Pillow 写不了 16 位彩色图。
    """
    if arr.dtype != 'uint8':
        import cv2
        ext = '.tiff' if source.format == 'TIFF' else '.png'
        ok, buf = cv2.imencode(ext, arr)
        if not ok:
            raise ValueError('编码图片失败')
        _replace_atomically(str(out_path), buf.tofile)
        return
    if arr.ndim == 3:
        arr = arr[:, :, [2, 1, 0, 3]] if arr.shape[2] == 4 else arr[:, :, ::-1].copy()
    save_like_source(Image.fromarray(arr), out_path, source)
