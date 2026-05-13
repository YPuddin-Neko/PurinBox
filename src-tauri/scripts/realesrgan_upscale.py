#!/usr/bin/env python3
"""Real-ESRGAN 超分推理脚本 (自包含，仅依赖 torch + cv2)
内嵌 RRDBNet / SRVGGNetCompact 架构定义和 tile 推理逻辑。
"""

import argparse, json, math, os, sys, traceback
import cv2
import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

# ── JSON 输出 ──────────────────────────────────────

def emit(data):
    print(json.dumps(data, ensure_ascii=False), flush=True)

def emit_log(msg):
    emit({"type": "log", "message": msg})

def emit_error(msg):
    emit({"type": "error", "message": msg})

def emit_progress(cur, total, fname, status, msg=""):
    emit({"type": "progress", "current": cur, "total": total,
          "filename": fname, "status": status, "message": msg or f"[{cur}/{total}] {fname}"})

# ── 模型架构 ───────────────────────────────────────

class ResidualDenseBlock(nn.Module):
    def __init__(self, nf=64, gc=32):
        super().__init__()
        self.conv1 = nn.Conv2d(nf, gc, 3, 1, 1)
        self.conv2 = nn.Conv2d(nf + gc, gc, 3, 1, 1)
        self.conv3 = nn.Conv2d(nf + 2 * gc, gc, 3, 1, 1)
        self.conv4 = nn.Conv2d(nf + 3 * gc, gc, 3, 1, 1)
        self.conv5 = nn.Conv2d(nf + 4 * gc, nf, 3, 1, 1)
        self.act = nn.LeakyReLU(0.2, True)

    def forward(self, x):
        x1 = self.act(self.conv1(x))
        x2 = self.act(self.conv2(torch.cat((x, x1), 1)))
        x3 = self.act(self.conv3(torch.cat((x, x1, x2), 1)))
        x4 = self.act(self.conv4(torch.cat((x, x1, x2, x3), 1)))
        x5 = self.conv5(torch.cat((x, x1, x2, x3, x4), 1))
        return x5 * 0.2 + x

class RRDB(nn.Module):
    def __init__(self, nf, gc=32):
        super().__init__()
        self.rdb1 = ResidualDenseBlock(nf, gc)
        self.rdb2 = ResidualDenseBlock(nf, gc)
        self.rdb3 = ResidualDenseBlock(nf, gc)

    def forward(self, x):
        out = self.rdb3(self.rdb2(self.rdb1(x)))
        return out * 0.2 + x

class RRDBNet(nn.Module):
    def __init__(self, num_in_ch=3, num_out_ch=3, scale=4, num_feat=64, num_block=23, num_grow_ch=32):
        super().__init__()
        self.scale = scale
        self.conv_first = nn.Conv2d(num_in_ch, num_feat, 3, 1, 1)
        self.body = nn.Sequential(*[RRDB(num_feat, num_grow_ch) for _ in range(num_block)])
        self.conv_body = nn.Conv2d(num_feat, num_feat, 3, 1, 1)
        self.conv_up1 = nn.Conv2d(num_feat, num_feat, 3, 1, 1)
        self.conv_up2 = nn.Conv2d(num_feat, num_feat, 3, 1, 1)
        self.conv_hr = nn.Conv2d(num_feat, num_feat, 3, 1, 1)
        self.conv_last = nn.Conv2d(num_feat, num_out_ch, 3, 1, 1)
        self.lrelu = nn.LeakyReLU(0.2, True)

    def forward(self, x):
        feat = self.conv_first(x)
        feat = feat + self.conv_body(self.body(feat))
        feat = self.lrelu(self.conv_up1(F.interpolate(feat, scale_factor=2, mode="nearest")))
        feat = self.lrelu(self.conv_up2(F.interpolate(feat, scale_factor=2, mode="nearest")))
        return self.conv_last(self.lrelu(self.conv_hr(feat)))

class SRVGGNetCompact(nn.Module):
    def __init__(self, num_in_ch=3, num_out_ch=3, num_feat=64, num_conv=16, upscale=4, act_type="prelu"):
        super().__init__()
        self.upscale = upscale
        body = [nn.Conv2d(num_in_ch, num_feat, 3, 1, 1)]
        body.append(nn.PReLU(num_feat) if act_type == "prelu" else nn.LeakyReLU(0.1, True))
        for _ in range(num_conv):
            body.append(nn.Conv2d(num_feat, num_feat, 3, 1, 1))
            body.append(nn.PReLU(num_feat) if act_type == "prelu" else nn.LeakyReLU(0.1, True))
        body.append(nn.Conv2d(num_feat, num_out_ch * upscale * upscale, 3, 1, 1))
        self.body = nn.ModuleList(body)
        self.upsampler = nn.PixelShuffle(upscale)

    def forward(self, x):
        out = x
        for m in self.body:
            out = m(out)
        out = self.upsampler(out)
        return out + F.interpolate(x, scale_factor=self.upscale, mode="nearest")

# ── Tile 推理 ──────────────────────────────────────

