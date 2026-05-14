"""Windows CUDA DLL 目录注册 — 共用模块
必须在 import onnxruntime 之前调用 register_cuda_dlls()
"""
import os, sys


def register_cuda_dlls():
    """Windows Python 3.8+: 注册 CUDA/cuDNN DLL 目录
    解决 onnxruntime_providers_cuda.dll 找不到依赖的问题。
    非 Windows 平台直接跳过。
    """
    if sys.platform != "win32" or not hasattr(os, "add_dll_directory"):
        return

    cuda_dirs = set()

    def add_dir_with_subdirs(d):
        if os.path.isdir(d):
            cuda_dirs.add(d)
            try:
                for sub in os.listdir(d):
                    sub_path = os.path.join(d, sub)
                    if os.path.isdir(sub_path):
                        cuda_dirs.add(sub_path)
            except PermissionError:
                pass

    def read_reg_env(name):
        import subprocess
        for root in [
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            r"HKCU\Environment",
        ]:
            try:
                result = subprocess.run(
                    ["reg", "query", root, "/v", name],
                    capture_output=True, text=True, creationflags=0x08000000,
                )
                for line in result.stdout.splitlines():
                    line = line.strip()
                    if line.startswith(name):
                        parts = line.split(None, 2)
                        if len(parts) >= 3:
                            return parts[2]
            except Exception:
                pass
        return None

    # 1. CUDA 路径
    cuda_paths = {}
    for key, val in os.environ.items():
        if key in ("CUDA_PATH", "CUDA_HOME") or key.startswith("CUDA_PATH_V"):
            cuda_paths[key] = val
    if "CUDA_PATH" not in cuda_paths:
        reg_val = read_reg_env("CUDA_PATH")
        if reg_val:
            cuda_paths["CUDA_PATH"] = reg_val

    for key, val in cuda_paths.items():
        for d in [
            os.path.join(val, "bin"),
            os.path.join(val, "bin", "x64"),
            os.path.join(val, "lib", "x64"),
        ]:
            if os.path.isdir(d):
                cuda_dirs.add(d)

    # 2. cuDNN 路径
    cudnn_path = os.environ.get("CUDNN_PATH", "")
    if cudnn_path:
        add_dir_with_subdirs(os.path.join(cudnn_path, "bin"))
        add_dir_with_subdirs(os.path.join(cudnn_path, "lib"))

    # 3. PATH 中的 CUDA/cuDNN DLL 目录
    for p in os.environ.get("PATH", "").split(os.pathsep):
        if os.path.isdir(p):
            try:
                for f in os.listdir(p):
                    fl = f.lower()
                    if fl.startswith("cudnn") or fl.startswith("cublas") or fl.startswith("cufft") or fl.startswith("nvinfer"):
                        add_dir_with_subdirs(p)
                        break
            except PermissionError:
                pass

    # 注册
    current_path = os.environ.get("PATH", "")
    new_dirs = []
    for d in sorted(cuda_dirs):
        try:
            os.add_dll_directory(d)
        except OSError:
            pass
        if d not in current_path:
            new_dirs.append(d)
    if new_dirs:
        os.environ["PATH"] = os.pathsep.join(new_dirs) + os.pathsep + current_path
