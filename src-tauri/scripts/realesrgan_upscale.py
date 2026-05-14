#!/usr/bin/env python3
"""Real-ESRGAN 超分推理脚本 (onnxruntime 版本)
使用 onnxruntime 进行推理，支持 CUDA / CoreML / CPU。
不依赖 PyTorch。
"""

import argparse, json, math, os, sys
import cv2
import numpy as np

# ── JSON 输出 ──────────────────────────────────────

def emit(data):
    # Windows 中文系统 stdout 默认 GBK，无法编码 ✓✗ 等 Unicode
    # 直接写 bytes 到 stdout.buffer 避免编码错误
    line = json.dumps(data, ensure_ascii=False) + "\n"
    sys.stdout.buffer.write(line.encode("utf-8"))
    sys.stdout.buffer.flush()

def emit_log(msg):
    emit({"type": "log", "message": msg})

def emit_error(msg):
    emit({"type": "error", "message": msg})

def emit_progress(cur, total, fname, status, msg=""):
    emit({"type": "progress", "current": cur, "total": total,
          "filename": fname, "status": status, "message": msg or f"[{cur}/{total}] {fname}"})

# ── 模型配置 ───────────────────────────────────────

MODEL_CONFIGS = {
    "realesrgan-x4plus": {
        "scale": 4,
        "onnx_file": "RealESRGAN_x4plus.onnx",
    },
    "realesrgan-x4plus-anime": {
        "scale": 4,
        "onnx_file": "RealESRGAN_x4plus_anime_6B.onnx",
    },
}

SUPPORTED_EXTS = {".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tiff", ".tif"}

# ── Tile 推理 ──────────────────────────────────────

def tile_process(img_np, session, input_name, output_name, scale, tile_size=0, tile_pad=10):
    """分块推理 + 重叠融合，避免拼接痕迹
    img_np: (1, 3, H, W) float32 numpy array
    """
    if tile_size <= 0:
        return session.run([output_name], {input_name: img_np})[0]

    _, _, h, w = img_np.shape
    out_h, out_w = h * scale, w * scale
    output = np.zeros((1, 3, out_h, out_w), dtype=np.float32)

    tiles_y = math.ceil(h / tile_size)
    tiles_x = math.ceil(w / tile_size)

    for yi in range(tiles_y):
        for xi in range(tiles_x):
            ofs_x = xi * tile_size
            ofs_y = yi * tile_size
            in_x0 = max(ofs_x - tile_pad, 0)
            in_x1 = min(ofs_x + tile_size + tile_pad, w)
            in_y0 = max(ofs_y - tile_pad, 0)
            in_y1 = min(ofs_y + tile_size + tile_pad, h)

            tile = img_np[:, :, in_y0:in_y1, in_x0:in_x1]
            out_tile = session.run([output_name], {input_name: tile})[0]

            # 从 out_tile 中截取不含 pad 的区域
            crop_x0 = (ofs_x - in_x0) * scale
            crop_y0 = (ofs_y - in_y0) * scale
            crop_x1 = crop_x0 + min(tile_size, w - ofs_x) * scale
            crop_y1 = crop_y0 + min(tile_size, h - ofs_y) * scale

            out_x0 = ofs_x * scale
            out_y0 = ofs_y * scale
            out_x1 = min(out_x0 + tile_size * scale, out_w)
            out_y1 = min(out_y0 + tile_size * scale, out_h)

            output[:, :, out_y0:out_y1, out_x0:out_x1] = out_tile[:, :, crop_y0:crop_y1, crop_x0:crop_x1]

    return output

# ── 设备检测 ───────────────────────────────────────

def create_session(onnx_path, device):
    """创建 onnxruntime InferenceSession，自动选择最佳 EP"""
    import onnxruntime as ort

    emit_log(f"onnxruntime {ort.__version__}, 可用 EP: {ort.get_available_providers()}")

    onnx_path = os.path.abspath(onnx_path)

    providers = []
    actual_device = "cpu"

    if device != "cpu":
        available = ort.get_available_providers()

        # CUDA (Windows/Linux)
        if "CUDAExecutionProvider" in available:
            providers.append("CUDAExecutionProvider")
            actual_device = "cuda"
        # CoreML (macOS Apple Silicon)
        elif "CoreMLExecutionProvider" in available:
            providers.append(("CoreMLExecutionProvider", {
                "MLComputeUnits": "ALL",  # Use ANE + GPU + CPU
            }))
            actual_device = "coreml"

    providers.append("CPUExecutionProvider")

    sess_opts = ort.SessionOptions()
    sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

    try:
        session = ort.InferenceSession(onnx_path, sess_options=sess_opts, providers=providers)
    except Exception as e:
        # GPU EP failed (e.g. CoreML doesn't support some ops) — fall back to CPU
        emit_log(f"GPU 加载失败 ({e})，回退到 CPU")
        actual_device = "cpu"
        session = ort.InferenceSession(onnx_path, sess_options=sess_opts, providers=["CPUExecutionProvider"])

    active_ep = session.get_providers()[0] if session.get_providers() else "CPUExecutionProvider"

    if "CUDA" in active_ep:
        try:
            import subprocess
            r = subprocess.run(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"],
                               capture_output=True, text=True, timeout=5)
            gpu_name = r.stdout.strip().split("\n")[0] if r.returncode == 0 else "NVIDIA GPU"
        except Exception:
            gpu_name = "NVIDIA GPU"
        emit_log(f"使用 GPU: {gpu_name} (CUDA)")
    elif "CoreML" in active_ep:
        emit_log("使用 GPU: Apple Silicon (CoreML)")
    else:
        emit_log("使用 CPU 推理")

    return session, actual_device

