#!/usr/bin/env python3
"""Real-ESRGAN 超分推理脚本 (onnxruntime 版本)
使用 onnxruntime 进行推理，支持 CUDA / CoreML / CPU。
不依赖 PyTorch。
"""

import argparse, json, math, os, sys
import cv2
import numpy as np

from purin_proto import bootstrap, done, error, log, log_i18n, progress

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
    from gpu_diagnostics import (create_session_with_cpu_fallback, quiet_session_options,
                                 resolve_ort_providers)

    onnx_path = os.path.abspath(onnx_path)

    # 统一流程：探测环境 + 输出日志 + 决定 providers
    # CoreML 启用 ANE+GPU+CPU 全部计算单元
    providers = resolve_ort_providers(
        log_i18n,
        use_gpu=(device != "cpu"),
        coreml_options={"MLComputeUnits": "ALL"},
    )

    sess_opts = quiet_session_options(ort)
    sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

    session = create_session_with_cpu_fallback(
        onnx_path, providers, sess_opts,
        lambda provider, e: log(f"GPU 加载失败 ({e})，回退到 CPU"))

    active_ep = session.get_providers()[0] if session.get_providers() else "CPUExecutionProvider"
    if "CUDA" in active_ep:
        actual_device = "cuda"
    elif "CoreML" in active_ep:
        actual_device = "coreml"
    else:
        actual_device = "cpu"

    return session, actual_device

# ── 主函数 ─────────────────────────────────────────

def main():
    bootstrap()
    from image_save import SourceInfo, save_array_like_source

    ap = argparse.ArgumentParser()
    ap.add_argument("--files", required=True)
    ap.add_argument("--model-path", required=True)
    ap.add_argument("--scale", type=int, default=4)
    ap.add_argument("--tile", type=int, default=0)
    ap.add_argument("--tta", action="store_true")
    ap.add_argument("--device", default="auto")
    args = ap.parse_args()

    with open(args.files, "r", encoding="utf-8") as f:
        files = json.load(f)
    if not files:
        error("未找到任何图片")
        sys.exit(1)

    total = len(files)
    log(f"找到 {total} 张图片")
    onnx_path = args.model_path

    if not os.path.exists(onnx_path):
        error(f"模型文件不存在: {onnx_path}")
        sys.exit(1)

    log("正在加载模型...")
    session, device = create_session(onnx_path, args.device)

    input_name = session.get_inputs()[0].name
    output_name = session.get_outputs()[0].name
    native_scale = 4
    out_scale = args.scale

    device_name = {"coreml": "CoreML", "cuda": "CUDA", "cpu": "CPU"}.get(device, device)
    log(f"模型: {os.path.basename(onnx_path)}, 设备: {device_name}, 倍率: {out_scale}x")

    def infer(tensor):
        if not args.tta:
            return tile_process(tensor, session, input_name, output_name, native_scale, args.tile)
        outputs = []
        for flip_h in [False, True]:
            for rot in [0, 1, 2, 3]:
                t = tensor
                if flip_h:
                    t = t[:, :, :, ::-1].copy()
                if rot > 0:
                    t = np.rot90(t, rot, axes=(2, 3)).copy()
                out = tile_process(t, session, input_name, output_name, native_scale, args.tile)
                if rot > 0:
                    out = np.rot90(out, -rot, axes=(2, 3)).copy()
                if flip_h:
                    out = out[:, :, :, ::-1].copy()
                outputs.append(out)
        return np.mean(outputs, axis=0)

    success, fail = 0, 0
    errors = []
    for i, (fpath, out_path) in enumerate(files):
        fname = os.path.basename(fpath)
        progress(i + 1, total, fname, "processing", f"[{i + 1}/{total}] {fname}")
        try:
            source = SourceInfo(fpath)

            # cv2.imread 在 Windows 上不支持 Unicode 路径，用 numpy 中转
            img = cv2.imdecode(np.fromfile(fpath, dtype=np.uint8), cv2.IMREAD_UNCHANGED)
            if img is None:
                raise ValueError("无法读取图片")

            is_gray = img.ndim == 2
            if is_gray:
                img = cv2.cvtColor(img, cv2.COLOR_GRAY2BGR)
            if img.shape[2] == 4:
                alpha = img[:, :, 3:4]
                img = img[:, :, :3]
                has_alpha = True
            else:
                has_alpha = False

            img = cv2.cvtColor(img, cv2.COLOR_BGR2RGB)
            # 按位深归一化: 16-bit PNG (uint16) 除以 65535，8-bit 除以 255；输出保持源图位深
            is16 = img.dtype == np.uint16
            max_val = 65535.0 if is16 else 255.0
            img_f = img.astype(np.float32) / max_val
            tensor = np.transpose(img_f, (2, 0, 1))[np.newaxis, ...]

            try:
                output = infer(tensor)
            except Exception as inf_err:
                # 仅在明确的显存不足/分配失败时给显存提示，其余原样透传真实错误
                err_lower = str(inf_err).lower()
                oom_keywords = ("out of memory", "alloc", "memory", "oom")
                if any(k in err_lower for k in oom_keywords):
                    raise RuntimeError("GPU 显存不足，请调小“分块大小”或降低图片分辨率") from inf_err
                raise

            output = output.squeeze(0).clip(0, 1)
            output = (np.transpose(output, (1, 2, 0)) * max_val).round().astype(img.dtype)
            output = cv2.cvtColor(output, cv2.COLOR_RGB2BGR)

            # 如果目标倍率 != native_scale，resize
            if out_scale != native_scale:
                h, w = img_f.shape[:2]
                new_h, new_w = int(h * out_scale), int(w * out_scale)
                output = cv2.resize(output, (new_w, new_h), interpolation=cv2.INTER_LANCZOS4)

            if has_alpha:
                alpha_up = cv2.resize(alpha, (output.shape[1], output.shape[0]),
                                      interpolation=cv2.INTER_LANCZOS4)
                if alpha_up.ndim == 2:
                    alpha_up = alpha_up[:, :, np.newaxis]
                output = np.concatenate([output, alpha_up], axis=2)

            if is_gray and not has_alpha:
                output = cv2.cvtColor(output, cv2.COLOR_BGR2GRAY)
            os.makedirs(os.path.dirname(out_path), exist_ok=True)
            save_array_like_source(output, out_path, source)
            success += 1
            progress(i + 1, total, fname, "success", f"[{i+1}/{total}] ✓ {fname}")
        except Exception as e:
            fail += 1
            message = f"[{i+1}/{total}] ✗ {fname}: {e}"
            errors.append(message)
            progress(i + 1, total, fname, "error", message)

    done(success_count=success, fail_count=fail, total=total, errors=errors)

if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as e:
        import traceback
        error(f"脚本异常: {e}\n{traceback.format_exc()}")
        sys.exit(1)
