#!/usr/bin/env python3
"""
三分法裁切 - 动漫人物检测裁切脚本
使用 deepghs anime detection ONNX 模型，每种裁切类型使用独立的专用检测模型。
通过 stdin 接收 JSON 指令，通过 stdout 输出 JSON 结果。
"""
import sys
import json
import os
import traceback
from functools import lru_cache, partial
import numpy as np
from pathlib import Path

from purin_proto import bootstrap, emit, log_i18n, read_text_compat, utf8_stdin, write_text_atomic

def _diag(msg):
    """诊断信息只写 stderr：stdout 上每张图只回一行结果"""
    sys.stderr.write(f"[person_crop] {msg}\n")
    sys.stderr.flush()

def load_model(model_path, providers):
    """加载 ONNX 模型（使用预先解析好的 providers），GPU 建会话失败时回退 CPU"""
    import onnxruntime as ort
    from gpu_diagnostics import create_session_with_cpu_fallback, quiet_session_options

    return create_session_with_cpu_fallback(
        model_path, providers, quiet_session_options(ort),
        lambda provider, e: _diag(f"⚠ GPU 加载失败 ({e})，回退到 CPU"))

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

def crop_square(img, cx, cy, size, padding_ratio=0.05):
    """以中心点为基准裁切正方形区域"""
    w, h = img.size
    half = size / 2
    pad = size * padding_ratio
    x1 = max(0, int(cx - half - pad))
    y1 = max(0, int(cy - half - pad))
    x2 = min(w, int(cx + half + pad))
    y2 = min(h, int(cy + half + pad))
    return img.crop((x1, y1, x2, y2))

def crop_box(img, x1, y1, x2, y2, padding_ratio=0.05):
    """按检测框裁切，做正方形居中"""
    bw, bh = x2 - x1, y2 - y1
    cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
    size = max(bw, bh)
    return crop_square(img, cx, cy, size, padding_ratio)

def scale_box(x1, y1, x2, y2, factor):
    """以框中心为基准把检测框放大 factor 倍"""
    bw, bh = x2 - x1, y2 - y1
    cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
    nw, nh = bw * factor, bh * factor
    return cx - nw / 2, cy - nh / 2, cx + nw / 2, cy + nh / 2

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
    from image_save import SourceInfo, save_like_source

    # 按原图模式裁切（保留透明通道与位深），检测共用一份 RGB 图，同一输入尺寸只预处理一次
    img = Image.open(image_path)
    img.load()
    letterbox = lru_cache(maxsize=None)(partial(preprocess_image, img.convert('RGB')))
    source = SourceInfo(image_path)
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
            _diag(f"⚠ 原标签读取失败，未复制: {tag_file.name}")
        else:
            orig_tags = text.strip()
    
    def save_crop(cropped_img, out_name):
        """同名冲突加计数器（链式使用时 x_0_full 会撞名互相覆盖）；按源图格式写出。"""
        out_path = Path(output_dir) / out_name
        if out_path.exists():
            base, sfx = out_path.stem, out_path.suffix
            n = 1
            while out_path.exists():
                out_path = Path(output_dir) / f'{base}_{n}{sfx}'
                n += 1
        save_like_source(cropped_img, out_path, source)
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
            cropped = crop_box(img, x1, y1, x2, y2, padding)
            final_path = save_crop(cropped, f'{stem}{suffix}_{name_suffix}{ext}')
            save_tag_for(final_path, tag)
            results.append(f'{label}({c:.2f})')

    if not results:
        return {'status': 'skip', 'message': '未检测到目标'}
    
    return {'status': 'success', 'message': f'裁切: {", ".join(results)}'}

def main():
    """主循环: stdin 读取 JSON, stdout 输出结果"""
    bootstrap()
    stdin_reader = utf8_stdin()

    # 读取初始化配置
    init_line = stdin_reader.readline().strip()
    if not init_line:
        emit({"type": "error", "message": "未收到初始化配置"})
        return
    
    try:
        config = json.loads(init_line)
    except json.JSONDecodeError as e:
        emit({"type": "error", "message": f"JSON 解析失败: {e}"})
        return
    
    # 加载多个模型
    model_paths = config.get('model_paths', {})
    use_gpu = config.get('use_gpu', False)
    options = config.get('options', {})
    
    if not model_paths:
        emit({"type": "error", "message": "未指定模型路径"})
        return
    
    models = {}
    try:
        from gpu_diagnostics import resolve_ort_providers
        providers = resolve_ort_providers(log_i18n, use_gpu=use_gpu)
        for crop_type, path in model_paths.items():
            _diag(f"加载 {crop_type} 模型: {os.path.basename(path)}")
            models[crop_type] = load_model(path, providers)
    except Exception as e:
        emit({"type": "error", "message": f"模型加载失败: {e}"})
        return
    
    loaded_types = list(models.keys())
    _diag(f"已加载 {len(models)} 个模型: {', '.join(loaded_types)}")
    
    emit({"type": "ready"})
    
    # 处理循环
    for line in stdin_reader:
        line = line.strip()
        if not line or line == 'EXIT':
            break
        
        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            emit({"type": "error", "message": "JSON 解析失败"})
            continue
        
        action = cmd.get('action')
        if action != 'process':
            emit({"type": "error", "message": f"未知操作: {action}"})
            continue
        
        image_path = cmd.get('image_path', '')
        output_dir = cmd.get('output_dir', '')

        try:
            result = process_image(models, image_path, options, output_dir)
            emit({"type": "result", "image_path": image_path, **result})
        except Exception as e:
            _diag(f"处理失败 {image_path}: {traceback.format_exc()}")
            emit({"type": "error", "image_path": image_path, "message": str(e)})

if __name__ == '__main__':
    main()
