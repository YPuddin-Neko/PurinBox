#!/usr/bin/env python3
"""
图片美学评分推理脚本 - 由 Tauri 后端调用
使用 deepghs/anime_aesthetic 的 ONNX 模型对动漫图片进行美学评分

通信协议: JSON lines (stdin/stdout)
- 输入: {"cmd": "init", "model_path": "...", "use_gpu": false}
- 输入: {"cmd": "score_batch", "images": [{"image_path": "...", "copy_files": false, "output_path": "...", "relative_dir": "..."}]}
- 输入: {"cmd": "quit"}
  评分后图片移到（copy_files 时复制到）<output_path 或原目录>/<标签>/<relative_dir>/
- 输出: {"type": "ready"}
- 输出: {"type": "result", "image_path": "...", "label": "masterpiece", "score": 5.8, "confidence": 0.93}
- 输出: {"type": "error", "message": "...", "image_path": "..."}（image_path 仅单图失败时有）
- 输出: {"type": "log", "message": "..."}（可带 i18n_key / i18n_params）
"""

import os
import json
import shutil
import traceback
import numpy as np
from pathlib import Path

from purin_proto import bootstrap, emit, error, log, log_i18n, replace_atomically, result, utf8_stdin


def _finite(x, default=0.0):
    """NaN/Inf 会让 json.dumps 输出非法 JSON（裸 NaN），Rust 侧解析失败后两端互等挂死"""
    try:
        x = float(x)
    except (TypeError, ValueError):
        return default
    return x if x == x and x != float("inf") and x != float("-inf") else default


def _safe_move(src_p, dest_p, keep_src=False):
    """移动：同盘直接 os.rename（O(1) 且本身原子，永不产生半截文件）；
    跨盘（EXDEV）或复制模式才走 copy → 原子 replace →（可选）删源，
    中途被取消杀死时最坏留一份重复，而不是半截损坏文件。
    keep_src=True 为复制模式：工作流输出到临时目录时必须保留原图，否则清理临时目录=删数据集。"""
    if not keep_src:
        try:
            os.rename(str(src_p), str(dest_p))
            return
        except OSError:
            pass  # 跨设备等情形回退到拷贝路径
    replace_atomically(str(dest_p), lambda tmp: shutil.copy2(str(src_p), tmp))
    if not keep_src:
        try:
            os.unlink(str(src_p))
        except OSError:
            pass


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
    """预处理图片 - SwinV2 模型输入
    对齐官方 imgutils generic/classify 实现:
    白底合成转 RGB -> 直接 BILINEAR 拉伸 resize 到 target_size (不保持比例、不 pad 正方形)
    -> (x/255 - 0.5) / 0.5 归一化到 [-1, 1] -> float32
    """
    from PIL import Image

    image = Image.open(image_path)

    # 处理透明通道 (白色背景合成)
    if image.mode not in ["RGB", "RGBA"]:
        image = image.convert("RGBA") if "transparency" in image.info else image.convert("RGB")
    if image.mode == "RGBA":
        background = Image.new("RGB", image.size, (255, 255, 255))
        background.paste(image, mask=image.split()[3])
        image = background

    # 官方实现: 直接拉伸 resize 到目标尺寸
    image = image.resize((target_size, target_size), Image.BILINEAR)

    img_array = np.array(image, dtype=np.float32) / 255.0
    img_array = (img_array - 0.5) / 0.5

    if input_format == "NCHW":
        # HWC -> CHW
        img_array = np.transpose(img_array, (2, 0, 1))
    # 加 batch 维: NCHW 或 NHWC
    img_array = np.expand_dims(img_array, axis=0)

    return img_array.astype(np.float32)

def softmax(x):
    """Softmax 函数"""
    e_x = np.exp(x - np.max(x))
    return e_x / e_x.sum()


def cpu_session(model_path):
    import onnxruntime as ort
    from gpu_diagnostics import quiet_session_options
    options = quiet_session_options(ort)
    options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    session = ort.InferenceSession(model_path, options, providers=["CPUExecutionProvider"])
    return session, session.get_inputs()[0].name


def classify(logits, labels):
    probs = softmax(logits)
    top_idx = int(np.argmax(probs))
    label = labels[top_idx] if top_idx < len(labels) else "unknown"
    score = sum(float(probs[i]) * LABEL_SCORES.get(labels[i], 0)
                for i in range(min(len(probs), len(labels))))
    return label, score, _finite(probs[top_idx])


