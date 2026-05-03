#!/usr/bin/env python3
"""
AI Tagger 推理脚本 - 由 Tauri 后端调用
使用 onnxruntime Python 包进行 ONNX 模型推理

通信协议: JSON lines (stdin/stdout)
- 输入: {"cmd": "init", "model_path": "...", "tags_path": "...", "use_gpu": false}
- 输入: {"cmd": "tag", "image_path": "...", "general_threshold": 0.35, "character_threshold": 0.85, "enabled_categories": ["general", "character"]}
- 输入: {"cmd": "quit"}
- 输出: {"type": "ready", "info": "...", "input_format": "NHWC", "input_size": 448}
- 输出: {"type": "result", "image_path": "...", "tags": [...], "tag_count": 10}
- 输出: {"type": "error", "message": "..."}
- 输出: {"type": "log", "message": "..."}
"""

import sys
import os
import json
import csv
import traceback
import numpy as np
from pathlib import Path

def log(msg):
    """输出日志到 stdout (JSON line)"""
    print(json.dumps({"type": "log", "message": msg}), flush=True)

def error(msg):
    """输出错误到 stdout (JSON line)"""
    print(json.dumps({"type": "error", "message": msg}), flush=True)

def result(data):
    """输出结果到 stdout (JSON line)"""
    print(json.dumps(data), flush=True)

def preprocess_image(image_path, target_size, input_format):
    """预处理图片，参考 sd-scripts 的实现"""
    from PIL import Image

    image = Image.open(image_path)

    # 处理透明通道
    if image.mode in ("RGBA", "LA") or "transparency" in image.info:
        image = image.convert("RGBA")
        background = Image.new("RGB", image.size, (255, 255, 255))
        background.paste(image, mask=image.split()[3])
        image = background
    elif image.mode != "RGB":
        image = image.convert("RGB")

    image = np.array(image)
    image = image[:, :, ::-1]  # RGB -> BGR

    # pad to square
    h, w = image.shape[:2]
    size = max(h, w)
    pad_x = size - w
    pad_y = size - h
    pad_l = pad_x // 2
    pad_t = pad_y // 2
    image = np.pad(image, ((pad_t, pad_y - pad_t), (pad_l, pad_x - pad_l), (0, 0)),
                   mode="constant", constant_values=255)

    # resize
    from PIL import Image as PILImage
    pil_img = PILImage.fromarray(image[:, :, ::-1])  # BGR -> RGB for PIL
    pil_img = pil_img.resize((target_size, target_size), PILImage.LANCZOS)
    image = np.array(pil_img)
    image = image[:, :, ::-1]  # RGB -> BGR again

    image = image.astype(np.float32)

    if input_format == "NCHW":
        # [H, W, C] -> [C, H, W]
        image = image.transpose(2, 0, 1)
        # normalize to [-1, 1]
        image = image / 127.5 - 1.0
        return image[np.newaxis, ...]  # [1, C, H, W]
    else:
        # NHWC: [1, H, W, C], values in [0, 255]
        return image[np.newaxis, ...]  # [1, H, W, C]

def load_tags_csv(csv_path):
    """从 CSV 加载标签定义"""
    tags = []
    category_map = {9: "rating", 0: "general", 4: "character", 1: "artist", 3: "copyright", 5: "meta"}
    with open(csv_path, "r", encoding="utf-8") as f:
        reader = csv.reader(f)
        header = next(reader)
        for row in reader:
            if len(row) >= 3:
                name = row[1]
                cat_id = int(row[2])
                category = category_map.get(cat_id, "general")
                tags.append({"name": name, "category": category})
    return tags

def load_tags_json(json_path):
    """从 JSON 加载标签定义 (CL Tagger 格式)"""
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    tags = []
    for idx_str in sorted(data.keys(), key=int):
        info = data[idx_str]
        tag_name = info.get("tag", "")
        category = info.get("category", "General").lower()
        tags.append({"name": tag_name, "category": category})
    return tags

def detect_model_format(session):
    """检测模型输入格式"""
    inp = session.get_inputs()[0]
    shape = inp.shape  # e.g. [1, 448, 448, 3] or [1, 3, 448, 448] or ['N', 3, 448, 448]

    # 过滤掉动态维度
    dims = []
    for d in shape:
        if isinstance(d, int) and d > 0:
            dims.append(d)
        else:
            dims.append(-1)

    if len(dims) == 4:
        if dims[3] == 3 or dims[3] == 1:
            # NHWC: [B, H, W, C]
            size = dims[1] if dims[1] > 0 else (dims[2] if dims[2] > 0 else 448)
            return "NHWC", size
        elif dims[1] == 3 or dims[1] == 1:
            # NCHW: [B, C, H, W]
            size = dims[2] if dims[2] > 0 else (dims[3] if dims[3] > 0 else 448)
            return "NCHW", size

    # fallback
    return "NHWC", 448

