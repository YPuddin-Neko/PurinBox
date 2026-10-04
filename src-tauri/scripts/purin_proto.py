"""各脚本共用的子进程协议输出与文件工具。

stdout 只写协议行：每行一个 JSON 对象，UTF-8 编码。诊断信息写 stderr。
入口脚本由 Rust 以 `python 脚本.py` 启动，脚本所在目录就是 sys.path[0]，
同目录的本模块和 cuda_dll_helper 等都可以直接 import。
"""
import io
import json
import os
import sys

# 在途临时文件的标记，由 Rust 按每轮任务设置：子进程被强制结束后，Rust 按它找到写了一半的临时文件并删除
TEMP_TAG_ENV = "PURIN_TEMP_TAG"


def bootstrap():
    """入口脚本在 import onnxruntime / torch 之前调用：注册 Windows 的 CUDA DLL 目录"""
    from cuda_dll_helper import register_cuda_dlls
    register_cuda_dlls()


# ── 协议输出 ──────────────────────────────────────

def emit(data):
    """写一行协议 JSON。直接写 UTF-8 字节：中文 Windows 的 stdout 默认按 GBK 编码，✓✗ 等字符会编码失败"""
    line = json.dumps(data, ensure_ascii=False) + "\n"
    sys.stdout.buffer.write(line.encode("utf-8", errors="replace"))
    sys.stdout.buffer.flush()


def log(message):
    emit({"type": "log", "message": message})


def log_i18n(key, params=None):
    """前端按 i18n_key 翻译；message 填 key 本身，查不到译文时前端显示它"""
    data = {"type": "log", "i18n_key": key, "message": key}
    if params:
        data["i18n_params"] = params
    emit(data)


def ready(**fields):
    """常驻脚本初始化完成、可以接收命令"""
    emit({"type": "ready", **fields})


def error(message, **fields):
    """{"type": "error", **fields, "message": ...}，fields 用来带 image_path 等定位信息"""
    emit({"type": "error", **fields, "message": message})


def progress(current, total, filename, status=None, message=None):
    """进度行；status / message 为 None 时不写该键"""
    data = {"type": "progress", "current": current, "total": total, "filename": filename}
    if status is not None:
        data["status"] = status
    if message is not None:
        data["message"] = message
    emit(data)


def result(**fields):
    emit({"type": "result", **fields})


def done(**fields):
    emit({"type": "done", **fields})


def utf8_stdin():
    """按 UTF-8 解码的 stdin。Rust 写入的是 UTF-8，而 Windows 上 sys.stdin 默认按系统代码页解码"""
    return io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8", errors="replace")


# ── 文件工具 ──────────────────────────────────────

def is_under(path, parent):
    """path 是否就是 parent 或在其之下；parent 为空时恒为 False"""
    if not parent:
        return False
    try:
        # Windows 路径大小写不敏感，必须 normcase 后比对，否则手输的大小写变体会让排除失效
        p = os.path.normcase(os.path.abspath(path))
        base = os.path.normcase(os.path.abspath(parent))
        return os.path.commonpath([p, base]) == base
    except ValueError:
        # Windows 上不同盘符的路径没有公共前缀
        return False


def read_text_compat(path):
    """读文本标签文件：先按 UTF-8（可带 BOM），再按严格 GBK（中文 Windows 老工具产出的 ANSI 文件）。

    两种都解不开时返回 None，由调用方决定跳过还是报错；读文件本身出错（OSError）照常抛出。
    换行与文本模式的 open() 一致，\\r\\n 和 \\r 都转成 \\n。
    """
    with open(path, "rb") as f:
        data = f.read()
    for encoding in ("utf-8-sig", "gbk"):
        try:
            text = data.decode(encoding)
        except UnicodeDecodeError:
            continue
        return text.replace("\r\n", "\n").replace("\r", "\n")
    return None


def temp_path(out_path):
    """写 out_path 期间用的临时文件 `<out_path>.<标记>.tmp`。
    标记取环境变量 PURIN_TEMP_TAG，没有设置时为 purin-<进程号>"""
    tag = os.environ.get(TEMP_TAG_ENV) or f"purin-{os.getpid()}"
    return f"{out_path}.{tag}.tmp"


def replace_atomically(out_path, write):
    """write(tmp) 先写到临时文件（见 temp_path），再原子替换为 out_path；出错时删掉临时文件后重新抛出。

    进程在写盘中途被杀时，正名下要么是旧文件要么是完整的新文件，不会是半截文件。
    """
    tmp = temp_path(out_path)
    try:
        write(tmp)
        os.replace(tmp, out_path)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def write_text_atomic(path, text):
    """以 UTF-8 原子写出文本。走文本模式，换行转换与 open(path, "w") 相同"""
    def write(tmp):
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(text)
    replace_atomically(path, write)
