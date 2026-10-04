"""测试共用的工具：任意位深的 PNG，只认这种 PNG 的 cv2 替身（测试环境没有 OpenCV），替换单个模块"""
import contextlib
import struct
import sys
import types
import zlib

import numpy as np

_MISSING = object()


@contextlib.contextmanager
def stub_module(name, module):
    """只替换 sys.modules 里的一项，退出时只还原这一项。
    mock.patch.dict(sys.modules) 退出时会删掉期间新导入的模块，numpy 等 C 扩展再导入会失败"""
    saved = sys.modules.get(name, _MISSING)
    sys.modules[name] = module
    try:
        yield module
    finally:
        if saved is _MISSING:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = saved


_COLOR_TYPES = {1: 0, 2: 4, 3: 2, 4: 6}


def _chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


def png_bytes(arr):
    """把 HxW 或 HxWxC（C 为 1-4）的 uint8 / uint16 数组写成 PNG，通道按 RGB(A) 顺序，每行不做滤波"""
    h, w = arr.shape[:2]
    channels = 1 if arr.ndim == 2 else arr.shape[2]
    depth = 16 if arr.dtype == np.uint16 else 8
    rows = arr.astype('>u2' if depth == 16 else 'u1').reshape(h, -1)
    raw = b''.join(b'\0' + row.tobytes() for row in rows)
    header = struct.pack('>IIBBBBB', w, h, depth, _COLOR_TYPES[channels], 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + _chunk(b'IHDR', header) + _chunk(b'IDAT', zlib.compress(raw))
            + _chunk(b'IEND', b''))


def read_png(data):
    """解析 png_bytes 写出的 PNG：返回 (位深, RGB(A) 顺序的数组)"""
    pos, idat, header = 8, b'', None
    while pos < len(data):
        length = struct.unpack('>I', data[pos:pos + 4])[0]
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + length]
        if kind == b'IHDR':
            header = struct.unpack('>IIBBBBB', body)
        elif kind == b'IDAT':
            idat += body
        pos += 12 + length
    w, h, depth, color_type = header[:4]
    channels = {v: k for k, v in _COLOR_TYPES.items()}[color_type]
    row = w * channels * depth // 8
    raw = zlib.decompress(idat)
    rows = b''.join(raw[y * (row + 1) + 1:(y + 1) * (row + 1)] for y in range(h))
    arr = np.frombuffer(rows, dtype='>u2' if depth == 16 else 'u1').astype(
        np.uint16 if depth == 16 else np.uint8)
    return depth, arr.reshape((h, w) if channels == 1 else (h, w, channels))


def _swap_rb(arr):
    """RGB(A) 与 BGR(A) 互换，灰度原样"""
    if arr.ndim == 3 and arr.shape[2] >= 3:
        arr = arr.copy()
        arr[:, :, [0, 2]] = arr[:, :, [2, 0]]
    return arr


def fake_cv2():
    """cv2 替身：imencode 只写 PNG，imdecode 只读 png_bytes 写出的 PNG；数组按 OpenCV 的 BGR(A) 顺序"""
    def imencode(ext, arr):
        if ext != '.png':
            return False, None
        return True, np.frombuffer(png_bytes(_swap_rb(arr)), dtype=np.uint8)

    def imdecode(buf, flags):
        return _swap_rb(read_png(bytes(buf))[1])

    return types.SimpleNamespace(imencode=imencode, imdecode=imdecode, IMREAD_UNCHANGED=-1)
