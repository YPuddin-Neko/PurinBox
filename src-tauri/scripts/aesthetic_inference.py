#!/usr/bin/env python3
"""
图片美学评分推理脚本 - 由 Tauri 后端调用
使用 deepghs/anime_aesthetic 的 ONNX 模型对动漫图片进行美学评分

通信协议: JSON lines (stdin/stdout)
- 输入: {"cmd": "init", "model_path": "...", "use_gpu": false}
- 输入: {"cmd": "score", "image_path": "...", "move_files": true}
- 输入: {"cmd": "quit"}
- 输出: {"type": "ready", "labels": [...]}
- 输出: {"type": "result", "image_path": "...", "label": "masterpiece", "score": 5.8, "probs": {...}}
- 输出: {"type": "error", "message": "..."}
- 输出: {"type": "log", "message": "..."}
"""

import sys
import os
import json
import shutil
import traceback
import numpy as np
from pathlib import Path

def _emit(data):
    """输出 JSON line 到 stdout (Windows GBK 安全)"""
    line = json.dumps(data, ensure_ascii=False) + "\n"
    sys.stdout.buffer.write(line.encode("utf-8"))
    sys.stdout.buffer.flush()

def log(msg):
    _emit({"type": "log", "message": msg})

def error(msg):
    _emit({"type": "error", "message": msg})

def result(data):
    _emit(data)

# 标签对应的加权分数 (用于计算综合分)
LABEL_SCORES = {
    "masterpiece": 6,
    "best": 5,
    "great": 4,
    "good": 3,
    "normal": 2,
    "low": 1,
    "worst": 0,
}

