"""GPU 诊断工具 - 检测 GPU、驱动、CUDA 环境状态"""
import sys
import os
import re
import subprocess


def diagnose_gpu():
    """
    检测 GPU 环境，返回诊断信息列表。
    每条信息是一个 dict: {"key": "i18n_key", "params": {...}}
    调用方将其作为 i18n 消息发送给前端翻译。
    
    返回: list[dict]
    """
    if sys.platform != "win32":
        if sys.platform == "darwin":
            return [{"key": "gpu.macPlatform"}]
        return [{"key": "gpu.noBackend"}]

    results = []
    gpu_name = None
    driver_version = None
    driver_cuda_version = None
    nvcc_version = None

    # 1. nvidia-smi: 获取 GPU 名称 + 驱动版本
    try:
        r = subprocess.run(
            ["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"],
            capture_output=True, text=True, timeout=5
        )
        if r.returncode == 0:
            parts = r.stdout.strip().split(", ")
            gpu_name = parts[0] if len(parts) > 0 else None
            driver_version = parts[1] if len(parts) > 1 else None
    except (FileNotFoundError, Exception):
        pass

    # 2. nvidia-smi: 获取驱动支持的最高 CUDA 版本
    try:
        r = subprocess.run(["nvidia-smi"], capture_output=True, text=True, timeout=5)
        if r.returncode == 0:
            m = re.search(r"CUDA Version:\s*([\d.]+)", r.stdout)
            if m:
                driver_cuda_version = m.group(1)
    except (FileNotFoundError, Exception):
        pass

    # 3. nvcc: 检测 CUDA Toolkit 是否安装
    try:
        r = subprocess.run(["nvcc", "--version"], capture_output=True, text=True, timeout=5)
        if r.returncode == 0:
            m = re.search(r"release\s+([\d.]+)", r.stdout)
            if m:
                nvcc_version = m.group(1)
    except (FileNotFoundError, Exception):
        pass

    # 输出诊断信息
    if gpu_name:
        results.append({"key": "gpu.detected", "params": {"name": gpu_name}})
        if driver_version:
            results.append({"key": "gpu.driverVersion", "params": {"version": driver_version}})
        if driver_cuda_version:
            results.append({"key": "gpu.driverCuda", "params": {"version": driver_cuda_version}})
        if nvcc_version:
            results.append({"key": "gpu.cudaToolkit", "params": {"version": nvcc_version}})
        else:
            results.append({"key": "gpu.noCudaEnv"})
    else:
        results.append({"key": "gpu.noGpuFound"})

    return results
