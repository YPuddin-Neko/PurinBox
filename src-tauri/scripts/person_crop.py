#!/usr/bin/env python3
"""
三分法裁切 - 动漫人物检测裁切脚本
使用 deepghs anime detection ONNX 模型，每种裁切类型使用独立的专用检测模型。

通信协议: JSON lines (stdin/stdout)
- 输入: {"cmd": "init", "model_paths": {"person": "...", ...}, "use_gpu": false, "options": {...}}
- 输入: {"cmd": "process", "image_path": "...", "output_dir": "..."}
- 输入: {"cmd": "quit"}
- 输出: {"type": "ready"}
- 输出: {"type": "result", "image_path": "...", "status": "success" | "skip", "message": "..."}
- 输出: {"type": "error", "message": "...", "image_path": "..."}（image_path 仅单图失败时有）
- 输出: {"type": "log", "message": "..."}（可带 i18n_key / i18n_params）
"""
import json
import os
import traceback
from functools import lru_cache, partial
import numpy as np
from pathlib import Path

from purin_proto import (bootstrap, error, log, log_i18n, read_text_compat, ready, result, utf8_stdin,
                         write_text_atomic)

def load_model(model_path, providers):
    """加载 ONNX 模型（使用预先解析好的 providers），GPU 建会话失败时回退 CPU"""
    import onnxruntime as ort
    from gpu_diagnostics import create_session_with_cpu_fallback, quiet_session_options

    return create_session_with_cpu_fallback(
        model_path, providers, quiet_session_options(ort),
        lambda provider, e: log(f"⚠ GPU 加载失败 ({e})，回退到 CPU"))

def preprocess_image(img, input_size=640):
    """预处理 RGB 图: 等比缩放 -> letterbox padding -> 归一化"""
    from PIL import Image
    orig_w, orig_h = img.size

    scale = min(input_size / orig_w, input_size / orig_h)
    new_w, new_h = int(orig_w * scale), int(orig_h * scale)
    img_resized = img.resize((new_w, new_h), Image.BILINEAR)

    canvas = Image.new('RGB', (input_size, input_size), (114, 114, 114))
    pad_x = (input_size - new_w) // 2
    pad_y = (input_size - new_h) // 2
    canvas.paste(img_resized, (pad_x, pad_y))

    arr = np.array(canvas, dtype=np.float32) / 255.0
    arr = arr.transpose(2, 0, 1)  # HWC -> CHW
    arr = np.expand_dims(arr, 0)  # NCHW

    return arr, orig_w, orig_h, scale, pad_x, pad_y

def postprocess_yolo(output, orig_w, orig_h, scale, pad_x, pad_y, conf_thresh=0.3):
    """
    后处理 YOLO 输出，返回检测框列表 [(x1, y1, x2, y2, conf), ...]
    deepghs 模型只有 1 个类别 (class 0 = target)
    支持 YOLOv8 格式: (1, 4+nc, N) 和 YOLOv5 格式: (1, N, 5+nc)
    """
    pred = output[0]

    if len(pred.shape) == 3:
        if pred.shape[1] < pred.shape[2]:
            pred = pred.transpose(0, 2, 1)
        pred = pred[0]
    elif len(pred.shape) == 2:
        pass
    else:
        return []

    boxes = []
    num_cols = pred.shape[1]

    def add_box(det, score):
        """letterbox 坐标的 (cx, cy, w, h) 换算回原图并裁到图内，过小的框丢弃"""
        cx, cy, w, h = det[0], det[1], det[2], det[3]
        x1, y1 = cx - w / 2, cy - h / 2
        x2, y2 = cx + w / 2, cy + h / 2
        x1 = (x1 - pad_x) / scale
        y1 = (y1 - pad_y) / scale
        x2 = (x2 - pad_x) / scale
        y2 = (y2 - pad_y) / scale
        x1 = max(0, min(x1, orig_w))
        y1 = max(0, min(y1, orig_h))
        x2 = max(0, min(x2, orig_w))
        y2 = max(0, min(y2, orig_h))
        if x2 - x1 > 5 and y2 - y1 > 5:
            boxes.append((x1, y1, x2, y2, score))

    # 每行列数：YOLOv8 为 4 + nc，YOLOv5 多一列 obj_conf 为 5 + nc。
    # deepghs 模型只有 1 个类别（v8 为 5 列、v5 为 6 列）；超过 6 列时奇数列按 v5
    is_v5_format = (num_cols % 2 == 1) if num_cols > 6 else (num_cols >= 6)

    if is_v5_format:
        for det in pred:
            obj_conf = det[4]
            if obj_conf < conf_thresh:
                continue
            cls_scores = det[5:]
            cls_id = np.argmax(cls_scores)
            score = obj_conf * cls_scores[cls_id]
            if score < conf_thresh:
                continue
            add_box(det, float(score))
    else:
        for det in pred:
            score = float(np.max(det[4:]))
            if score < conf_thresh:
                continue
            add_box(det, score)

    # NMS
    if len(boxes) > 1:
        boxes.sort(key=lambda b: b[4], reverse=True)
        keep = []
        for box in boxes:
            is_dup = False
            for kept in keep:
                iou = compute_iou(box, kept)
                if iou > 0.5:
                    is_dup = True
                    break
            if not is_dup:
                keep.append(box)
        boxes = keep

    return boxes