def tile_process(img_tensor, model, scale, tile_size=0, tile_pad=10, device="cpu", half=False):
    """分块推理 + 重叠融合，避免拼接痕迹"""
    if tile_size <= 0:
        with torch.no_grad():
            return model(img_tensor)

    _, _, h, w = img_tensor.shape
    out_h, out_w = h * scale, w * scale
    output = img_tensor.new_zeros((1, 3, out_h, out_w))

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

            tile = img_tensor[:, :, in_y0:in_y1, in_x0:in_x1]
            with torch.no_grad():
                out_tile = model(tile)

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

# ── 模型配置 ───────────────────────────────────────

MODEL_CONFIGS = {
    "realesrgan-x4plus": {
        "build": lambda: RRDBNet(3, 3, 4, 64, 23, 32),
        "scale": 4,
        "url": "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.1.0/RealESRGAN_x4plus.pth",
        "file": "RealESRGAN_x4plus.pth",
    },
    "realesrgan-x4plus-anime": {
        "build": lambda: RRDBNet(3, 3, 4, 64, 6, 32),
        "scale": 4,
        "url": "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.2.4/RealESRGAN_x4plus_anime_6B.pth",
        "file": "RealESRGAN_x4plus_anime_6B.pth",
    },
    "realesr-animevideov3": {
        "build": lambda: SRVGGNetCompact(3, 3, 64, 16, 4, "prelu"),
        "scale": 4,
        "url": "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesr-animevideov3.pth",
        "file": "realesr-animevideov3.pth",
    },
}

SUPPORTED_EXTS = {".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tiff", ".tif"}

# ── 工具函数 ───────────────────────────────────────

def weights_dir():
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "models", "realesrgan_weights")

def download_weights(url, dest):
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    if os.path.exists(dest):
        return
    emit_log(f"正在下载模型权重: {os.path.basename(dest)}")
    torch.hub.download_url_to_file(url, dest, progress=False)
    emit_log("模型权重下载完成")

def detect_device(requested):
    if requested == "cpu":
        return "cpu"
    if requested == "cuda" and torch.cuda.is_available():
        emit_log(f"使用 GPU: {torch.cuda.get_device_name(0)} (CUDA)")
        return "cuda"
    if requested == "mps" and hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
        emit_log("使用 GPU: Apple Silicon (MPS)")
        return "mps"
    if requested == "auto":
        if torch.cuda.is_available():
            emit_log(f"使用 GPU: {torch.cuda.get_device_name(0)} (CUDA)")
            return "cuda"
        if hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
            emit_log("使用 GPU: Apple Silicon (MPS)")
            return "mps"
    emit_log("使用 CPU 推理")
    return "cpu"

def collect_images(path):
    if os.path.isfile(path):
        return [path]
    return sorted(os.path.join(path, f) for f in os.listdir(path)
                  if os.path.splitext(f)[1].lower() in SUPPORTED_EXTS)

# ── 主函数 ─────────────────────────────────────────

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True)
    ap.add_argument("--output", required=True)
    ap.add_argument("--model", default="realesrgan-x4plus")
    ap.add_argument("--scale", type=int, default=4)
    ap.add_argument("--tile", type=int, default=0)
    ap.add_argument("--tta", action="store_true")
    ap.add_argument("--device", default="auto")
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

    # 设备
    device = detect_device(args.device)
    half = device == "cuda"

    # 下载权重
    wpath = os.path.join(weights_dir(), cfg["file"])
    try:
        download_weights(cfg["url"], wpath)
    except Exception as e:
        emit_error(f"模型权重下载失败: {e}")
        sys.exit(1)

    # 构建模型
    emit_log("正在加载模型...")
    model = cfg["build"]()
    loadnet = torch.load(wpath, map_location="cpu", weights_only=True)
    for key in ("params_ema", "params"):
        if key in loadnet:
            loadnet = loadnet[key]
            break
    model.load_state_dict(loadnet, strict=True)
    model.eval().to(device)
    if half:
        model.half()

    native_scale = cfg["scale"]
    out_scale = args.scale
    tile_size = args.tile if args.tile > 0 else 0

    emit_log(f"模型: {args.model}, 设备: {device}, 倍率: {out_scale}x")

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

            # BGR → RGB（模型期望 RGB 输入）
            img = cv2.cvtColor(img, cv2.COLOR_BGR2RGB)
            img_f = img.astype(np.float32) / 255.0
            tensor = torch.from_numpy(img_f).permute(2, 0, 1).unsqueeze(0).to(device)
            if half:
                tensor = tensor.half()

            # 推理
            output = tile_process(tensor, model, native_scale, tile_size, 10, device, half)

            # 后处理: RGB → BGR（cv2 保存需要 BGR）
            output = output.squeeze(0).float().clamp(0, 1).cpu().numpy()
            output = (output.transpose(1, 2, 0) * 255.0).round().astype(np.uint8)
            output = cv2.cvtColor(output, cv2.COLOR_RGB2BGR)

            # 如果目标倍率 != native_scale，resize
            if out_scale != native_scale:
                h, w = img_f.shape[:2]
                new_h, new_w = int(h * out_scale), int(w * out_scale)
                output = cv2.resize(output, (new_w, new_h), interpolation=cv2.INTER_LANCZOS4)

            # alpha 通道处理
            if has_alpha:
                alpha_up = cv2.resize(alpha, (output.shape[1], output.shape[0]), interpolation=cv2.INTER_LANCZOS4)
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
    main()
