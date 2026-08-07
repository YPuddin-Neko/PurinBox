"""GPU 环境探测 — 所有 AI 功能共用

探测流程（严格按此顺序，只读本机环境，不安装/下载任何东西）：
  1. 有没有独立显卡？没有 → 回退 CPU
  2. 有 → 输出显卡型号
  3. 有没有可用的 CUDA 环境？没有 → 回退 CPU 并提示缺 CUDA/cuDNN
  4. 有 → 输出 CUDA / cuDNN 版本，启用 GPU 加速

对外接口：
  probe_gpu()               → GpuInfo，纯探测，不产生日志
  emit_gpu_report(emit, ..) → 按上述顺序输出 i18n 日志，返回最终是否用 GPU
  diagnose_gpu()            → 兼容旧接口，返回 i18n 条目列表
"""
import sys
import os
import re
import glob
import subprocess

# Windows 下隐藏子进程控制台窗口
_NO_WINDOW = 0x08000000 if sys.platform == "win32" else 0


def _run(args, timeout=5):
    """执行命令，返回 stdout（失败返回 None）"""
    try:
        r = subprocess.run(
            args, capture_output=True, text=True, timeout=timeout,
            creationflags=_NO_WINDOW,
        )
        return r.stdout if r.returncode == 0 else None
    except Exception:
        return None


class GpuInfo:
    """GPU 环境探测结果"""

    def __init__(self):
        self.platform = sys.platform
        # 独立显卡
        self.has_discrete_gpu = False
        self.gpu_name = None
        self.vendor = None          # "nvidia" | "apple" | "amd" | "intel" | None
        self.driver_version = None
        # CUDA 环境
        self.driver_cuda_version = None   # 驱动支持的最高 CUDA 版本
        self.cuda_toolkit_version = None  # nvcc 报告的版本
        self.cudnn_version = None
        self.cudnn_found = False

    @property
    def cuda_available(self):
        """CUDA 是否可用：驱动支持 CUDA 即可（Toolkit 非必需，
        onnxruntime-gpu / torch 自带运行时库）"""
        return bool(self.driver_cuda_version or self.cuda_toolkit_version)

    @property
    def can_use_gpu(self):
        """最终是否可用 GPU 加速"""
        if self.vendor == "apple":
            return True  # Apple Silicon 走 CoreML / MPS
        return self.has_discrete_gpu and self.cuda_available


def _detect_nvidia(info):
    """nvidia-smi：显卡型号 + 驱动版本 + 驱动 CUDA 版本"""
    out = _run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"])
    if not out:
        return False
    first = out.strip().splitlines()[0] if out.strip() else ""
    if not first:
        return False
    parts = [p.strip() for p in first.split(",")]
    info.gpu_name = parts[0] if parts else None
    info.driver_version = parts[1] if len(parts) > 1 else None
    info.vendor = "nvidia"
    info.has_discrete_gpu = True

    # 驱动支持的最高 CUDA 版本
    full = _run(["nvidia-smi"])
    if full:
        m = re.search(r"CUDA Version:\s*([\d.]+)", full)
        if m:
            info.driver_cuda_version = m.group(1)
    return True


def _detect_cuda_toolkit(info):
    """nvcc：CUDA Toolkit 版本（可选，缺失不影响 GPU 加速）"""
    candidates = ["nvcc"]
    for var in ("CUDA_PATH", "CUDA_HOME"):
        base = os.environ.get(var)
        if base:
            candidates.append(os.path.join(base, "bin", "nvcc.exe" if sys.platform == "win32" else "nvcc"))

    for exe in candidates:
        out = _run([exe, "--version"])
        if out:
            m = re.search(r"release\s+([\d.]+)", out)
            if m:
                info.cuda_toolkit_version = m.group(1)
                return


def _detect_cudnn(info):
    """查找 cuDNN 库文件并尽力解析版本"""
    search_dirs = []

    for var in ("CUDNN_PATH", "CUDA_PATH", "CUDA_HOME"):
        base = os.environ.get(var)
        if base:
            search_dirs += [os.path.join(base, "bin"), os.path.join(base, "lib"),
                            os.path.join(base, "lib64")]

    # PATH / LD_LIBRARY_PATH 中的目录
    path_var = "PATH" if sys.platform == "win32" else "LD_LIBRARY_PATH"
    search_dirs += [p for p in os.environ.get(path_var, "").split(os.pathsep) if p]

    # onnxruntime-gpu / torch 的 wheel 自带 cuDNN
    try:
        import importlib.util
        for mod in ("onnxruntime", "torch", "nvidia"):
            spec = importlib.util.find_spec(mod)
            if spec and spec.submodule_search_locations:
                for loc in spec.submodule_search_locations:
                    search_dirs += [loc, os.path.join(loc, "lib"),
                                    os.path.join(loc, "capi"),
                                    os.path.join(loc, "cudnn", "lib")]
    except Exception:
        pass

    pattern = "cudnn64_*.dll" if sys.platform == "win32" else "libcudnn.so*"
    for d in search_dirs:
        if not d or not os.path.isdir(d):
            continue
        try:
            hits = glob.glob(os.path.join(d, pattern))
        except Exception:
            continue
        if hits:
            info.cudnn_found = True
            # 从文件名解析主版本：cudnn64_9.dll / libcudnn.so.9.1.0
            for h in hits:
                name = os.path.basename(h)
                m = re.search(r"cudnn64_(\d+)", name) or re.search(r"libcudnn\.so\.([\d.]+)", name)
                if m:
                    info.cudnn_version = m.group(1)
                    return
            return


