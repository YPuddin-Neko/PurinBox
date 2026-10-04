"""按源图格式读写处理结果：沿用源文件的实际编码格式，且不降低画质。

JPEG 的量化不比源图粗、质量不低于 JPEG_MIN_QUALITY，且不做色度抽样；WebP 用无损编码；
PNG/BMP/TIFF/GIF 本身无损。源图的 ICC 配置文件在色彩空间一致时一并写回。
Pillow 把 16 位彩色图读成 8 位、也写不了 16 位彩色图，这类图用 OpenCV 数组读写
（load_array / save_array_like_source）。
"""
import io
import struct
import zlib
from functools import lru_cache

import numpy as np
from PIL import Image

from purin_proto import replace_atomically

# 低质量源图若按原质量再压一次，损失会叠加一代
JPEG_MIN_QUALITY = 95

# Pillow 按原位深保存像素的高位深模式（都是灰度）
_DEEP_GRAY_MODES = {'I', 'I;16', 'I;16L', 'I;16B', 'I;16N', 'F'}
_GRAY_MODES = {'1', 'L', 'LA'} | _DEEP_GRAY_MODES

_TIFF_BITS_PER_SAMPLE = 258
_TIFF_ICC_PROFILE = 34675


class SourceInfo:
    """源图的编码信息（只读文件头，不解码像素）"""

    def __init__(self, path):
        with Image.open(path) as im:
            # 按内容识别，扩展名可能与之不符。带多图段的 JPEG 被识别为 MPO：按 JPEG 处理，
            # 量化表取第一帧的；按 MPO 写出会用默认质量和 4:2:0 抽样
            self.format = 'JPEG' if im.format == 'MPO' else im.format
            self.icc_profile = im.info.get('icc_profile')
            self.qtables = dict(im.quantization) if self.format == 'JPEG' else None
            self.bits = _bits_per_sample(im, path)


def _bits_per_sample(im, path):
    """每个通道的位数：PNG 取 IHDR 的位深，TIFF 取 BitsPerSample，其余格式按 8"""
    if im.format == 'PNG':
        with open(path, 'rb') as f:
            head = f.read(25)
        return head[24] if len(head) == 25 and head[12:16] == b'IHDR' else 8
    if im.format == 'TIFF':
        bits = im.tag_v2.get(_TIFF_BITS_PER_SAMPLE, 8)
        return max(bits) if isinstance(bits, tuple) else int(bits)
    return 8


def pillow_drops_depth(img, source):
    """Pillow 读这张图时丢了位深：16 位的彩色或带透明通道的图会被读成 8 位"""
    return source.bits > 8 and img.mode not in _DEEP_GRAY_MODES


def load_array(path):
    """按原位深读成 OpenCV 数组（BGR / BGRA / 灰度），16 位图为 uint16。

    没有 OpenCV 时这张图报错，不退回 Pillow：Pillow 会把它读成 8 位，结果就降了位深。
    """
    try:
        import cv2
    except ImportError as e:
        raise RuntimeError('未安装 OpenCV（opencv-python-headless），无法按原位深处理这张 16 位图片') from e
    # cv2.imread 在 Windows 上不支持 Unicode 路径，用 numpy 中转
    arr = cv2.imdecode(np.fromfile(path, dtype=np.uint8), cv2.IMREAD_UNCHANGED)
    if arr is None:
        raise ValueError('无法读取图片')
    return arr


def to_8bit(img):
    """高位深灰度图（I;16、I、F 模式）按位深缩放成 8 位灰度，其余模式原样返回。

    这些模式直接 convert('RGB') 会把超过 255 的值截成 255，整张图几乎全白。
    """
    if img.mode not in _DEEP_GRAY_MODES:
        return img
    arr = np.asarray(img, dtype=np.float64)
    if img.mode == 'F' and (arr.size == 0 or arr.max() <= 1.0):
        arr = arr * 255.0
    else:
        arr = arr / 257.0
    return Image.fromarray(np.clip(np.rint(arr), 0, 255).astype(np.uint8))


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
    replace_atomically(str(out_path), lambda tmp: img.save(tmp, format=fmt, **kwargs))