def move_scored_image(image_path, label, output_path, relative_dir, copy_files):
    src = Path(image_path)
    dest_dir = (Path(output_path) if output_path else src.parent) / label
    if relative_dir:
        dest_dir /= relative_dir
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest_path = dest_dir / src.name
    counter = 1
    while dest_path.exists():
        dest_path = dest_dir / f"{src.stem}_{counter}{src.suffix}"
        counter += 1
    _safe_move(src, dest_path, keep_src=copy_files)

    # 标签沿用图片最终 stem，冲突时保留已有标签。
    for tag_ext in [".txt", ".json", ".caption"]:
        tag_src = src.parent / (src.stem + tag_ext)
        if tag_src.exists():
            tag_dest = dest_dir / f"{dest_path.stem}{tag_ext}"
            counter = 1
            while tag_dest.exists():
                tag_dest = dest_dir / f"{dest_path.stem}_{counter}{tag_ext}"
                counter += 1
            _safe_move(tag_src, tag_dest, keep_src=copy_files)

def main():
    bootstrap()

    session = None
    labels = []
    input_size = 448
    input_name = None
    input_format = "NCHW"
    _model_path_saved = ""

    for line in utf8_stdin():
        line = line.strip()
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
                from gpu_diagnostics import resolve_ort_providers, quiet_session_options

                model_path = cmd["model_path"]
                use_gpu = cmd.get("use_gpu", False)
                _model_path_saved = model_path

                with open(Path(model_path).parent / "meta.json", "r", encoding="utf-8") as f:
                    meta = json.load(f)
                labels = meta.get("labels", ["masterpiece", "best", "great", "good", "normal", "low", "worst"])
                input_size = meta.get("img_size", 448)

                # 选择 provider — 统一流程：探测环境 + 输出日志 + 决定 providers
                providers = resolve_ort_providers(log_i18n, use_gpu=use_gpu)

                sess_options = quiet_session_options(ort)
                sess_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

                session = ort.InferenceSession(model_path, sess_options, providers=providers)
                input_info = session.get_inputs()[0]
                input_name = input_info.name
                input_shape = input_info.shape
                # 4 维输入时 shape[1] 为 3 按 NCHW，否则 shape[3] 为 3 按 NHWC；其余情况按 NCHW
                input_format = ("NHWC" if len(input_shape) == 4 and input_shape[1] != 3
                                and input_shape[3] == 3 else "NCHW")

                emit({"type": "ready"})

            except Exception as e:
                error(f"初始化失败: {traceback.format_exc()}")

        elif command == "score_batch":
            if session is None:
                error("模型未初始化")
                continue

            images = cmd.get("images", [])
            if not images:
                error("score_batch: images 为空")
                continue

            # 批量预处理
            batch_data = []
            valid_indices = []
            for idx, img_cmd in enumerate(images):
                img_path = img_cmd.get("image_path", "")
                try:
                    img_data = preprocess_image(img_path, input_size, input_format)
                    batch_data.append(img_data)
                    valid_indices.append(idx)
                except Exception as e:
                    error(f"预处理失败: {e}", image_path=img_path)

            if not batch_data:
                continue

            batch_tensor = np.concatenate(batch_data, axis=0)

            # 批量推理 (GPU 失败回退 CPU，仍失败则降级为逐张推理)
            all_logits = None
            try:
                outputs = session.run(None, {input_name: batch_tensor})
                all_logits = outputs[0]  # shape: [N, num_labels]
            except Exception as e:
                # GPU 推理失败，回退 CPU 重试
                try:
                    log(f"GPU 批量推理失败，自动回退到 CPU: {type(e).__name__}")
                    session, input_name = cpu_session(_model_path_saved)
                    log("已切换到 CPU 模式，重试批量推理")
                    outputs = session.run(None, {input_name: batch_tensor})
                    all_logits = outputs[0]
                except Exception as e2:
                    log(f"批量推理失败，降级为逐张推理重试: {type(e2).__name__}")

            if all_logits is None:
                # 逐张推理重试，单张失败才对该张报 error
                all_logits = []
                for bi, vi in enumerate(valid_indices):
                    img_path = images[vi].get("image_path", "")
                    try:
                        out_single = session.run(None, {input_name: batch_data[bi]})
                        all_logits.append(out_single[0][0])
                    except Exception as e3:
                        all_logits.append(None)
                        error(f"推理失败: {e3}", image_path=img_path)

            # 逐张处理结果
            for batch_idx, orig_idx in enumerate(valid_indices):
                img_cmd = images[orig_idx]
                image_path = img_cmd.get("image_path", "")
                copy_files = img_cmd.get("copy_files", False)
                output_path = img_cmd.get("output_path", "")
                relative_dir = img_cmd.get("relative_dir", "")

                try:
                    logits = all_logits[batch_idx]
                    if logits is None:
                        continue  # 逐张重试已失败并报过 error
                    top_label, weighted_score, top_prob = classify(logits, labels)
                    move_scored_image(image_path, top_label, output_path, relative_dir, copy_files)

                    result(
                        image_path=image_path,
                        label=top_label,
                        score=round(_finite(weighted_score), 2),
                        confidence=round(top_prob, 4),
                    )
                except Exception as e:
                    error(f"评分失败 [{Path(image_path).name}]: {traceback.format_exc()}", image_path=image_path)

        else:
            error(f"未知命令: {command}")

if __name__ == "__main__":
    main()