def _detect_apple(info):
    """macOS：Apple Silicon 判定"""
    out = _run(["sysctl", "-n", "machdep.cpu.brand_string"])
    chip = out.strip() if out else ""
    if chip.startswith("Apple"):
        info.gpu_name = chip
        info.vendor = "apple"
        info.has_discrete_gpu = True
    else:
        info.gpu_name = chip or None
        info.vendor = "intel"


def probe_gpu():
    """探测 GPU 环境，返回 GpuInfo。纯读取，不产生日志。"""
    info = GpuInfo()

    if sys.platform == "darwin":
        _detect_apple(info)
        return info

    if _detect_nvidia(info):
        _detect_cuda_toolkit(info)
        _detect_cudnn(info)

    return info


def emit_gpu_report(emit_i18n, use_gpu=True):
    """按统一流程输出 GPU 探测日志，返回最终是否启用 GPU 加速。

    emit_i18n(key, params=None) — 由调用方提供的日志输出函数
    use_gpu — 用户是否请求 GPU（False 时直接回退 CPU）
    """
    if not use_gpu:
        emit_i18n("gpu.usingCpu")
        return False

    info = probe_gpu()

    # macOS：Apple Silicon 用 CoreML / MPS
    if info.vendor == "apple":
        emit_i18n("gpu.detected", {"name": info.gpu_name or "Apple Silicon"})
        return True

    # 步骤 1：有没有独立显卡
    if not info.has_discrete_gpu:
        emit_i18n("gpu.noGpuFound")
        emit_i18n("gpu.fallbackCpu")
        return False

    # 步骤 2：输出显卡型号
    emit_i18n("gpu.detected", {"name": info.gpu_name or "Unknown"})
    if info.driver_version:
        emit_i18n("gpu.driverVersion", {"version": info.driver_version})

    # 步骤 3：CUDA 环境是否可用
    if not info.cuda_available:
        emit_i18n("gpu.noCudaFound")
        emit_i18n("gpu.fallbackCpu")
        return False

    # 步骤 4：输出 CUDA / cuDNN 版本，启用 GPU
    if info.driver_cuda_version:
        emit_i18n("gpu.driverCuda", {"version": info.driver_cuda_version})
    if info.cuda_toolkit_version:
        emit_i18n("gpu.cudaToolkit", {"version": info.cuda_toolkit_version})
    if info.cudnn_version:
        emit_i18n("gpu.cudnnVersion", {"version": info.cudnn_version})
    elif info.cudnn_found:
        emit_i18n("gpu.cudnnFound")
    else:
        emit_i18n("gpu.cudnnNotFound")

    return True


def resolve_ort_providers(emit_i18n, use_gpu=True, coreml_options=None):
    """onnxruntime 各功能统一的 ExecutionProvider 选择入口。

    先按统一流程探测本机环境并输出日志，再结合 onnxruntime 实际可用的
    provider 决定最终列表。始终把 CPUExecutionProvider 作为兜底。

    coreml_options — 可选 dict，需要定制 CoreML 行为时传入
                     （如 {"MLComputeUnits": "ALL"} 启用 ANE+GPU+CPU）

    返回 providers 列表，可直接传给 ort.InferenceSession。
    """
    cpu_only = ["CPUExecutionProvider"]

    # 步骤 1~4：环境探测 + 日志
    if not emit_gpu_report(emit_i18n, use_gpu=use_gpu):
        return cpu_only

    import onnxruntime as ort
    available = ort.get_available_providers()

    if "CUDAExecutionProvider" in available:
        return ["CUDAExecutionProvider", "CPUExecutionProvider"]
    if "CoreMLExecutionProvider" in available:
        coreml = ("CoreMLExecutionProvider", coreml_options) if coreml_options \
            else "CoreMLExecutionProvider"
        return [coreml, "CPUExecutionProvider"]

    # 本机环境齐备，但当前 onnxruntime 没编入 GPU provider
    # （典型情况：装的是 CPU-only 的 onnxruntime 包）
    emit_i18n("gpu.unavailable")
    emit_i18n("gpu.fallbackCpu")
    return cpu_only


def diagnose_gpu():
    """兼容旧接口：返回 i18n 条目列表 [{"key":..., "params":...}]"""
    items = []
    emit_gpu_report(lambda key, params=None: items.append(
        {"key": key, **({"params": params} if params else {})}
    ))
    return items