def compute_iou(a, b):
    x1 = max(a[0], b[0])
    y1 = max(a[1], b[1])
    x2 = min(a[2], b[2])
    y2 = min(a[3], b[3])
    inter = max(0, x2 - x1) * max(0, y2 - y1)
    area_a = (a[2] - a[0]) * (a[3] - a[1])
    area_b = (b[2] - b[0]) * (b[3] - b[1])
    union = area_a + area_b - inter
    return inter / max(union, 1e-6)

def detect_with_model(sess, letterbox, conf_thresh=0.3):
    """使用模型检测图片，返回检测框列表。letterbox(input_size) 给出该尺寸的预处理结果"""
    input_info = sess.get_inputs()[0]
    input_name = input_info.name
    input_shape = input_info.shape
    input_size = input_shape[2] if len(input_shape) >= 3 else 640
    if isinstance(input_size, str) or input_size <= 0:
        input_size = 640

    arr, orig_w, orig_h, scale, pad_x, pad_y = letterbox(input_size)
    outputs = sess.run(None, {input_name: arr})
    return postprocess_yolo(outputs, orig_w, orig_h, scale, pad_x, pad_y, conf_thresh)

def square_box(width, height, x1, y1, x2, y2, padding_ratio=0.05):
    """以检测框中心为基准、长边为边长取正方形，四周外扩 padding_ratio，裁到图内"""
    size = max(x2 - x1, y2 - y1)
    cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
    half = size / 2
    pad = size * padding_ratio
    return (max(0, int(cx - half - pad)), max(0, int(cy - half - pad)),
            min(width, int(cx + half + pad)), min(height, int(cy + half + pad)))

def scale_box(x1, y1, x2, y2, factor):
    """以框中心为基准把检测框放大 factor 倍"""
    bw, bh = x2 - x1, y2 - y1
    cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
    nw, nh = bw * factor, bh * factor
    return cx - nw / 2, cy - nh / 2, cx + nw / 2, cy + nh / 2

def save_cropped(pixels, box, out_path, source):
    """裁出 box 并按源图格式写出；pixels 是 PIL 图或 OpenCV 数组"""
    from image_save import save_array_like_source, save_like_source
    if isinstance(pixels, np.ndarray):
        x1, y1, x2, y2 = box
        save_array_like_source(np.ascontiguousarray(pixels[y1:y2, x1:x2]), out_path, source)
    else:
        save_like_source(pixels.crop(box), out_path, source)

# 裁切类型：(模型键, 输出名后缀, 结果名, 置信度参数, 标签参数, 放大倍数参数, 边距比例)
CROP_SPECS = (
    ('person', 'full', '全身', 'person_conf', None, None, 0.08),
    ('halfbody', 'halfbody', '半身', 'upper_conf', 'upper_tag', None, 0.06),
    ('head', 'head', '头部', 'head_conf', 'head_tag', 'head_scale', 0.02),
    ('eyes', 'eyes', '眼部', 'eyes_conf', 'eyes_tag', 'eyes_scale', 0.02),
)