# ── 工具函数 ───────────────────────────────────────

def collect_images(path):
    if os.path.isfile(path):
        return [path]
    return sorted(os.path.join(path, f) for f in os.listdir(path)
                  if os.path.splitext(f)[1].lower() in SUPPORTED_EXTS)

# ── 主函数 ─────────────────────────────────────────

def main():
    # Windows: 注册 CUDA DLL 目录（必须在 import onnxruntime 之前）
    from cuda_dll_helper import register_cuda_dlls
    register_cuda_dlls()

    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True)
    ap.add_argument("--output", required=True)
    ap.add_argument("--model", default="realesrgan-x4plus")
    ap.add_argument("--scale", type=int, default=4)
    ap.add_argument("--tile", type=int, default=0)
    ap.add_argument("--tta", action="store_true")
    ap.add_argument("--device", default="auto")
    ap.add_argument("--weights-dir", default=None, help="Override weights directory")
    args = ap.parse_args()

    cfg = MODEL_CONFIGS.get(args.model)
    if not cfg:
        emit_error(f"未知模型: {args.model}")
        sys.exit(1)

    files = collect_images(args.input)
    if not files:
        emit_error("未找到任何图片")
        sys.exit(1)

    total = len(files)
    emit_log(f"找到 {total} 张图片")
    os.makedirs(args.output, exist_ok=True)

    # 加载 ONNX 模型
    wdir = args.weights_dir if args.weights_dir else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "models", "realesrgan_weights")
    onnx_path = os.path.join(wdir, cfg["onnx_file"])

    if not os.path.exists(onnx_path):
        emit_error(f"模型文件不存在: {onnx_path}")
        sys.exit(1)

    emit_log("正在加载模型...")
    session, device = create_session(onnx_path, args.device)

    input_name = session.get_inputs()[0].name
    output_name = session.get_outputs()[0].name
    native_scale = cfg["scale"]
    out_scale = args.scale
    tile_size = args.tile if args.tile > 0 else 0

    device_name = {"coreml": "CoreML", "cuda": "CUDA", "cpu": "CPU"}.get(device, device)
    emit_log(f"模型: {args.model}, 设备: {device_name}, 倍率: {out_scale}x")

    success, fail = 0, 0
    for i, fpath in enumerate(files):
        fname = os.path.basename(fpath)
        emit_progress(i + 1, total, fname, "processing")
        try:
            img = cv2.imread(fpath, cv2.IMREAD_UNCHANGED)
            if img is None:
                raise ValueError("无法读取图片")

            # 预处理
            if img.ndim == 2:
                img = cv2.cvtColor(img, cv2.COLOR_GRAY2BGR)
            if img.shape[2] == 4:
                alpha = img[:, :, 3:4]
                img = img[:, :, :3]
                has_alpha = True
            else:
                has_alpha = False

            # BGR → RGB
            img = cv2.cvtColor(img, cv2.COLOR_BGR2RGB)
            img_f = img.astype(np.float32) / 255.0
            # HWC → NCHW
            tensor = np.transpose(img_f, (2, 0, 1))[np.newaxis, ...]

            # 推理
            if args.tta:
                # TTA: 8 种变换取平均
                outputs = []
                for flip_h in [False, True]:
                    for rot in [0, 1, 2, 3]:
                        t = tensor.copy()
                        if flip_h:
                            t = t[:, :, :, ::-1].copy()
                        if rot > 0:
                            t = np.rot90(t, rot, axes=(2, 3)).copy()
                        out = tile_process(t, session, input_name, output_name, native_scale, tile_size)
                        if rot > 0:
                            out = np.rot90(out, -rot, axes=(2, 3)).copy()
                        if flip_h:
                            out = out[:, :, :, ::-1].copy()
                        outputs.append(out)
                output = np.mean(outputs, axis=0)
            else:
                output = tile_process(tensor, session, input_name, output_name, native_scale, tile_size)

            # 后处理: NCHW → HWC, RGB → BGR
            output = output.squeeze(0).clip(0, 1)
            output = (np.transpose(output, (1, 2, 0)) * 255.0).round().astype(np.uint8)
            output = cv2.cvtColor(output, cv2.COLOR_RGB2BGR)

            # 如果目标倍率 != native_scale，resize
            if out_scale != native_scale:
                h, w = img_f.shape[:2]
                new_h, new_w = int(h * out_scale), int(w * out_scale)
                output = cv2.resize(output, (new_w, new_h), interpolation=cv2.INTER_LANCZOS4)

            # alpha 通道处理
            if has_alpha:
                alpha_up = cv2.resize(alpha, (output.shape[1], output.shape[0]),
                                      interpolation=cv2.INTER_LANCZOS4)
                if alpha_up.ndim == 2:
                    alpha_up = alpha_up[:, :, np.newaxis]
                output = np.concatenate([output, alpha_up], axis=2)

            stem = os.path.splitext(fname)[0]
            cv2.imwrite(os.path.join(args.output, f"{stem}.png"), output)
            success += 1
            emit_progress(i + 1, total, fname, "success", f"[{i+1}/{total}] ✓ {fname}")
        except Exception as e:
            fail += 1
            emit_progress(i + 1, total, fname, "error", f"[{i+1}/{total}] ✗ {fname}: {e}")

    emit({"type": "done", "success": success, "fail": fail, "total": total})

if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as e:
        import traceback
        emit_error(f"脚本异常: {e}\n{traceback.format_exc()}")
        sys.exit(1)
