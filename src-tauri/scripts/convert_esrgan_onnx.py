#!/usr/bin/env python3
"""Convert Real-ESRGAN PyTorch weights (.pth) to ONNX format.
This script contains the model architecture definitions and exports to ONNX.
"""
import os, sys, argparse
import torch
import torch.nn as nn
import torch.nn.functional as F

# ── Model Architectures ──

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

# ── Model Configs ──

MODELS = {
    "RealESRGAN_x4plus": {
        "build": lambda: RRDBNet(3, 3, 4, 64, 23, 32),
        "url": "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.1.0/RealESRGAN_x4plus.pth",
    },
    "RealESRGAN_x4plus_anime_6B": {
        "build": lambda: RRDBNet(3, 3, 4, 64, 6, 32),
        "url": "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.2.4/RealESRGAN_x4plus_anime_6B.pth",
    },
    "realesr-animevideov3": {
        "build": lambda: SRVGGNetCompact(3, 3, 64, 16, 4, "prelu"),
        "url": "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesr-animevideov3.pth",
    },
}

def download(url, dest):
    if os.path.exists(dest):
        return
    print(f"Downloading {os.path.basename(dest)}...")
    torch.hub.download_url_to_file(url, dest, progress=True)

def convert(name, pth_dir, onnx_dir):
    cfg = MODELS[name]
    pth_path = os.path.join(pth_dir, f"{name}.pth")
    onnx_path = os.path.join(onnx_dir, f"{name}.onnx")

    if os.path.exists(onnx_path):
        print(f"  {name}.onnx already exists, skipping")
        return

    # Download .pth if needed
    download(cfg["url"], pth_path)

    # Build model and load weights
    print(f"  Loading {name}.pth...")
    model = cfg["build"]()
    state = torch.load(pth_path, map_location="cpu", weights_only=True)
    for key in ("params_ema", "params"):
        if key in state:
            state = state[key]
            break
    model.load_state_dict(state, strict=True)
    model.eval()

    # Export to ONNX with dynamic spatial dims
    print(f"  Exporting to ONNX...")
    dummy = torch.randn(1, 3, 64, 64)
    torch.onnx.export(
        model, dummy, onnx_path,
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={
            "input": {0: "batch", 2: "height", 3: "width"},
            "output": {0: "batch", 2: "height", 3: "width"},
        },
        opset_version=17,
        do_constant_folding=True,
    )
    # Get file size
    size_mb = os.path.getsize(onnx_path) / (1024 * 1024)
    print(f"  ✓ {name}.onnx ({size_mb:.1f} MB)")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pth-dir", default="./pth_weights", help="Directory for .pth files")
    ap.add_argument("--onnx-dir", default="./onnx_weights", help="Output directory for .onnx files")
    ap.add_argument("--models", nargs="*", default=list(MODELS.keys()), help="Models to convert")
    args = ap.parse_args()

    os.makedirs(args.pth_dir, exist_ok=True)
    os.makedirs(args.onnx_dir, exist_ok=True)

    for name in args.models:
        if name not in MODELS:
            print(f"Unknown model: {name}")
            continue
        print(f"Converting {name}...")
        convert(name, args.pth_dir, args.onnx_dir)

    print("\nDone! ONNX files are in:", args.onnx_dir)

if __name__ == "__main__":
    main()
