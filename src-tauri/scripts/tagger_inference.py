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

def _emit(data):
    """输出 JSON line 到 stdout (Windows GBK 安全)"""
    line = json.dumps(data, ensure_ascii=False) + "\n"
    sys.stdout.buffer.write(line.encode("utf-8"))
    sys.stdout.buffer.flush()

def log(msg):
    """输出日志到 stdout (JSON line)"""
    _emit({"type": "log", "message": msg})

def log_i18n(key, params=None):
    d = {"type": "log", "i18n_key": key, "message": key}
    if params:
        d["i18n_params"] = params
    _emit(d)

def error(msg):
    """输出错误到 stdout (JSON line)"""
    _emit({"type": "error", "message": msg})

def result(data):
    """输出结果到 stdout (JSON line)"""
    _emit(data)

def preprocess_image(image_path, target_size, input_format, preprocess_mode="auto"):
    """预处理图片"""
    from PIL import Image

    image = Image.open(image_path)

    # 处理透明通道
    if image.mode not in ["RGB", "RGBA"]:
        image = image.convert("RGBA") if "transparency" in image.info else image.convert("RGB")
    if image.mode == "RGBA":
        background = Image.new("RGB", image.size, (255, 255, 255))
        background.paste(image, mask=image.split()[3])
        image = background

    if preprocess_mode == "siglip2":
        image = image.resize((target_size, target_size), Image.BICUBIC)
        img_array = np.array(image, dtype=np.float32) / 255.0
        img_array = img_array.transpose(2, 0, 1)
        mean = np.array([0.5, 0.5, 0.5], dtype=np.float32).reshape(3, 1, 1)
        std = np.array([0.5, 0.5, 0.5], dtype=np.float32).reshape(3, 1, 1)
        img_array = (img_array - mean) / std
        return img_array[np.newaxis, ...].astype(np.float32)

    if input_format == "NCHW":
        # CL Tagger 预处理 (参考官方 HuggingFace Space)
        # 1. Pad to square (白色填充, 使用 PIL)
        w, h = image.size
        if w != h:
            new_size = max(w, h)
            new_image = Image.new("RGB", (new_size, new_size), (255, 255, 255))
            new_image.paste(image, ((new_size - w) // 2, (new_size - h) // 2))
            image = new_image
        # 2. Resize with BICUBIC
        image = image.resize((target_size, target_size), Image.BICUBIC)
        # 3. to numpy float32 / 255.0
        img_array = np.array(image, dtype=np.float32) / 255.0
        # 4. HWC -> CHW
        img_array = img_array.transpose(2, 0, 1)
        # 5. RGB -> BGR
        img_array = img_array[::-1, :, :]
        # 6. normalize: (x - 0.5) / 0.5
        mean = np.array([0.5, 0.5, 0.5], dtype=np.float32).reshape(3, 1, 1)
        std = np.array([0.5, 0.5, 0.5], dtype=np.float32).reshape(3, 1, 1)
        img_array = (img_array - mean) / std
        return img_array[np.newaxis, ...].astype(np.float32)  # [1, C, H, W]
    else:
        # WD Tagger 预处理 (NHWC, sd-scripts 风格)
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
        return image[np.newaxis, ...]  # [1, H, W, C]

def load_tags_csv(csv_path):
    """从 CSV 加载标签定义"""
    tags = []
    category_map = {9: "rating", 0: "general", 4: "character", 1: "artist", 3: "copyright", 5: "meta", 6: "quality", 7: "model"}
    with open(csv_path, "r", encoding="utf-8") as f:
        reader = csv.reader(f)
        header = next(reader)
        has_count = len(header) >= 4 and header[3].strip().lower() == "count"
        for row in reader:
            if len(row) >= 3:
                name = row[1]
                cat_id = int(row[2])
                category = category_map.get(cat_id, "general")
                count = int(row[3]) if has_count and len(row) >= 4 and row[3].strip().isdigit() else 0
                tags.append({"name": name, "category": category, "count": count})
    return tags

def load_tags_json(json_path):
    """从 JSON 加载标签定义 (CL Tagger 格式)"""
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    if isinstance(data, dict) and "idx_to_tag" in data:
        return load_vocabulary_json(data)

    tags = []
    for idx_str in sorted(data.keys(), key=int):
        info = data[idx_str]
        tag_name = info.get("tag", "")
        category = info.get("category", "General").lower()
        count = info.get("count", 0)
        tags.append({"name": tag_name, "category": category, "count": count})
    return tags

def _resolve_category_index(index, categories):
    if categories is None:
        return None
    if isinstance(categories, list) and 0 <= index < len(categories):
        return categories[index]
    if isinstance(categories, dict):
        return categories.get(str(index)) or categories.get(index)
    return None

def _normalize_category(raw, categories=None):
    if isinstance(raw, int):
        raw = _resolve_category_index(raw, categories)
    elif isinstance(raw, str) and raw.isdigit():
        raw = _resolve_category_index(int(raw), categories) or raw
    if raw is None:
        raw = "general"
    key = str(raw).strip().lower().replace("-", "_")
    if key == "copyrights":
        key = "copyright"
    elif key == "characters":
        key = "character"
    allowed = {"general", "artist", "copyright", "character", "meta", "rating", "quality", "model"}
    return key if key in allowed else "general"

def load_vocabulary_json(data):
    """加载 CL Tagger v2 model_vocabulary.json"""
    idx_to_tag = data.get("idx_to_tag", {})
    tag_to_category = data.get("tag_to_category", {})
    idx_to_category = data.get("idx_to_category", {})
    tag_to_count = data.get("tag_to_count", {})
    categories = data.get("categories")

    indexed_tags = []
    if isinstance(idx_to_tag, list):
        indexed_tags = list(enumerate(idx_to_tag))
    elif isinstance(idx_to_tag, dict):
        indexed_tags = []
        for idx_str, tag_name in idx_to_tag.items():
            try:
                idx = int(idx_str)
            except Exception:
                idx = len(indexed_tags)
            indexed_tags.append((idx, tag_name))
        indexed_tags.sort(key=lambda item: item[0])

    tags = []
    for idx, tag_name in indexed_tags:
        tag_name = str(tag_name)
        category_raw = tag_to_category.get(tag_name)
        if category_raw is None:
            category_raw = idx_to_category.get(str(idx)) if isinstance(idx_to_category, dict) else None
        count = tag_to_count.get(tag_name, 0) if isinstance(tag_to_count, dict) else 0
        tags.append({
            "name": tag_name,
            "category": _normalize_category(category_raw, categories),
            "count": int(count) if isinstance(count, (int, float)) else 0,
        })
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

# ── 关键词集合：用于将 general 标签分为 appearance / environment / tags ──
_APPEARANCE_KEYWORDS = {
    # 发型 / 发色
    "hair", "bangs", "ponytail", "twintails", "braid", "ahoge", "sidelocks",
    "bob cut", "short hair", "long hair", "medium hair", "very long hair",
    "twin braids", "side ponytail", "low ponytail", "high ponytail",
    "hair bun", "double bun", "single braid", "french braid",
    "hair ornament", "hairclip", "hairpin", "hairband", "hair ribbon",
    "hair flower", "hair bow", "hair tie",
    "blonde", "brunette", "redhead", "silver hair", "white hair", "black hair",
    "blue hair", "green hair", "pink hair", "purple hair", "red hair",
    "multicolored hair", "gradient hair", "streaked hair", "colored tips",
    # 眼睛
    "eyes", "eye", "heterochromia", "slit pupils", "eyelashes",
    # 身体特征
    "breasts", "flat chest", "large breasts", "medium breasts", "small breasts",
    "huge breasts", "tail", "horns", "wings", "ears", "fang", "fangs",
    "pointy ears", "animal ears", "cat ears", "dog ears", "fox ears",
    "rabbit ears", "cat tail", "fox tail", "demon tail",
    "halo", "antenna", "antennae",
    # 服饰
    "dress", "shirt", "skirt", "pants", "shorts", "uniform", "hat", "cap",
    "ribbon", "bow", "tie", "necktie", "bowtie",
    "boots", "shoes", "sandals", "sneakers", "high heels", "loafers",
    "gloves", "glasses", "sunglasses", "earrings", "necklace", "bracelet",
    "ring", "choker", "collar", "scarf", "hood",
    "stockings", "thighhighs", "pantyhose", "socks", "kneehighs",
    "jacket", "coat", "hoodie", "sweater", "vest", "armor", "cape", "cloak",
    "headband", "tiara", "crown", "mask", "veil", "goggles",
    "sleeve", "sleeves", "detached sleeves", "long sleeves", "short sleeves",
    "bikini", "swimsuit", "leotard", "bodysuit", "maid", "apron",
    "kimono", "yukata", "chinese clothes", "school uniform", "sailor collar",
    "serafuku", "blazer", "cardigan", "miniskirt", "pleated skirt",
    "frills", "lace", "zipper", "belt", "suspenders",
    "bare shoulders", "midriff", "navel", "cleavage",
    "off shoulder", "strapless", "backless", "sideboob",
    "clothing cutout", "cleavage cutout",
    "thigh strap", "garter", "garter straps", "garter belt",
    "frilled dress", "frilled skirt",
    # 肤色 / 体型
    "dark skin", "pale skin", "tan", "muscular", "slim", "petite",
}

_ENVIRONMENT_KEYWORDS = {
    "outdoors", "indoors", "sky", "cloud", "clouds", "water", "ocean", "sea",
    "lake", "river", "pool", "rain", "snow", "ice",
    "grass", "tree", "trees", "forest", "mountain", "hill", "field",
    "building", "city", "town", "street", "road", "alley", "bridge",
    "night", "night sky", "day", "sunset", "sunrise", "dawn", "dusk",
    "moonlight", "sunlight", "starry sky", "starry", "star", "stars",
    "moon", "sun", "rainbow",
    "flower", "flowers", "garden", "park", "bench",
    "room", "bedroom", "classroom", "kitchen", "bathroom", "hallway",
    "school", "beach", "shore", "sand",
    "window", "door", "stairs", "balcony", "rooftop", "ceiling", "floor",
    "wall", "fence", "railing", "pillar",
    "castle", "church", "temple", "shrine", "ruins", "cave",
    "train", "car", "bus", "boat", "ship", "airplane",
    "stage", "spotlight", "curtain", "carpet",
    "lamp", "lantern", "candle", "chandelier", "light",
    "cherry blossoms", "petals", "leaves", "autumn leaves",
    "snow", "snowflakes", "wind", "fog", "mist",
    "space", "planet", "galaxy", "nebula", "constellation",
    "underwater", "bubble", "bubbles",
}

# 人数标签
_COUNT_TAGS = {
    "1girl", "2girls", "3girls", "4girls", "5girls", "6+girls",
    "1boy", "2boys", "3boys", "4boys", "5boys", "6+boys",
    "1other", "multiple girls", "multiple boys",
    "solo", "duo", "trio", "group",
}

# 颜文字标签白名单 (kohya/wd14 通用列表)：这些标签的下划线是表情的一部分，不做替换
_KAOMOJI_TAGS = {
    "0_0", "(o)_(o)", "+_+", "+_-", "._.", "<o>_<o>", "<|>_<|>", "=_=",
    ">_<", "3_3", "6_9", ">_o", "@_@", "^_^", "o_o", "u_u", "x_x",
    "|_|", "||_||",
}


def _classify_general_tag(tag_name):
    """判断 general 标签属于 appearance / environment / tags"""
    lower = tag_name.lower()
    # 完整匹配
    if lower in _APPEARANCE_KEYWORDS:
        return "appearance"
    if lower in _ENVIRONMENT_KEYWORDS:
        return "environment"
    # 部分匹配（包含关键词）
    for kw in _APPEARANCE_KEYWORDS:
        if len(kw) >= 4 and kw in lower:
            return "appearance"
    for kw in _ENVIRONMENT_KEYWORDS:
        if len(kw) >= 4 and kw in lower:
            return "environment"
    return "tags"


def _build_structured_json(selected_tags):
    """
    将 (tag_name, category) 列表构建为 AnimaLoraStudio 完整格式 JSON。
    文档: fixed.quality / fixed.series / fixed.artist / character.name / ai_output.*
    """
    character_name = ""
    series_name = ""
    artist_name = ""
    count_tags = []
    appearance = []
    tags_list = []
    environment = []
    quality_parts = []  # rating + quality 合并

    for tag_name, cat, *_ in selected_tags:
        if cat == "character":
            character_name = tag_name if not character_name else f"{character_name}, {tag_name}"
        elif cat == "copyright":
            series_name = tag_name if not series_name else f"{series_name}, {tag_name}"
        elif cat == "rating":
            quality_parts.insert(0, tag_name)  # rating 放前面
        elif cat == "quality":
            quality_parts.append(tag_name)
        elif cat == "artist":
            artist_name = tag_name if not artist_name else f"{artist_name}, {tag_name}"
        elif cat == "model":
            tags_list.append(tag_name)
        else:
            lower = tag_name.lower()
            if lower in _COUNT_TAGS:
                count_tags.append(tag_name)
            else:
                sub = _classify_general_tag(tag_name)
                if sub == "appearance":
                    appearance.append(tag_name)
                elif sub == "environment":
                    environment.append(tag_name)
                else:
                    tags_list.append(tag_name)

    out = {}
    # fixed: quality, series, artist
    fixed = {}
    if quality_parts:
        fixed["quality"] = ", ".join(quality_parts)
    if series_name:
        fixed["series"] = series_name
    if artist_name:
        fixed["artist"] = f"@{artist_name}" if not artist_name.startswith("@") else artist_name
    if fixed:
        out["fixed"] = fixed
    # character
    if character_name:
        out["character"] = {"name": character_name}
    # ai_output
    ai = {}
    if count_tags:
        ai["count"] = ", ".join(count_tags)
    if appearance:
        ai["appearance"] = appearance
    if tags_list:
        ai["tags"] = tags_list
    if environment:
        ai["environment"] = environment
    if ai:
        out["ai_output"] = ai

    return out


def _build_simplified_json(selected_tags):
    """简化格式：所有字段扁平化，对齐 AnimaLoraStudio 简化格式"""
    out = {}
    characters = []
    series_list = []
    artist_name = ""
    count_tags = []
    appearance = []
    tags_list = []
    environment = []
    quality_parts = []  # rating + quality 合并

    for tag_name, cat, *_ in selected_tags:
        if cat == "character":
            characters.append(tag_name)
        elif cat == "copyright":
            series_list.append(tag_name)
        elif cat == "rating":
            quality_parts.insert(0, tag_name)
        elif cat == "quality":
            quality_parts.append(tag_name)
        elif cat == "artist":
            artist_name = tag_name if not artist_name else f"{artist_name}, {tag_name}"
        elif cat == "model":
            tags_list.append(tag_name)
        else:
            lower = tag_name.lower()
            if lower in _COUNT_TAGS:
                count_tags.append(tag_name)
            else:
                sub = _classify_general_tag(tag_name)
                if sub == "appearance":
                    appearance.append(tag_name)
                elif sub == "environment":
                    environment.append(tag_name)
                else:
                    tags_list.append(tag_name)

    if quality_parts:
        out["quality"] = ", ".join(quality_parts)
    if count_tags:
        out["count"] = ", ".join(count_tags)
    if characters:
        out["character"] = ", ".join(characters)
    if series_list:
        out["series"] = ", ".join(series_list)
    if artist_name:
        out["artist"] = f"@{artist_name}" if not artist_name.startswith("@") else artist_name
    if appearance:
        out["appearance"] = appearance
    if tags_list:
        out["tags"] = tags_list
    if environment:
        out["environment"] = environment

    return out


def _normalize_tag_key(tag):
    """词表查询键归一化：下划线/空格互换、大小写不敏感"""
    return tag.strip().lower().replace("_", " ")


def run_convert_mode():
    """--convert 一次性模式：把图片旁的 .txt 标签按模型词表分类后转换为 JSON。

    仅加载词表（CSV/JSON），不加载 ONNX/onnxruntime，速度很快。
    LLM 调优新增的、不在词表中的标签按 general 处理（再走外观/环境关键词细分）。
    """
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--convert", action="store_true")
    parser.add_argument("--input", required=True)
    parser.add_argument("--tags-path", required=True)
    parser.add_argument("--simplified", action="store_true")
    parser.add_argument("--remove-txt", action="store_true")
    parser.add_argument("--recursive", action="store_true")
    args = parser.parse_args()

    if args.tags_path.endswith(".json"):
        defs = load_tags_json(args.tags_path)
    else:
        defs = load_tags_csv(args.tags_path)
    cat_by_name = {}
    for d in defs:
        cat_by_name[_normalize_tag_key(d["name"])] = d["category"]

    exts = {".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tiff", ".tif", ".gif"}
    root = Path(args.input)
    if not root.is_dir():
        result({"type": "error", "message": f"输入目录不存在: {args.input}"})
        return
    if args.recursive:
        images = sorted(f for f in root.rglob("*") if f.is_file() and f.suffix.lower() in exts)
    else:
        images = sorted(f for f in root.iterdir() if f.is_file() and f.suffix.lower() in exts)

    total = len(images)
    converted = 0
    skipped = 0
    failed = 0
    for i, img in enumerate(images):
        txt = img.parent / f"{img.stem}.txt"
        if not txt.exists():
            skipped += 1
        else:
            try:
                raw = txt.read_text(encoding="utf-8")
                tag_list = [t.strip() for t in raw.replace("\n", ",").split(",") if t.strip()]
                selected = []
                for t in tag_list:
                    plain = t.replace("\\(", "(").replace("\\)", ")")
                    cat = cat_by_name.get(_normalize_tag_key(plain), "general")
                    selected.append((plain, cat))
                data = _build_simplified_json(selected) if args.simplified else _build_structured_json(selected)
                json_path = img.parent / f"{img.stem}.json"
                with open(json_path, "w", encoding="utf-8") as f:
                    json.dump(data, f, ensure_ascii=False, indent=2)
                if args.remove_txt:
                    txt.unlink()
                converted += 1
            except Exception as e:
                failed += 1
                result({"type": "log", "message": f"转换失败 {img.name}: {e}"})
        result({"type": "progress", "current": i + 1, "total": total, "filename": img.name})

    result({
        "type": "done",
        "converted": converted,
        "skipped": skipped,
        "failed": failed,
        "total": total,
    })


def main():
    # --convert：txt → JSON 转换模式（无需 ONNX，处理完直接退出）
    if "--convert" in sys.argv:
        run_convert_mode()
        return

    # Windows: 注册 CUDA DLL 目录（必须在 import onnxruntime 之前）
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from cuda_dll_helper import register_cuda_dlls
    register_cuda_dlls()

    import onnxruntime as ort

    session = None
    tags = []
    input_format = "NHWC"
    input_size = 448
    input_name = None
    preprocess_mode = "auto"

    # Windows 上 sys.stdin 默认用 GBK 编码，但 Rust 发送的是 UTF-8
    # 必须用 buffer 以二进制读取再手动 UTF-8 解码
    import io
    stdin_reader = io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8", errors="replace")

    for line in stdin_reader:
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
                preprocess_mode = cmd.get("preprocess_mode", "auto")

                # === ONNX Runtime 后端 ===
                # 统一流程：探测环境（显卡型号 / CUDA / cuDNN）+ 输出日志 + 决定 providers
                from gpu_diagnostics import resolve_ort_providers
                providers = resolve_ort_providers(log_i18n, use_gpu=use_gpu)
                gpu_provider = providers[0] if providers[0] != "CPUExecutionProvider" else None

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

                # 检测输入格式
                input_name = session.get_inputs()[0].name
                input_format, detected_size = detect_model_format(session)
                actual_providers = session.get_providers()
                actual_info = f"onnxruntime {ort.__version__}, providers: {actual_providers}"

                # input_size
                override_size = cmd.get("input_size", 0)
                if override_size and override_size > 0:
                    input_size = override_size
                else:
                    input_size = detected_size if detected_size > 0 else 448

                # 加载标签
                if tags_path.endswith(".json"):
                    tags = load_tags_json(tags_path)
                else:
                    tags = load_tags_csv(tags_path)

                log(f"✓ 模型已就绪 ({len(tags)} 标签, {input_size}x{input_size})")

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
                replace_underscore = cmd.get("replace_underscore", True)
                exclude_tags_str = cmd.get("exclude_tags", "")
                append_tags_str = cmd.get("append_tags", "")
                append_position = cmd.get("append_position", "append")
                escape_parentheses = cmd.get("escape_parentheses", False)
                sort_by = cmd.get("sort_by", "confidence")  # "confidence" or "frequency"

                # 解析排除标签集合
                exclude_set = set()
                if exclude_tags_str.strip():
                    for t in exclude_tags_str.split(","):
                        t = t.strip()
                        if t:
                            exclude_set.add(t)

                # 解析追加标签列表
                append_list = []
                if append_tags_str.strip():
                    for t in append_tags_str.split(","):
                        t = t.strip()
                        if t:
                            append_list.append(t)

                # 预处理
                img_data = preprocess_image(image_path, input_size, input_format, preprocess_mode)

                # 推理
                outputs = session.run(None, {input_name: img_data})
                probs = outputs[0][0]  # shape: [num_tags]

                # 对 NCHW 模型的输出需要 sigmoid (CL Tagger 输出 logits)
                if input_format == "NCHW":
                    probs = 1 / (1 + np.exp(-np.clip(probs, -30, 30)))

                # 筛选标签（带分类信息）
                # 严格对齐官方 CL Tagger 逻辑:
                #   - rating: argmax (取最高分1个)
                #   - quality: argmax (取最高分1个)
                #   - general/meta: gen_threshold 阈值过滤
                #   - character/copyright/artist: char_threshold 阈值过滤
                #   - model: gen_threshold 阈值过滤
                selected_tags = []      # (tag_name, category, prob, count) 用于分类
                selected_flat = []      # 纯名称列表，用于 txt 输出

                # 按类别收集所有标签的 (index, prob)
                category_indices = {}  # cat -> [(idx, prob)]
                for idx, prob in enumerate(probs):
                    if idx >= len(tags):
                        break
                    tag = tags[idx]
                    cat = tag["category"]
                    if cat not in enabled_categories:
                        continue
                    if cat not in category_indices:
                        category_indices[cat] = []
                    category_indices[cat].append((idx, float(prob)))

                # argmax 类别: rating, quality
                for argmax_cat in ["rating", "quality"]:
                    if argmax_cat not in category_indices:
                        continue
                    pairs = category_indices[argmax_cat]
                    if not pairs:
                        continue
                    best_idx, best_prob = max(pairs, key=lambda x: x[1])
                    tag_name = tags[best_idx]["name"]
                    tag_count = tags[best_idx].get("count", 0)
                    if replace_underscore and tag_name not in _KAOMOJI_TAGS:
                        tag_name = tag_name.replace("_", " ")
                    if escape_parentheses:
                        tag_name = tag_name.replace("(", "\\(").replace(")", "\\)")
                    if tag_name in exclude_set or tags[best_idx]["name"] in exclude_set:
                        continue
                    selected_tags.append((tag_name, argmax_cat, best_prob, tag_count))
                    selected_flat.append(tag_name)

                # 阈值类别
                threshold_cats = {
                    "general": general_threshold,
                    "character": character_threshold,
                    "copyright": character_threshold,
                    "artist": character_threshold,
                    "meta": general_threshold,
                    "model": general_threshold,
                }
                for cat, thresh in threshold_cats.items():
                    if cat not in category_indices:
                        continue
                    pairs = category_indices[cat]
                    # 按概率降序排列
                    pairs_sorted = sorted(pairs, key=lambda x: x[1], reverse=True)
                    for idx, prob in pairs_sorted:
                        if prob < thresh:
                            continue
                        tag_name = tags[idx]["name"]
                        tag_count = tags[idx].get("count", 0)
                        if replace_underscore and tag_name not in _KAOMOJI_TAGS:
                            tag_name = tag_name.replace("_", " ")
                        if escape_parentheses:
                            tag_name = tag_name.replace("(", "\\(").replace(")", "\\)")
                        if tag_name in exclude_set or tags[idx]["name"] in exclude_set:
                            continue
                        selected_tags.append((tag_name, cat, prob, tag_count))
                        selected_flat.append(tag_name)

                # 按频率排序（如果启用）
                if sort_by == "frequency":
                    # 将 selected_tags 和 selected_flat 按 count 降序重新排序
                    # 保持 (tag_name, cat, prob, count) 的对应关系
                    indexed = list(enumerate(selected_tags))
                    indexed.sort(key=lambda x: x[1][3], reverse=True)
                    selected_tags = [item[1] for item in indexed]
                    selected_flat = [item[1][0] for item in indexed]

                # 输出格式
                output_format = cmd.get("output_format", "txt")
                existing_tags_action = cmd.get("existing_tags_action", "overwrite")
                stem = Path(image_path).stem
                parent = Path(image_path).parent

                if output_format == "json":
                    json_simplified = cmd.get("json_simplified", False)
                    json_path = parent / f"{stem}.json"

                    # 已标识文件操作
                    if existing_tags_action == "skip" and json_path.exists():
                        result({
                            "type": "result",
                            "image_path": image_path,
                            "tags": [],
                            "tag_count": 0,
                            "skipped": True,
                        })
                        continue

                    if existing_tags_action in ("prepend", "append") and json_path.exists():
                        # JSON 格式合并：读取已有文件，与新标签合并
                        try:
                            with open(json_path, "r", encoding="utf-8") as f:
                                existing_data = json.load(f)
                            # 简化处理：JSON 模式下直接覆盖（JSON 结构合并太复杂）
                            # 保留已有数据并用新数据更新
                            if existing_tags_action == "append":
                                # 已有数据优先，新数据补充
                                merged = existing_data.copy()
                                if json_simplified:
                                    new_data = _build_simplified_json(selected_tags)
                                else:
                                    new_data = _build_structured_json(selected_tags)
                                for k, v in new_data.items():
                                    if k not in merged:
                                        merged[k] = v
                                    elif isinstance(v, dict) and isinstance(merged[k], dict):
                                        for kk, vv in v.items():
                                            if kk not in merged[k]:
                                                merged[k][kk] = vv
                                            elif isinstance(vv, list) and isinstance(merged[k][kk], list):
                                                existing_set = set(merged[k][kk])
                                                merged[k][kk] = merged[k][kk] + [t for t in vv if t not in existing_set]
                                    elif isinstance(v, list) and isinstance(merged[k], list):
                                        existing_set = set(merged[k])
                                        merged[k] = merged[k] + [t for t in v if t not in existing_set]
                            else:
                                # prepend: 已有数据保持不变，新数据中不重复的部分补充进去
                                merged = existing_data.copy()
                                if json_simplified:
                                    new_data = _build_simplified_json(selected_tags)
                                else:
                                    new_data = _build_structured_json(selected_tags)
                                for k, v in new_data.items():
                                    if k not in merged:
                                        merged[k] = v
                                    elif isinstance(v, dict) and isinstance(merged[k], dict):
                                        for kk, vv in v.items():
                                            if kk not in merged[k]:
                                                merged[k][kk] = vv
                                            elif isinstance(vv, list) and isinstance(merged[k][kk], list):
                                                existing_set = set(merged[k][kk])
                                                merged[k][kk] = merged[k][kk] + [t for t in vv if t not in existing_set]
                                    elif isinstance(v, list) and isinstance(merged[k], list):
                                        existing_set = set(merged[k])
                                        merged[k] = merged[k] + [t for t in v if t not in existing_set]
                            with open(json_path, "w", encoding="utf-8") as f:
                                json.dump(merged, f, ensure_ascii=False, indent=2)
                        except Exception as merge_err:
                            # 合并失败时不覆盖用户原文件：仅警告并跳过写入
                            log(f"⚠ JSON 合并失败，跳过写入以保护原文件 [{json_path.name}]: {merge_err}")
                    else:
                        # overwrite 或文件不存在
                        if json_simplified:
                            data = _build_simplified_json(selected_tags)
                        else:
                            data = _build_structured_json(selected_tags)
                        with open(json_path, "w", encoding="utf-8") as f:
                            json.dump(data, f, ensure_ascii=False, indent=2)
                else:
                    txt_path = parent / f"{stem}.txt"

                    # 已标识文件操作
                    if existing_tags_action == "skip" and txt_path.exists():
                        result({
                            "type": "result",
                            "image_path": image_path,
                            "tags": [],
                            "tag_count": 0,
                            "skipped": True,
                        })
                        continue

                    if existing_tags_action in ("prepend", "append") and txt_path.exists():
                        try:
                            with open(txt_path, "r", encoding="utf-8") as f:
                                existing_text = f.read().strip()
                            existing_list = [t.strip() for t in existing_text.split(",") if t.strip()]
                            # 去重合并
                            if existing_tags_action == "append":
                                # 已有标签在前，新标签补充到后面（去掉模型输出中的重复）
                                existing_set = set(existing_list)
                                merged = existing_list + [t for t in selected_flat if t not in existing_set]
                            else:
                                # 新标签在前，已有标签保持原位（去掉模型输出中的重复）
                                existing_set = set(existing_list)
                                merged = [t for t in selected_flat if t not in existing_set] + existing_list
                            selected_flat = merged
                        except Exception:
                            pass  # 读取失败，使用新标签覆盖

                    # 追加标签最后处理（优先级最高，确保触发词始终在指定位置）
                    if append_list:
                        append_set = set(append_list)
                        selected_flat = [n for n in selected_flat if n not in append_set]
                        if append_position == "prepend":
                            selected_flat = append_list + selected_flat
                        else:
                            selected_flat = selected_flat + append_list

                    with open(txt_path, "w", encoding="utf-8") as f:
                        f.write(", ".join(selected_flat))

                result({
                    "type": "result",
                    "image_path": image_path,
                    "tags": selected_flat,
                    "tag_count": len(selected_flat),
                })

            elif cmd["cmd"] == "tag_batch":
                if session is None:
                    error("模型未初始化，请先发送 init 命令")
                    continue

                images = cmd.get("images", [])
                if not images:
                    error("tag_batch: images 为空")
                    continue

                # 批量预处理
                batch_data = []
                valid_indices = []  # 预处理成功的索引
                for idx, img_cmd in enumerate(images):
                    img_path = img_cmd.get("image_path", "")
                    try:
                        img_data = preprocess_image(img_path, input_size, input_format, preprocess_mode)
                        batch_data.append(img_data)
                        valid_indices.append(idx)
                    except Exception as e:
                        result({
                            "type": "error",
                            "image_path": img_path,
                            "message": f"预处理失败: {e}",
                        })

                if not batch_data:
                    continue

                # 拼接 batch tensor: [N, C, H, W] or [N, H, W, C]
                batch_tensor = np.concatenate(batch_data, axis=0)

                # 批量推理 (失败时降级为逐张推理重试)
                try:
                    outputs = session.run(None, {input_name: batch_tensor})
                    all_probs = outputs[0]  # shape: [N, num_tags]
                except Exception as e:
                    log(f"批量推理失败，降级为逐张推理重试: {type(e).__name__}")
                    all_probs = []
                    for bi, vi in enumerate(valid_indices):
                        img_path = images[vi].get("image_path", "")
                        try:
                            out_single = session.run(None, {input_name: batch_data[bi]})
                            all_probs.append(out_single[0][0])
                        except Exception as e2:
                            all_probs.append(None)
                            result({
                                "type": "error",
                                "image_path": img_path,
                                "message": f"推理失败: {e2}",
                            })

                # 逐张处理结果
                for batch_idx, orig_idx in enumerate(valid_indices):
                    img_cmd = images[orig_idx]
                    image_path = img_cmd.get("image_path", "")
                    try:
                        probs = all_probs[batch_idx]
                        if probs is None:
                            continue  # 逐张重试已失败并报过 error

                        general_threshold = img_cmd.get("general_threshold", 0.35)
                        character_threshold = img_cmd.get("character_threshold", 0.85)
                        enabled_categories = set(img_cmd.get("enabled_categories", ["general", "character"]))
                        replace_underscore = img_cmd.get("replace_underscore", True)
                        exclude_tags_str = img_cmd.get("exclude_tags", "")
                        append_tags_str = img_cmd.get("append_tags", "")
                        append_position = img_cmd.get("append_position", "append")
                        escape_parentheses = img_cmd.get("escape_parentheses", False)
                        sort_by = img_cmd.get("sort_by", "confidence")

                        exclude_set = set()
                        if exclude_tags_str.strip():
                            for t_str in exclude_tags_str.split(","):
                                t_str = t_str.strip()
                                if t_str:
                                    exclude_set.add(t_str)

                        append_list = []
                        if append_tags_str.strip():
                            for t_str in append_tags_str.split(","):
                                t_str = t_str.strip()
                                if t_str:
                                    append_list.append(t_str)

                        if input_format == "NCHW":
                            probs = 1 / (1 + np.exp(-np.clip(probs, -30, 30)))

                        selected_tags = []
                        selected_flat = []

                        category_indices = {}
                        for idx, prob in enumerate(probs):
                            if idx >= len(tags):
                                break
                            tag = tags[idx]
                            cat = tag["category"]
                            if cat not in enabled_categories:
                                continue
                            if cat not in category_indices:
                                category_indices[cat] = []
                            category_indices[cat].append((idx, float(prob)))

                        for argmax_cat in ["rating", "quality"]:
                            if argmax_cat not in category_indices:
                                continue
                            pairs = category_indices[argmax_cat]
                            if not pairs:
                                continue
                            best_idx, best_prob = max(pairs, key=lambda x: x[1])
                            tag_name = tags[best_idx]["name"]
                            tag_count = tags[best_idx].get("count", 0)
                            if replace_underscore and tag_name not in _KAOMOJI_TAGS:
                                tag_name = tag_name.replace("_", " ")
                            if escape_parentheses:
                                tag_name = tag_name.replace("(", "\\(").replace(")", "\\)")
                            if tag_name in exclude_set or tags[best_idx]["name"] in exclude_set:
                                continue
                            selected_tags.append((tag_name, argmax_cat, best_prob, tag_count))
                            selected_flat.append(tag_name)

                        threshold_cats = {
                            "general": general_threshold,
                            "character": character_threshold,
                            "copyright": character_threshold,
                            "artist": character_threshold,
                            "meta": general_threshold,
                            "model": general_threshold,
                        }
                        for cat, thresh in threshold_cats.items():
                            if cat not in category_indices:
                                continue
                            pairs = category_indices[cat]
                            pairs_sorted = sorted(pairs, key=lambda x: x[1], reverse=True)
                            for idx, prob in pairs_sorted:
                                if prob < thresh:
                                    continue
                                tag_name = tags[idx]["name"]
                                tag_count = tags[idx].get("count", 0)
                                if replace_underscore and tag_name not in _KAOMOJI_TAGS:
                                    tag_name = tag_name.replace("_", " ")
                                if escape_parentheses:
                                    tag_name = tag_name.replace("(", "\\(").replace(")", "\\)")
                                if tag_name in exclude_set or tags[idx]["name"] in exclude_set:
                                    continue
                                selected_tags.append((tag_name, cat, prob, tag_count))
                                selected_flat.append(tag_name)

                        if sort_by == "frequency":
                            indexed = list(enumerate(selected_tags))
                            indexed.sort(key=lambda x: x[1][3], reverse=True)
                            selected_tags = [item[1] for item in indexed]
                            selected_flat = [item[1][0] for item in indexed]

                        output_format = img_cmd.get("output_format", "txt")
                        existing_tags_action = img_cmd.get("existing_tags_action", "overwrite")
                        stem = Path(image_path).stem
                        parent = Path(image_path).parent

                        if output_format == "json":
                            json_simplified = img_cmd.get("json_simplified", False)
                            json_path = parent / f"{stem}.json"

                            if existing_tags_action == "skip" and json_path.exists():
                                result({"type": "result", "image_path": image_path, "tags": [], "tag_count": 0, "skipped": True})
                                continue

                            if existing_tags_action in ("prepend", "append") and json_path.exists():
                                try:
                                    with open(json_path, "r", encoding="utf-8") as f:
                                        existing_data = json.load(f)
                                    new_data = _build_simplified_json(selected_tags) if json_simplified else _build_structured_json(selected_tags)
                                    merged = existing_data.copy()
                                    for k, v in new_data.items():
                                        if k not in merged:
                                            merged[k] = v
                                        elif isinstance(v, dict) and isinstance(merged[k], dict):
                                            for kk, vv in v.items():
                                                if kk not in merged[k]:
                                                    merged[k][kk] = vv
                                                elif isinstance(vv, list) and isinstance(merged[k][kk], list):
                                                    existing_set = set(merged[k][kk])
                                                    merged[k][kk] = merged[k][kk] + [t_val for t_val in vv if t_val not in existing_set]
                                        elif isinstance(v, list) and isinstance(merged[k], list):
                                            existing_set = set(merged[k])
                                            merged[k] = merged[k] + [t_val for t_val in v if t_val not in existing_set]
                                    with open(json_path, "w", encoding="utf-8") as f:
                                        json.dump(merged, f, ensure_ascii=False, indent=2)
                                except Exception as merge_err:
                                    # 合并失败时不覆盖用户原文件：仅警告并跳过写入
                                    log(f"⚠ JSON 合并失败，跳过写入以保护原文件 [{json_path.name}]: {merge_err}")
                            else:
                                data = _build_simplified_json(selected_tags) if json_simplified else _build_structured_json(selected_tags)
                                with open(json_path, "w", encoding="utf-8") as f:
                                    json.dump(data, f, ensure_ascii=False, indent=2)
                        else:
                            txt_path = parent / f"{stem}.txt"

                            if existing_tags_action == "skip" and txt_path.exists():
                                result({"type": "result", "image_path": image_path, "tags": [], "tag_count": 0, "skipped": True})
                                continue

                            if existing_tags_action in ("prepend", "append") and txt_path.exists():
                                try:
                                    with open(txt_path, "r", encoding="utf-8") as f:
                                        existing_text = f.read().strip()
                                    existing_list = [t_str.strip() for t_str in existing_text.split(",") if t_str.strip()]
                                    if existing_tags_action == "append":
                                        existing_set = set(existing_list)
                                        merged = existing_list + [t_str for t_str in selected_flat if t_str not in existing_set]
                                    else:
                                        existing_set = set(existing_list)
                                        merged = [t_str for t_str in selected_flat if t_str not in existing_set] + existing_list
                                    selected_flat = merged
                                except Exception:
                                    pass

                            if append_list:
                                append_set = set(append_list)
                                selected_flat = [n for n in selected_flat if n not in append_set]
                                if append_position == "prepend":
                                    selected_flat = append_list + selected_flat
                                else:
                                    selected_flat = selected_flat + append_list

                            with open(txt_path, "w", encoding="utf-8") as f:
                                f.write(", ".join(selected_flat))

                        result({
                            "type": "result",
                            "image_path": image_path,
                            "tags": selected_flat,
                            "tag_count": len(selected_flat),
                        })
                    except Exception as e:
                        result({
                            "type": "error",
                            "image_path": image_path,
                            "message": f"{traceback.format_exc()}",
                        })

            elif cmd["cmd"] == "quit":
                log("推理进程退出")
                break

            else:
                error(f"未知命令: {cmd['cmd']}")

        except Exception as e:
            err_payload = {"type": "error", "message": f"{traceback.format_exc()}"}
            # 单图模式带上当前正在处理的图片路径（拿不到则省略）
            img_p = cmd.get("image_path", "") if isinstance(cmd, dict) else ""
            if img_p:
                err_payload["image_path"] = img_p
            result(err_payload)

if __name__ == "__main__":
    main()