def process_image(models, image_path, options, output_dir):
    """处理单张图片 — 每种裁切类型用独立模型检测（models 只含启用的类型）"""
    from PIL import Image
    from image_save import SourceInfo, load_array, pillow_drops_depth, to_8bit

    img = Image.open(image_path)
    img.load()
    source = SourceInfo(image_path)
    # 裁切按源图原样（模式、透明通道、位深）：Pillow 读丢位深的图改用 OpenCV 按原位深读
    pixels = load_array(image_path) if pillow_drops_depth(img, source) else img
    width, height = img.size
    # 检测共用一份 8 位 RGB 图，同一输入尺寸只预处理一次
    letterbox = lru_cache(maxsize=None)(partial(preprocess_image, to_8bit(img).convert('RGB')))
    stem = Path(image_path).stem
    ext = Path(image_path).suffix or '.png'
    results = []

    tag_file = Path(image_path).with_suffix('.txt')
    orig_tags = ''
    if options['keep_original_tags'] and tag_file.exists():
        try:
            text = read_text_compat(tag_file)
        except OSError:
            text = None
        if text is None:
            # 标签读不出来只是不带原标签，不阻断裁切本身；解不开的字节不能硬解成乱码写进新标签
            log(f"⚠ 原标签读取失败，未复制: {tag_file.name}")
        else:
            orig_tags = text.strip()

    def unique_path(out_name):
        """同名冲突加计数器（链式使用时 x_0_full 会撞名互相覆盖）"""
        out_path = Path(output_dir) / out_name
        if out_path.exists():
            base, sfx = out_path.stem, out_path.suffix
            n = 1
            while out_path.exists():
                out_path = Path(output_dir) / f'{base}_{n}{sfx}'
                n += 1
        return out_path

    def save_tag_for(img_out_path, extra_tag=''):
        tags = orig_tags
        if extra_tag:
            tags = f'{extra_tag}, {tags}' if tags else extra_tag
        if tags:
            write_text_atomic(Path(img_out_path).with_suffix('.txt'), tags)

    for kind, name_suffix, label, conf_key, tag_key, scale_key, padding in CROP_SPECS:
        if kind not in models:
            continue
        boxes = detect_with_model(models[kind], letterbox, options[conf_key])
        tag = options[tag_key] if tag_key else ''
        for idx, (x1, y1, x2, y2, c) in enumerate(boxes):
            suffix = f'_{idx}' if len(boxes) > 1 else ''
            if scale_key:
                x1, y1, x2, y2 = scale_box(x1, y1, x2, y2, options[scale_key])
            final_path = unique_path(f'{stem}{suffix}_{name_suffix}{ext}')
            save_cropped(pixels, square_box(width, height, x1, y1, x2, y2, padding), final_path, source)
            save_tag_for(final_path, tag)
            results.append(f'{label}({c:.2f})')

    if not results:
        return {'status': 'skip', 'message': '未检测到目标'}

    return {'status': 'success', 'message': f'裁切: {", ".join(results)}'}

def load_models(config):
    """按 init 命令加载启用的检测模型，返回 {裁切类型: 会话}"""
    from gpu_diagnostics import resolve_ort_providers
    providers = resolve_ort_providers(log_i18n, use_gpu=config.get('use_gpu', False))
    models = {}
    for crop_type, path in config['model_paths'].items():
        log(f"加载 {crop_type} 模型: {os.path.basename(path)}")
        models[crop_type] = load_model(path, providers)
    log(f"已加载 {len(models)} 个模型: {', '.join(models)}")
    return models

def main():
    """主循环: stdin 读取 JSON 命令, stdout 输出结果"""
    bootstrap()
    stdin_reader = utf8_stdin()

    init_line = stdin_reader.readline().strip()
    try:
        config = json.loads(init_line) if init_line else None
    except json.JSONDecodeError as e:
        error(f"JSON 解析失败: {e}")
        return
    if not isinstance(config, dict) or config.get('cmd') != 'init':
        error("未收到初始化配置")
        return
    if not config.get('model_paths'):
        error("未指定模型路径")
        return
    options = config.get('options', {})
    try:
        models = load_models(config)
    except Exception as e:
        error(f"模型加载失败: {e}")
        return
    ready()

    for line in stdin_reader:
        line = line.strip()
        if not line:
            continue
        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            error(f"无法解析命令: {line}")
            continue
        command = cmd.get('cmd') if isinstance(cmd, dict) else None
        if command == 'quit':
            break
        if command != 'process':
            error(f"未知命令: {command}")
            continue

        image_path = cmd.get('image_path', '')
        try:
            result(image_path=image_path, **process_image(models, image_path, options, cmd.get('output_dir', '')))
        except Exception as e:
            log(f"处理失败 {image_path}: {traceback.format_exc()}")
            error(str(e), image_path=image_path)

if __name__ == '__main__':
    main()
