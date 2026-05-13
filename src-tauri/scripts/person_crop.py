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
import numpy as np
from pathlib import Path

def load_model(model_path, use_gpu=False):
    """加载 ONNX 模型"""
    import onnxruntime as ort
    
    if not os.path.exists(model_path):
        raise FileNotFoundError(f"模型文件不存在: {model_path}")
    
    providers = ['CPUExecutionProvider']
    if use_gpu:
        available = ort.get_available_providers()
        if 'CUDAExecutionProvider' in available:
            providers = ['CUDAExecutionProvider', 'CPUExecutionProvider']
        elif 'CoreMLExecutionProvider' in available:
            providers = ['CoreMLExecutionProvider', 'CPUExecutionProvider']
    
    sess = ort.InferenceSession(model_path, providers=providers)
    return sess

def preprocess_image(image_path, input_size=640):
    """预处理图片: 读取 -> 等比缩放 -> letterbox padding -> 归一化"""
    from PIL import Image
    img = Image.open(image_path).convert('RGB')
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
    
    # deepghs 模型只有 1 个类别
    # YOLOv5: [cx, cy, w, h, obj_conf, cls0] → 6 cols (奇数+1=偶数... 但只有1类是6)
    # YOLOv8: [cx, cy, w, h, cls0]           → 5 cols
    # 通用判断: 如果有 obj_conf 列 (v5), num_cols = 4 + 1 + nc; 否则 (v8), num_cols = 4 + nc
    
    is_v5_format = (num_cols % 2 == 1) if num_cols > 6 else (num_cols >= 6)
    
    if is_v5_format and num_cols >= 6:
        # YOLOv5 格式
        for det in pred:
            obj_conf = det[4]
            if obj_conf < conf_thresh:
                continue
            cls_scores = det[5:]
            cls_id = np.argmax(cls_scores)
            score = obj_conf * cls_scores[cls_id]
            if score < conf_thresh:
                continue
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
                boxes.append((x1, y1, x2, y2, float(score)))
    else:
        # YOLOv8 格式
        for det in pred:
            cls_scores = det[4:]
            score = float(np.max(cls_scores))
            if score < conf_thresh:
                continue
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

def detect_with_model(sess, image_path, conf_thresh=0.3):
    """使用模型检测图片，返回检测框列表"""
    input_info = sess.get_inputs()[0]
    input_name = input_info.name
    input_shape = input_info.shape
    input_size = input_shape[2] if len(input_shape) >= 3 else 640
    if isinstance(input_size, str) or input_size <= 0:
        input_size = 640
    
    arr, orig_w, orig_h, scale, pad_x, pad_y = preprocess_image(image_path, input_size)
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

