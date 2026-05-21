"""GPU 诊断工具 - 检测 GPU、驱动、CUDA 环境状态"""
import sys
import os
import re
import subprocess


def diagnose_gpu():
    """
    检测 GPU 环境，返回诊断信息列表。
    每条信息是一个字符串，调用方自行选择输出方式（log/emit_log/stderr）。
    
    返回: list[str]
    """
    if sys.platform != "win32":
        if sys.platform == "darwin":
            return ["当前为 macOS 平台，使用 CoreML / MPS 加速"]
        return ["当前平台未找到支持的 GPU 推理后端"]

    lines = []
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
        lines.append(f"GPU: {gpu_name}")
        if driver_version:
            lines.append(f"驱动版本: {driver_version}")
        if driver_cuda_version:
            lines.append(f"驱动支持 CUDA: {driver_cuda_version}")
        if nvcc_version:
            lines.append(f"CUDA Toolkit: {nvcc_version}")
        else:
            lines.append("没有检测到可用的 CUDA 环境")
    else:
        lines.append("原因: 没有找到可用的 NVIDIA GPU 或显卡驱动异常")

    return lines