def main():
    # Windows Python 3.8+: 必须在 import onnxruntime 前注册 CUDA DLL 目录
    # 否则 onnxruntime_providers_cuda.dll 加载时找不到 CUDA/cuDNN 依赖
    if sys.platform == "win32" and hasattr(os, "add_dll_directory"):
        cuda_dirs = set()

        def add_dir_with_subdirs(d):
            """添加目录及其子目录"""
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
            """从 Windows 注册表读取环境变量（GUI 进程可能没有最新值）"""
            import subprocess
            for root in [r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment", r"HKCU\Environment"]:
                try:
                    result = subprocess.run(["reg", "query", root, "/v", name],
                                          capture_output=True, text=True, creationflags=0x08000000)
                    for line in result.stdout.splitlines():
                        line = line.strip()
                        if line.startswith(name):
                            parts = line.split(None, 2)
                            if len(parts) >= 3:
                                return parts[2]
                except Exception:
                    pass
            return None

        # 1. CUDA 路径：从环境变量读取 + 注册表回退
        cuda_paths = {}
        for key, val in os.environ.items():
            if key in ("CUDA_PATH", "CUDA_HOME") or key.startswith("CUDA_PATH_V"):
                cuda_paths[key] = val
        # 注册表补充
        if "CUDA_PATH" not in cuda_paths:
            reg_val = read_reg_env("CUDA_PATH")
            if reg_val:
                cuda_paths["CUDA_PATH"] = reg_val
                log(f"从注册表读取 CUDA_PATH={reg_val}")

        for key, val in cuda_paths.items():
            bin_dir = os.path.join(val, "bin")
            bin_x64 = os.path.join(val, "bin", "x64")  # cuDNN 9.x
            lib_x64 = os.path.join(val, "lib", "x64")
            for d in [bin_dir, bin_x64, lib_x64]:
                if os.path.isdir(d):
                    cuda_dirs.add(d)

        # 2. cuDNN 路径：从环境变量读取
        cudnn_path = os.environ.get("CUDNN_PATH", "")
        if cudnn_path:
            # cuDNN 9.x 结构: bin/12.x 子目录
            add_dir_with_subdirs(os.path.join(cudnn_path, "bin"))
            add_dir_with_subdirs(os.path.join(cudnn_path, "lib"))

        # 3. 搜索 PATH 中包含 CUDA/cuDNN DLL 的目录 + 子目录
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

        # 注册所有目录：同时加到 PATH 和 add_dll_directory
        # onnxruntime 的 C++ 层用 LoadLibrary 加载 cuDNN，只看 PATH
        current_path = os.environ.get("PATH", "")
        new_dirs = []
        for d in sorted(cuda_dirs):
            try:
                os.add_dll_directory(d)
            except OSError:
                pass
            if d not in current_path:
                new_dirs.append(d)
                log(f"注册 DLL 目录: {d}")
        if new_dirs:
            os.environ["PATH"] = os.pathsep.join(new_dirs) + os.pathsep + current_path

    import onnxruntime as ort

    session = None
    tags = []
    input_format = "NHWC"
    input_size = 448
    input_name = None

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            cmd = json.loads(line)
        except json.JSONDecodeError as e:
            error(f"JSON 解析错误: {e}")
            continue

        try:
            if cmd["cmd"] == "init":
                model_path = cmd["model_path"]
                tags_path = cmd["tags_path"]
                use_gpu = cmd.get("use_gpu", False)

                # 检测模型格式
                is_onnx_model = model_path.lower().endswith(".onnx")

                # === ONNX Runtime 后端 ===
                log(f"Python onnxruntime v{ort.__version__}")
                log(f"可用 providers: {ort.get_available_providers()}")

                available = ort.get_available_providers()
                gpu_provider = None
                if use_gpu:
                    if "CUDAExecutionProvider" in available:
                        gpu_provider = "CUDAExecutionProvider"
                        log("使用 GPU (CUDA) 加速")
                    elif sys.platform == "darwin":
                        if is_onnx_model:
                            log("⚠ ONNX 模型不支持 MPS (Metal) 加速")
                            log("macOS: 使用 CPU 推理")
                        else:
                            log("macOS: 使用 CPU 推理")
                    else:
                        log("GPU 加速不可用，使用 CPU 推理")

                if gpu_provider:
                    providers = [gpu_provider, "CPUExecutionProvider"]
                else:
                    providers = ["CPUExecutionProvider"]
                    if not use_gpu:
                        log("使用 CPU 推理")

                log(f"加载模型: {model_path}")

                # GPU 模式下打印诊断信息（仅 Windows CUDA）
                if use_gpu and sys.platform == "win32" and gpu_provider == "CUDAExecutionProvider":
                    import ctypes, glob
                    cudnn_found = False
                    for p in os.environ.get("PATH", "").split(os.pathsep):
                        cudnn_dlls = glob.glob(os.path.join(p, "cudnn*.dll"))
                        if cudnn_dlls:
                            log(f"cuDNN DLL 路径: {p}")
                            for dll in cudnn_dlls[:3]:
                                log(f"  {os.path.basename(dll)}")
                            cudnn_found = True
                            break
                    if not cudnn_found:
                        log("⚠ 未在 PATH 中找到 cuDNN DLL (cudnn*.dll)")

                # 尝试创建 session，GPU 失败时自动回退 CPU
                try:
                    session = ort.InferenceSession(model_path, providers=providers)
                except Exception as e:
                    if gpu_provider and gpu_provider in providers:
                        err_msg = str(e)
                        log(f"⚠ {gpu_provider} 加载失败")
                        if "cuDNN" in err_msg:
                            log("原因: 未找到 cuDNN 9.x — 请安装 cuDNN 9.x for CUDA 12.x")
                            log("下载: https://developer.nvidia.com/cudnn-downloads")
                        elif "CUDA" in err_msg:
                            log("原因: CUDA 运行时未找到 — 请确认 CUDA 12.x 已安装且在 PATH 中")
                        else:
                            log(f"原因: {err_msg[:200]}")
                        log("自动回退到 CPU 推理")
                        providers = ["CPUExecutionProvider"]
                        session = ort.InferenceSession(model_path, providers=providers)
                    else:
                        raise

                actual_providers = session.get_providers()
                log(f"实际使用 providers: {actual_providers}")

                # 检测输入格式
                input_name = session.get_inputs()[0].name
                input_format, detected_size = detect_model_format(session)
                actual_info = f"onnxruntime {ort.__version__}, providers: {actual_providers}"

                # 共通：input_size 和标签加载
                override_size = cmd.get("input_size", 0)
                if override_size and override_size > 0:
                    input_size = override_size
                else:
                    input_size = detected_size if detected_size > 0 else 448
                log(f"输入格式: {input_format}, 输入大小: {input_size}x{input_size} (检测: {detected_size})")

                # 加载标签
                if tags_path.endswith(".json"):
                    tags = load_tags_json(tags_path)
                else:
                    tags = load_tags_csv(tags_path)
                log(f"已加载 {len(tags)} 个标签定义")

                result({
                    "type": "ready",
                    "info": actual_info,
                    "input_format": input_format,
                    "input_size": input_size,
                    "tag_count": len(tags),
                })

            elif cmd["cmd"] == "tag":
                if session is None:
                    error("模型未初始化，请先发送 init 命令")
                    continue

                image_path = cmd["image_path"]
                general_threshold = cmd.get("general_threshold", 0.35)
                character_threshold = cmd.get("character_threshold", 0.85)
                enabled_categories = set(cmd.get("enabled_categories", ["general", "character"]))

                # 预处理
                img_data = preprocess_image(image_path, input_size, input_format)

                # 推理
                outputs = session.run(None, {input_name: img_data})
                probs = outputs[0][0]  # shape: [num_tags]

                # 对 NCHW 模型的输出可能需要 sigmoid
                if input_format == "NCHW":
                    probs = 1 / (1 + np.exp(-probs))

                # 筛选标签
                selected_tags = []
                for idx, prob in enumerate(probs):
                    if idx >= len(tags):
                        break
                    tag = tags[idx]
                    cat = tag["category"]

                    if cat not in enabled_categories:
                        continue

                    threshold = character_threshold if cat == "character" else general_threshold
                    if float(prob) >= threshold:
                        selected_tags.append(tag["name"])

                # 写入 txt 文件
                stem = Path(image_path).stem
                parent = Path(image_path).parent
                txt_path = parent / f"{stem}.txt"
                with open(txt_path, "w", encoding="utf-8") as f:
                    f.write(", ".join(selected_tags))

                result({
                    "type": "result",
                    "image_path": image_path,
                    "tags": selected_tags,
                    "tag_count": len(selected_tags),
                })

            elif cmd["cmd"] == "quit":
                log("推理进程退出")
                break

            else:
                error(f"未知命令: {cmd['cmd']}")

        except Exception as e:
            error(f"{traceback.format_exc()}")

if __name__ == "__main__":
    main()