def save_array_like_source(arr, out_path, source):
    """把 OpenCV 的 BGR/BGRA/灰度数组按源图格式写到 out_path。

    16 位结果（源图只可能是 PNG/TIFF）交给 OpenCV 编码，Pillow 写不了 16 位彩色图；
    OpenCV 不写 ICC 配置文件，编码后再把源图的配置文件补进文件。
    """
    if arr.dtype != np.uint8:
        import cv2
        tiff = source.format == 'TIFF'
        ok, buf = cv2.imencode('.tiff' if tiff else '.png', arr)
        if not ok:
            raise ValueError('编码图片失败')
        data = buf.tobytes()
        icc = source.icc_profile
        if icc and _icc_matches(icc, 'L' if arr.ndim == 2 else 'RGB'):
            data = tiff_with_icc(data, icc) if tiff else png_with_icc(data, icc)
        replace_atomically(str(out_path), lambda tmp: _write_bytes(tmp, data))
        return
    if arr.ndim == 3:
        arr = arr[:, :, [2, 1, 0, 3]] if arr.shape[2] == 4 else arr[:, :, ::-1].copy()
    save_like_source(Image.fromarray(arr), out_path, source)


def _write_bytes(path, data):
    with open(path, 'wb') as f:
        f.write(data)


def png_with_icc(png, profile):
    """在 PNG 的 IHDR 之后插入 iCCP 块，去掉已有的 iCCP、sRGB 块（规范要求两者只留一个）"""
    body = b'ICC Profile\0\0' + zlib.compress(profile)
    iccp = struct.pack('>I', len(body)) + b'iCCP' + body + struct.pack('>I', zlib.crc32(b'iCCP' + body))
    out, pos = [png[:8]], 8
    while pos + 8 <= len(png):
        length = struct.unpack('>I', png[pos:pos + 4])[0]
        kind = png[pos + 4:pos + 8]
        end = pos + 12 + length
        if kind not in (b'iCCP', b'sRGB'):
            out.append(png[pos:end])
        if kind == b'IHDR':
            out.append(iccp)
        pos = end
    return b''.join(out)


def tiff_with_icc(tiff, profile):
    """把 ICC 配置文件写进经典 TIFF 第一个 IFD 的 InterColorProfile 标签。

    在文件末尾追加配置文件和改写后的 IFD，再让文件头指向新 IFD；原 IFD 里各标签的数据偏移都不变。
    不是经典 TIFF（如 BigTIFF）时原样返回。
    """
    order = {b'II': '<', b'MM': '>'}.get(tiff[:2])
    if order is None or struct.unpack(order + 'H', tiff[2:4])[0] != 42:
        return tiff
    ifd = struct.unpack(order + 'I', tiff[4:8])[0]
    count = struct.unpack(order + 'H', tiff[ifd:ifd + 2])[0]
    entries_end = ifd + 2 + 12 * count
    entries = [tiff[p:p + 12] for p in range(ifd + 2, entries_end, 12)]
    next_ifd = tiff[entries_end:entries_end + 4]

    def tag(entry):
        return struct.unpack(order + 'H', entry[:2])[0]

    out = bytearray(tiff)
    # IFD 和标签数据都要落在偶数偏移上
    if len(out) % 2:
        out += b'\0'
    icc_offset = len(out)
    out += profile
    if len(out) % 2:
        out += b'\0'
    entries = [e for e in entries if tag(e) != _TIFF_ICC_PROFILE]
    entries.append(struct.pack(order + 'HHII', _TIFF_ICC_PROFILE, 7, len(profile), icc_offset))
    entries.sort(key=tag)
    new_ifd = len(out)
    out += struct.pack(order + 'H', len(entries)) + b''.join(entries) + next_ifd
    out[4:8] = struct.pack(order + 'I', new_ifd)
    return bytes(out)