def process_image(models, image_path, options, output_dir):
    """处理单张图片 — 每种裁切类型用独立模型检测"""
    from PIL import Image
    
    img = Image.open(image_path).convert('RGB')
    stem = Path(image_path).stem
    ext = Path(image_path).suffix or '.png'
    results = []
    
    # 读取原始 tag 文件
    keep_tags = options.get('keep_original_tags', False)
    tag_file = Path(image_path).with_suffix('.txt')
    orig_tags = ''
    if tag_file.exists():
        orig_tags = tag_file.read_text(encoding='utf-8').strip()
    
    def save_tag(suffix, extra_tag=''):
        tags = orig_tags if keep_tags else ''
        if extra_tag:
            tags = f'{extra_tag}, {tags}' if tags else extra_tag
        if tags:
            tag_out = os.path.join(output_dir, f'{stem}{suffix}.txt')
            Path(tag_out).write_text(tags, encoding='utf-8')
    
    # 全身检测
    if options.get('person_enabled', True) and 'person' in models:
        conf = options.get('person_conf', 0.3)
        boxes = detect_with_model(models['person'], image_path, conf)
        for idx, (x1, y1, x2, y2, c) in enumerate(boxes):
            suffix = f'_{idx}' if len(boxes) > 1 else ''
            cropped = crop_box(img, x1, y1, x2, y2, 0.08)
            out_name = f'{stem}{suffix}_full{ext}'
            cropped.save(os.path.join(output_dir, out_name))
            if keep_tags and orig_tags:
                save_tag(f'{suffix}_full')
            results.append(f'全身({c:.2f})')
    
    # 半身检测
    if options.get('upper_enabled', True) and 'halfbody' in models:
        conf = options.get('upper_conf', 0.5)
        boxes = detect_with_model(models['halfbody'], image_path, conf)
        upper_tag = options.get('upper_tag', 'upper body')
        for idx, (x1, y1, x2, y2, c) in enumerate(boxes):
            suffix = f'_{idx}' if len(boxes) > 1 else ''
            cropped = crop_box(img, x1, y1, x2, y2, 0.06)
            out_name = f'{stem}{suffix}_halfbody{ext}'
            cropped.save(os.path.join(output_dir, out_name))
            save_tag(f'{suffix}_halfbody', upper_tag)
            results.append(f'半身({c:.2f})')
    
    # 头部检测
    if options.get('head_enabled', True) and 'head' in models:
        conf = options.get('head_conf', 0.4)
        head_scale = options.get('head_scale', 1.5)
        boxes = detect_with_model(models['head'], image_path, conf)
        head_tag = options.get('head_tag', 'head view')
        for idx, (x1, y1, x2, y2, c) in enumerate(boxes):
            suffix = f'_{idx}' if len(boxes) > 1 else ''
            # 按 head_scale 放大检测框
            bw, bh = x2 - x1, y2 - y1
            cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
            nw, nh = bw * head_scale, bh * head_scale
            sx1, sy1 = cx - nw / 2, cy - nh / 2
            sx2, sy2 = cx + nw / 2, cy + nh / 2
            cropped = crop_box(img, sx1, sy1, sx2, sy2, 0.02)
            out_name = f'{stem}{suffix}_head{ext}'
            cropped.save(os.path.join(output_dir, out_name))
            save_tag(f'{suffix}_head', head_tag)
            results.append(f'头部({c:.2f})')
    
    # 眼部检测
    if options.get('eyes_enabled', True) and 'eyes' in models:
        conf = options.get('eyes_conf', 0.3)
        eyes_scale = options.get('eyes_scale', 2.4)
        boxes = detect_with_model(models['eyes'], image_path, conf)
        eyes_tag = options.get('eyes_tag', 'eyes view')
        for idx, (x1, y1, x2, y2, c) in enumerate(boxes):
            suffix = f'_{idx}' if len(boxes) > 1 else ''
            # 按 eyes_scale 放大检测框
            bw, bh = x2 - x1, y2 - y1
            cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
            nw, nh = bw * eyes_scale, bh * eyes_scale
            sx1, sy1 = cx - nw / 2, cy - nh / 2
            sx2, sy2 = cx + nw / 2, cy + nh / 2
            cropped = crop_box(img, sx1, sy1, sx2, sy2, 0.02)
            out_name = f'{stem}{suffix}_eyes{ext}'
            cropped.save(os.path.join(output_dir, out_name))
            save_tag(f'{suffix}_eyes', eyes_tag)
            results.append(f'眼部({c:.2f})')
    
    if not results:
        return {'status': 'skip', 'message': '未检测到目标'}
    
    return {'status': 'success', 'message': f'裁切: {", ".join(results)}'}

def main():
    """主循环: stdin 读取 JSON, stdout 输出结果"""
    # 读取初始化配置
    init_line = sys.stdin.readline().strip()
    if not init_line:
        print(json.dumps({"error": "未收到初始化配置"}), flush=True)
        return
    
    try:
        config = json.loads(init_line)
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"JSON 解析失败: {e}"}), flush=True)
        return
    
    # 加载多个模型
    model_paths = config.get('model_paths', {})
    use_gpu = config.get('use_gpu', False)
    
    if not model_paths:
        print(json.dumps({"error": "未指定模型路径"}), flush=True)
        return
    
    models = {}
    try:
        for crop_type, path in model_paths.items():
            sys.stderr.write(f"[person_crop] 加载 {crop_type} 模型: {os.path.basename(path)}\n")
            sys.stderr.flush()
            models[crop_type] = load_model(path, use_gpu)
    except Exception as e:
        print(json.dumps({"error": f"模型加载失败: {e}"}), flush=True)
        return
    
    loaded_types = list(models.keys())
    sys.stderr.write(f"[person_crop] 已加载 {len(models)} 个模型: {', '.join(loaded_types)}\n")
    sys.stderr.flush()
    
    print(json.dumps({
        "status": "ready",
        "models_loaded": loaded_types
    }), flush=True)
    
    # 处理循环
    for line in sys.stdin:
        line = line.strip()
        if not line or line == 'EXIT':
            break
        
        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            print(json.dumps({"status": "error", "message": "JSON 解析失败"}), flush=True)
            continue
        
        action = cmd.get('action')
        if action != 'process':
            print(json.dumps({"status": "error", "message": f"未知操作: {action}"}), flush=True)
            continue
        
        image_path = cmd.get('image_path', '')
        output_dir = cmd.get('output_dir', '')
        options = cmd.get('options', {})
        
        try:
            result = process_image(models, image_path, options, output_dir)
            print(json.dumps(result), flush=True)
        except Exception as e:
            sys.stderr.write(f"[person_crop] 处理失败 {image_path}: {traceback.format_exc()}\n")
            sys.stderr.flush()
            print(json.dumps({"status": "error", "message": str(e)}), flush=True)

if __name__ == '__main__':
    main()