def preprocess_image(image_path, target_size, input_format="NCHW"):
    """预处理图片 - SwinV2 模型输入"""
    from PIL import Image

    image = Image.open(image_path)

    # 处理透明通道
    if image.mode not in ["RGB", "RGBA"]:
        image = image.convert("RGBA") if "transparency" in image.info else image.convert("RGB")
    if image.mode == "RGBA":
        background = Image.new("RGB", image.size, (255, 255, 255))
        background.paste(image, mask=image.split()[3])
        image = background

    # Pad to square (白色填充)
    w, h = image.size
    if w != h:
        max_dim = max(w, h)
        padded = Image.new("RGB", (max_dim, max_dim), (255, 255, 255))
        padded.paste(image, ((max_dim - w) // 2, (max_dim - h) // 2))
        image = padded

    # Resize
    image = image.resize((target_size, target_size), Image.LANCZOS)

    # 转 numpy: float32, 归一化 [0,1]
    img_array = np.array(image, dtype=np.float32) / 255.0

    if input_format == "NCHW":
        # HWC -> NCHW
        img_array = np.transpose(img_array, (2, 0, 1))
        img_array = np.expand_dims(img_array, axis=0)
    else:
        # NHWC: just add batch dim
        img_array = np.expand_dims(img_array, axis=0)

    return img_array

def softmax(x):
    """Softmax 函数"""
    e_x = np.exp(x - np.max(x))
    return e_x / e_x.sum()

def main():
    session = None
    labels = []
    input_size = 448
    input_name = None
    input_format = "NCHW"
    _model_path_saved = ""

    for raw_line in sys.stdin.buffer:
        try:
            line = raw_line.decode("utf-8").strip()
        except UnicodeDecodeError:
            line = raw_line.decode("utf-8", errors="replace").strip()
        if not line:
            continue

        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            error(f"无法解析命令: {line}")
            continue

        command = cmd.get("cmd", "")

        if command == "quit":
            break

        elif command == "init":
            try:
                import onnxruntime as ort

                model_path = cmd["model_path"]
                use_gpu = cmd.get("use_gpu", False)
                _model_path_saved = model_path

                # 读取 meta.json
                model_dir = Path(model_path).parent
                meta_path = model_dir / "meta.json"
                if meta_path.exists():
                    with open(meta_path, "r", encoding="utf-8") as f:
                        meta = json.load(f)
                    labels = meta.get("labels", ["masterpiece", "best", "great", "good", "normal", "low", "worst"])
                    input_size = meta.get("img_size", 448)
                else:
                    labels = ["masterpiece", "best", "great", "good", "normal", "low", "worst"]
                    input_size = 448

                log(f"加载模型: {model_path}")
                log(f"标签: {labels}")
                log(f"输入尺寸: {input_size}x{input_size}")

                # 选择 provider
                providers = []
                if use_gpu:
                    available = ort.get_available_providers()
                    if "CUDAExecutionProvider" in available:
                        providers.append("CUDAExecutionProvider")
                        log("使用 CUDA GPU 加速")
                    elif "CoreMLExecutionProvider" in available:
                        providers.append("CoreMLExecutionProvider")
                        log("使用 CoreML 加速")
                    else:
                        log("GPU 加速不可用，回退到 CPU")
                providers.append("CPUExecutionProvider")

                sess_options = ort.SessionOptions()
                sess_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

                session = ort.InferenceSession(model_path, sess_options, providers=providers)
                input_info = session.get_inputs()[0]
                input_name = input_info.name
                input_shape = input_info.shape
                
                # 自动检测输入格式 NCHW vs NHWC
                # shape: [batch, channels, H, W] -> NCHW
                # shape: [batch, H, W, channels] -> NHWC
                if len(input_shape) == 4:
                    # 如果 dim[1] == 3 且 dim[2] > 3 -> NCHW
                    # 如果 dim[3] == 3 且 dim[1] > 3 -> NHWC
                    d1 = input_shape[1] if isinstance(input_shape[1], int) else -1
                    d3 = input_shape[3] if isinstance(input_shape[3], int) else -1
                    if d1 == 3:
                        input_format = "NCHW"
                    elif d3 == 3:
                        input_format = "NHWC"
                    else:
                        # 默认 NCHW
                        input_format = "NCHW"
                
                log(f"输入格式: {input_format} | 形状: {input_shape}")

                _emit({
                    "type": "ready",
                    "labels": labels,
                    "input_size": input_size,
                    "input_format": input_format,
                })

            except Exception as e:
                error(f"初始化失败: {traceback.format_exc()}")

        elif command == "score":
            if session is None:
                error("模型未初始化")
                continue

            image_path = cmd.get("image_path", "")
            move_files = cmd.get("move_files", True)
            output_path = cmd.get("output_path", "")

            try:
                # 预处理
                img_data = preprocess_image(image_path, input_size, input_format)

                # 推理 (GPU 失败时自动回退 CPU)
                try:
                    outputs = session.run(None, {input_name: img_data})
                except Exception as gpu_err:
                    # CoreML / CUDA 推理失败，自动回退到 CPU
                    import onnxruntime as ort
                    log(f"GPU 推理失败，自动回退到 CPU: {type(gpu_err).__name__}")
                    sess_options = ort.SessionOptions()
                    sess_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
                    session = ort.InferenceSession(
                        _model_path_saved, sess_options,
                        providers=["CPUExecutionProvider"]
                    )
                    input_info = session.get_inputs()[0]
                    input_name = input_info.name
                    log("已切换到 CPU 模式，继续推理")
                    outputs = session.run(None, {input_name: img_data})
                logits = outputs[0][0]

                # Softmax 获取概率
                probs = softmax(logits)

                # 最高分标签
                top_idx = int(np.argmax(probs))
                top_label = labels[top_idx] if top_idx < len(labels) else "unknown"
                top_prob = float(probs[top_idx])

                # 加权分数 (0-6)
                weighted_score = sum(
                    float(probs[i]) * LABEL_SCORES.get(labels[i], 0)
                    for i in range(min(len(probs), len(labels)))
                )

                # 概率字典
                probs_dict = {}
                for i, label in enumerate(labels):
                    if i < len(probs):
                        probs_dict[label] = round(float(probs[i]), 4)

                # 移动文件到对应文件夹
                moved_to = ""
                if move_files:
                    src = Path(image_path)
                    # 如果指定了输出路径，使用输出路径作为基目录
                    if output_path:
                        base_dir = Path(output_path)
                    else:
                        base_dir = src.parent
                    dest_dir = base_dir / top_label
                    dest_dir.mkdir(parents=True, exist_ok=True)
                    dest_path = dest_dir / src.name

                    # 处理文件名冲突
                    if dest_path.exists():
                        stem = src.stem
                        ext = src.suffix
                        counter = 1
                        while dest_path.exists():
                            dest_path = dest_dir / f"{stem}_{counter}{ext}"
                            counter += 1

                    shutil.move(str(src), str(dest_path))
                    moved_to = str(dest_path)

                    # 同时移动关联的标签文件 (.txt, .json, .caption)
                    # 使用实际目标文件名的 stem，确保与图片名一致
                    actual_stem = dest_path.stem
                    for tag_ext in [".txt", ".json", ".caption"]:
                        tag_src = src.parent / (src.stem + tag_ext)
                        if tag_src.exists():
                            tag_dest = dest_dir / f"{actual_stem}{tag_ext}"
                            # 极端情况：标签文件也冲突
                            if tag_dest.exists():
                                tc = 1
                                while tag_dest.exists():
                                    tag_dest = dest_dir / f"{actual_stem}_{tc}{tag_ext}"
                                    tc += 1
                            shutil.move(str(tag_src), str(tag_dest))

                result({
                    "type": "result",
                    "image_path": image_path,
                    "label": top_label,
                    "score": round(weighted_score, 2),
                    "confidence": round(top_prob, 4),
                    "probs": probs_dict,
                    "moved_to": moved_to,
                })

            except Exception as e:
                error(f"评分失败 [{Path(image_path).name}]: {traceback.format_exc()}")

        else:
            error(f"未知命令: {command}")

if __name__ == "__main__":
    main()
