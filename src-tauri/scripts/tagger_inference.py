#!/usr/bin/env python3
"""
AI Tagger 推理脚本 - 由 Tauri 后端调用
使用 onnxruntime Python 包进行 ONNX 模型推理

常驻模式通信协议: JSON lines (stdin/stdout)
- 输入: {"cmd": "init", "model_path": "...", "tags_path": "...", "use_gpu": false,
         "input_size": 448, "preprocess_mode": "auto", "output_kind": "auto",
         "category_thresholds": {...}, "conservative_cuda": false}
  打标选项：general_threshold / character_threshold / enabled_categories / exclude_tags /
  append_tags / append_position / json_append_field / replace_underscore / output_format /
  json_simplified / escape_parentheses / sort_by / existing_tags_action
  已有标签的 skip 判断由 Rust 在发送前完成
- 输入: {"cmd": "tag_batch", "images": [{"image_path": "...", <打标选项>}, ...]}
  每张图回一条 result 或 error（带 image_path）
- 输入: {"cmd": "quit"}
- 输出: {"type": "ready"}
- 输出: {"type": "result", "image_path": "...", "tag_count": 10}，跳过写入时另带 "skipped": true
- 输出: {"type": "error", "message": "..."}，能确定图片时另带 "image_path"
- 输出: {"type": "log", "message": "..."}，可带 "i18n_key" / "i18n_params"

一次性模式（处理完即退出）:
- --detect <model_path>: 输出一行 {"type": "model_info", "input_size", "input_format", "input_shape"}，
  失败时写 stderr 并以退出码 1 结束
- --convert --input <目录或图片> --tags-path <词表> [--simplified] [--recursive]:
  txt → JSON，输出 progress / log 行，最后一行 {"type": "done", "converted", "skipped", "failed", "total"}；
  参数错误时输出 {"type": "error", "message": "..."}
"""

import sys
import json
import csv
import traceback
import numpy as np
from pathlib import Path

from purin_proto import (bootstrap, done, emit, error, log, log_i18n, progress, read_text_compat,
                         result, utf8_stdin, write_text_atomic)

def _pad_square(image, fill):
    from PIL import Image
    w, h = image.size
    side = max(w, h)
    if w == h:
        return image
    canvas = Image.new("RGB", (side, side), fill)
    canvas.paste(image, ((side - w) // 2, (side - h) // 2))
    return canvas


def _to_nchw(image, bgr=False):
    data = np.asarray(image, dtype=np.float32) / 255.0
    if bgr:
        data = data[:, :, ::-1]
    return ((data - 0.5) / 0.5).transpose(2, 0, 1)[np.newaxis, ...]


def preprocess_image(image_path, target_size, input_format, preprocess_mode="auto"):
    from PIL import Image

    with Image.open(image_path) as source:
        image = source.copy()
    if image.mode not in ["RGB", "RGBA"]:
        image = image.convert("RGBA") if "transparency" in image.info else image.convert("RGB")
    if image.mode == "RGBA":
        background = Image.new("RGB", image.size, (255, 255, 255))
        background.paste(image, mask=image.split()[3])
        image = background

    if preprocess_mode == "pixai_v1":
        # v1 使用等比 bilinear 缩放、居中黑填充和 RGB [-1, 1] 归一化。
        w, h = image.size
        if (w, h) != (target_size, target_size):
            scale = min(target_size / h, target_size / w)
            size = (max(1, int(w * scale)), max(1, int(h * scale)))
            image = image.resize(size, Image.Resampling.BILINEAR)
            canvas = Image.new("RGB", (target_size, target_size), (0, 0, 0))
            canvas.paste(image, ((target_size - size[0]) // 2, (target_size - size[1]) // 2))
            image = canvas
        return _to_nchw(image)

    if preprocess_mode in ("siglip2", "pixai"):
        resample = Image.Resampling.BICUBIC if preprocess_mode == "siglip2" else Image.Resampling.BILINEAR
        return _to_nchw(image.resize((target_size, target_size), resample))

    image = _pad_square(image, (255, 255, 255))
    if preprocess_mode == "wd_nchw" or input_format == "NCHW":
        image = image.resize((target_size, target_size), Image.Resampling.BICUBIC)
        # CL 使用 BGR；WD 的 timm 导出使用 RGB。
        return _to_nchw(image, bgr=preprocess_mode != "wd_nchw")
    image = image.resize((target_size, target_size), Image.Resampling.LANCZOS)
    return np.asarray(image, dtype=np.float32)[np.newaxis, :, :, ::-1]


def load_tags_csv(csv_path):
    """从 CSV 加载标签定义。

    按表头名取列而不是固定位置：SmilingWolf 系是 tag_id,name,category,count，
    PixAI(deepghs 导出)是 id,tag_id,name,category,count,ips——列位置不同。
    """
    tags = []
    category_map = {9: "rating", 0: "general", 4: "character", 1: "artist", 3: "copyright", 5: "meta", 6: "quality", 7: "model"}
    with open(csv_path, "r", encoding="utf-8") as f:
        reader = csv.reader(f)
        header = [h.strip().lower() for h in next(reader)]
        try:
            name_idx = header.index("name")
            cat_idx = header.index("category")
        except ValueError:
            # 无表头时使用旧格式的 tag_id,name,category[,count] 列顺序。
            name_idx, cat_idx = 1, 2
        count_idx = header.index("count") if "count" in header else None
        for row in reader:
            if len(row) > max(name_idx, cat_idx):
                name = row[name_idx]
                cat_id = int(row[cat_idx])
                category = category_map.get(cat_id, "general")
                count = 0
                if count_idx is not None and len(row) > count_idx and row[count_idx].strip().isdigit():
                    count = int(row[count_idx])
                tags.append({"name": name, "category": category, "count": count})
    return tags

def load_tags_json(json_path):
    """从 JSON 加载标签定义 (CL Tagger 格式)"""
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    if isinstance(data, dict) and "idx_to_tag" in data:
        return load_vocabulary_json(data)
    if isinstance(data, dict) and isinstance(data.get("categories"), list):
        return load_grouped_tags_json(data)

    tags = []
    for idx_str in sorted(data.keys(), key=int):
        info = data[idx_str]
        tag_name = info.get("tag", "")
        category = info.get("category", "General").lower()
        count = info.get("count", 0)
        tags.append({"name": tag_name, "category": category, "count": count})
    return tags

def load_grouped_tags_json(data):
    """按全局 offset 读取 PixAI v1 的分类词表，避免分类顺序改变标签索引。"""
    tags = []
    for group in sorted(data["categories"], key=lambda group: group["offset"]):
        names = group["tags"]
        if group["offset"] != len(tags) or group["count"] != len(names):
            raise ValueError("JSON 标签文件的分类索引或数量无效")
        category = _normalize_category(group["name"])
        for name in names:
            if not isinstance(name, str):
                raise ValueError("JSON 标签名称必须是字符串")
            tags.append({"name": name, "category": category, "count": 0})
    if len(tags) != data["num_classes"]:
        raise ValueError("JSON 标签总数与 num_classes 不一致")
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
    allowed = {"general", "artist", "style", "copyright", "character", "meta", "rating", "quality", "model"}
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

def load_tags(tags_path):
    """按扩展名选择词表加载函数：.json 走 JSON 词表，其余按 CSV。"""
    if tags_path.endswith(".json"):
        return load_tags_json(tags_path)
    return load_tags_csv(tags_path)


def select_tags(probs, tags, options, category_thresholds):
    """合并模型分类阈值与用户设置，供单张和批量推理共用。"""
    if len(probs) != len(tags):
        raise ValueError(f"模型输出数量 {len(probs)} 与词表数量 {len(tags)} 不一致")
    general = options.get("general_threshold", category_thresholds.get("general", 0.35))
    character = options.get("character_threshold", category_thresholds.get("character", 0.85))
    thresholds = {
        "general": general, "character": character, "copyright": character,
        "artist": character, "style": general, "meta": general, "model": general,
        **category_thresholds,
    }
    thresholds.update(general=general, character=character)
    enabled = set(options.get("enabled_categories", ["general", "character"]))
    excluded = {s.strip() for s in options.get("exclude_tags", "").split(",") if s.strip()}
    grouped = {}
    for tag, prob in zip(tags, probs):
        if tag["category"] in enabled:
            grouped.setdefault(tag["category"], []).append((tag, float(prob)))

    selected = []
    # 旧模型的 rating/quality 取最高分；提供分类阈值的模型按阈值筛选。
    for category in ("rating", "quality", "general", "character", "copyright", "artist", "style", "meta", "model"):
        pairs = grouped.get(category, [])
        if not pairs:
            continue
        if category in ("rating", "quality") and category not in category_thresholds:
            pairs = [max(pairs, key=lambda item: item[1])]
        else:
            threshold = thresholds[category]
            pairs = [(tag, prob) for tag, prob in pairs
                     if (prob > threshold if category in category_thresholds else prob >= threshold)]
            pairs.sort(key=lambda item: item[1], reverse=True)
        for tag, prob in pairs:
            name = tag["name"]
            if options.get("replace_underscore", True) and name not in _KAOMOJI_TAGS:
                name = name.replace("_", " ")
            if options.get("escape_parentheses", False):
                name = name.replace("(", "\\(").replace(")", "\\)")
            if name in excluded or name.replace("\\", "") in excluded or tag["name"] in excluded:
                continue
            selected.append((name, category, prob, tag.get("count", 0)))
    if options.get("sort_by", "confidence") == "frequency":
        selected.sort(key=lambda item: item[3], reverse=True)
    return selected, [tag[0] for tag in selected]


def _input_layout(shape):
    dims = [d if isinstance(d, int) and d > 0 else -1 for d in shape]
    if len(dims) == 4:
        if dims[3] in (1, 3, 4):
            return "NHWC", next((d for d in dims[1:3] if d > 0), 448)
        if dims[1] in (1, 3, 4):
            return "NCHW", next((d for d in dims[2:4] if d > 0), 448)
    return "NHWC", 448


def detect_model_format(session):
    return _input_layout(session.get_inputs()[0].shape)

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
    # "background" 靠下面的部分匹配覆盖 simple/white/blurry/gradient background 等全部变体。
    "background", "scenery", "landscape", "horizon",
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


def _format_artist(artist_name):
    return artist_name if not artist_name or artist_name.startswith("@") else f"@{artist_name}"


def _bucket_tags(selected_tags):
    buckets = {key: [] for key in _JSON_APPEND_FIELD_MAP}
    for name, category, *_ in selected_tags:
        if category == "rating":
            buckets["quality"].insert(0, name)
            continue
        if category == "artist":
            name = _format_artist(name)
        field = {"character": "character", "copyright": "series",
                 "quality": "quality", "artist": "artist",
                 "model": "tags", "style": "tags"}.get(category)
        if field is None:
            field = "count" if name.lower() in _COUNT_TAGS else _classify_general_tag(name)
        buckets[field].append(name)
    return {
        field: ", ".join(values) if field in ("quality", "series", "artist", "character", "count") else values
        for field, values in buckets.items()
    }


def _build_structured_json(selected_tags):
    # 空值和键顺序是 JSON 标签编辑器的完整格式契约。
    data = {"fixed": {}, "character": {}, "from_path": {"appearance": []}, "ai_output": {}}
    for field, value in _bucket_tags(selected_tags).items():
        container, key = _JSON_APPEND_FIELD_MAP[field]
        data[container][key] = value
    data["character"]["variant"] = ""
    data["ai_output"]["nl"] = ""
    return data


def _build_simplified_json(selected_tags):
    return {**_bucket_tags(selected_tags), "nl": ""}


def _normalize_tag_key(tag):
    """词表查询键归一化：下划线/空格互换、大小写不敏感"""
    return tag.strip().lower().replace("_", " ")


def _split_tags(text):
    """逗号分隔的标签串 → 去掉首尾空白、丢弃空项后的列表"""
    return [t.strip() for t in text.split(",") if t.strip()]


def _place(items, additions, position):
    """把 additions 放到 items 前面（prepend）或后面，items 中与之重复的项先去掉"""
    drop = set(additions)
    kept = [t for t in items if t not in drop]
    return additions + kept if position == "prepend" else kept + additions


def _read_tag_text(path):
    """读 txt 标签；UTF-8 和 GBK 都解不开时抛错，调用方据此放弃这个文件"""
    text = read_text_compat(path)
    if text is None:
        raise ValueError("无法按 UTF-8 或 GBK 解码")
    return text


def _sigmoid(x):
    return 1 / (1 + np.exp(-np.clip(x, -30, 30)))


def _fill_json(existing, new):
    """JSON 合并（existing_tags_action 为 append 或 prepend 时共用，两者效果相同）：
    保留已有字段，仅补充缺失字段，并对列表字段（含下一层）合并去重。"""
    merged = existing.copy()
    for k, v in new.items():
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
    return merged


def run_convert_mode():
    """--convert 一次性模式：txt → JSON 标签格式转换。

    按模型词表分类，仅加载词表（CSV/JSON），不加载 ONNX/onnxruntime，速度很快。
    LLM 调优新增的、不在词表中的标签按 general 处理（再走外观/环境关键词细分）。
    """
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--convert", action="store_true")
    parser.add_argument("--input", required=True)
    parser.add_argument("--tags-path", default=None)
    parser.add_argument("--simplified", action="store_true")
    parser.add_argument("--recursive", action="store_true")
    args = parser.parse_args()

    if not args.tags_path:
        error("txt → JSON 转换需要 --tags-path 指定模型词表")
        return
    cat_by_name = {}
    for d in load_tags(args.tags_path):
        cat_by_name[_normalize_tag_key(d["name"])] = d["category"]

    exts = {".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tiff", ".tif", ".gif"}
    root = Path(args.input)
    # 输入允许是单张图片：各页的输入选择器都支持"文件夹 / 单张图片"两种
    if root.is_file():
        images = [root] if root.suffix.lower() in exts else []
    elif not root.is_dir():
        error(f"输入路径不存在: {args.input}")
        return
    elif args.recursive:
        images = sorted(f for f in root.rglob("*") if f.is_file() and f.suffix.lower() in exts)
    else:
        images = sorted(f for f in root.iterdir() if f.is_file() and f.suffix.lower() in exts)

    total = len(images)
    converted = 0
    skipped = 0
    failed = 0
    for i, img in enumerate(images):
        txt = img.parent / f"{img.stem}.txt"
        json_path = img.parent / f"{img.stem}.json"
        if not txt.exists():
            skipped += 1
        elif json_path.exists():
            # 已有 JSON 就不拿 txt 盖掉：那份 JSON 可能已经有正确的字段归属和 nl
            skipped += 1
        else:
            try:
                raw = _read_tag_text(txt)
                selected = []
                for t in _split_tags(raw.replace("\n", ",")):
                    plain = t.replace("\\(", "(").replace("\\)", ")")
                    cat = cat_by_name.get(_normalize_tag_key(plain), "general")
                    selected.append((plain, cat))
                data = _build_simplified_json(selected) if args.simplified else _build_structured_json(selected)
                _write_json_atomic(json_path, data)
                converted += 1
            except Exception as e:
                failed += 1
                log(f"转换失败 {img.name}: {e}")
        progress(i + 1, total, img.name)

    done(converted=converted, skipped=skipped, failed=failed, total=total)


def _write_json_atomic(path, obj):
    write_text_atomic(path, json.dumps(obj, ensure_ascii=False, indent=2))


# JSON 追加标签可选字段 → 完整格式下的 (容器键, 字段键)；简化格式字段名即顶层键
_JSON_APPEND_FIELD_MAP = {
    "quality": ("fixed", "quality"),
    "series": ("fixed", "series"),
    "artist": ("fixed", "artist"),
    "character": ("character", "name"),
    "count": ("ai_output", "count"),
    "appearance": ("ai_output", "appearance"),
    "tags": ("ai_output", "tags"),
    "environment": ("ai_output", "environment"),
}
# 存储为逗号串的字段（builder / tag_manager 约定）；不在此表的为数组字段
_JSON_APPEND_STRING_KEYS = {"quality", "series", "artist", "name", "count"}


def _merge_append_tags_json(data, append_list, simplified, position, field="tags"):
    """JSON 输出合并追加标签（触发词）。

    field：用户选择的目标字段（简化/完整两种布局共用同一套逻辑字段名），
    未知字段回退 tags。nl 不可选——它是自然语言不是标签。
    """
    if field not in _JSON_APPEND_FIELD_MAP:
        field = "tags"
    if simplified:
        container, key = data, field
    else:
        container_key, key = _JSON_APPEND_FIELD_MAP[field]
        container = data.setdefault(container_key, {})
    # 画师字段遵循文件的 @ 前缀约定，追加的词逐个补前缀
    additions = [_format_artist(t) for t in append_list] if field == "artist" else list(append_list)
    arr = container.get(key)
    if isinstance(arr, str):
        # 逗号字符串形式（tag_manager 同样支持）——拆成列表合并，不能整个丢弃
        arr = _split_tags(arr)
        native_str = True
    elif isinstance(arr, list):
        arr = list(arr)
        native_str = False
    else:
        arr = []
        native_str = key in _JSON_APPEND_STRING_KEYS
    arr = _place(arr, additions, position)
    # 按字段原生类型写回：数组字段保持数组，逗号串字段拼回字符串
    container[key] = ", ".join(arr) if native_str else arr
    return data


def _write_outputs(image_path, probs, opts, tags, category_thresholds):
    selected_tags, selected_flat = select_tags(probs, tags, opts, category_thresholds)
    append_list = _split_tags(opts.get("append_tags", ""))
    append_position = opts.get("append_position", "append")
    action = opts.get("existing_tags_action", "overwrite")
    reply = {"image_path": image_path, "tag_count": 0, "skipped": True}
    if opts.get("output_format", "txt") == "json":
        simplified = opts.get("json_simplified", False)
        path = Path(image_path).with_suffix(".json")
        data = _build_simplified_json(selected_tags) if simplified else _build_structured_json(selected_tags)
        merging = action in ("prepend", "append") and path.exists()
        try:
            if merging:
                with open(path, "r", encoding="utf-8") as stream:
                    data = _fill_json(json.load(stream), data)
            if append_list:
                data = _merge_append_tags_json(
                    data, append_list, simplified, append_position, opts.get("json_append_field", "tags"))
            _write_json_atomic(path, data)
        except Exception as err:
            if not merging:
                raise
            log(f"⚠ JSON 合并失败，跳过写入以保护原文件 [{path.name}]: {err}")
            return reply
    else:
        path = Path(image_path).with_suffix(".txt")
        if action in ("prepend", "append") and path.exists():
            try:
                existing = _split_tags(_read_tag_text(path))
                selected_flat = _place(selected_flat, existing, "prepend" if action == "append" else "append")
            except Exception as err:
                log(f"⚠ 读取已有标签失败，跳过写入以保护原文件 [{path.name}]: {err}")
                return reply
        if append_list:
            selected_flat = _place(selected_flat, append_list, append_position)
        write_text_atomic(path, ", ".join(selected_flat))
    return {"image_path": image_path, "tag_count": len(selected_flat)}


def run_detect_mode():
    """--detect 一次性模式：加载 ONNX 模型输出输入信息后退出（Rust detect_model_info 调用）。"""
    try:
        idx = sys.argv.index("--detect")
        model_path = sys.argv[idx + 1]
    except (ValueError, IndexError):
        sys.stderr.write("--detect 需要模型路径参数\n")
        sys.exit(1)
    try:
        import onnxruntime as ort
        from gpu_diagnostics import quiet_session_options
        sess = ort.InferenceSession(
            model_path, quiet_session_options(ort), providers=["CPUExecutionProvider"]
        )
        inp = sess.get_inputs()[0]
        shape = [int(d) if isinstance(d, int) else -1 for d in inp.shape]
        fmt, size = _input_layout(inp.shape)
        emit({
            "type": "model_info",
            "input_size": size,
            "input_format": fmt,
            "input_shape": shape,
        })
    except Exception as e:
        sys.stderr.write(f"模型检测失败: {e}\n")
        sys.exit(1)


def _log_gpu_fallback(provider, err):
    """GPU 建会话失败、即将回退 CPU 时输出原因"""
    err_msg = str(err)
    provider_name = provider[0] if isinstance(provider, tuple) else provider
    log(f"⚠ {provider_name} 加载失败")
    if "cuDNN" in err_msg:
        log("原因: 未找到 cuDNN 9.x — 请安装 cuDNN 9.x for CUDA 12.x")
        log("下载: https://developer.nvidia.com/cudnn-downloads")
    elif "CUDA" in err_msg:
        log("原因: CUDA 运行时未找到 — 请确认 CUDA 12.x 已安装且在 PATH 中")
    else:
        log(f"原因: {err_msg[:200]}")
    log("自动回退到 CPU 推理")


def main():
    if "--detect" in sys.argv:
        run_detect_mode()
        return
    # --convert：txt → JSON 转换模式（无需 ONNX，处理完直接退出）
    if "--convert" in sys.argv:
        run_convert_mode()
        return

    bootstrap()

    import onnxruntime as ort
    from gpu_diagnostics import (create_session_with_cpu_fallback, quiet_session_options,
                                 resolve_ort_providers)

    session = None
    tags = []
    input_format = "NHWC"
    input_size = 448
    input_name = None
    preprocess_mode = "auto"
    category_thresholds = {}

    for line in utf8_stdin():
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
                category_thresholds = cmd.get("category_thresholds", {})

                # 先加载词表：词表损坏时立刻报错，不必等模型加载完
                tags = load_tags(tags_path)

                # === ONNX Runtime 后端 ===
                # 统一流程：探测环境（显卡型号 / CUDA / cuDNN）+ 输出日志 + 决定 providers
                cuda_options = None
                if cmd.get("conservative_cuda", False):
                    # 减少 cuDNN 的启动搜索和 CUDA 内存池扩张，避免大模型推理抢占过多显存。
                    cuda_options = {
                        "cudnn_conv_algo_search": "HEURISTIC",
                        "arena_extend_strategy": "kSameAsRequested",
                        "do_copy_in_default_stream": "1",
                    }
                providers = resolve_ort_providers(
                    log_i18n,
                    use_gpu=use_gpu,
                    cuda_options=cuda_options,
                )
                session = create_session_with_cpu_fallback(
                    model_path, providers, quiet_session_options(ort), _log_gpu_fallback)

                # 检测输入格式
                input_name = session.get_inputs()[0].name
                input_format, detected_size = detect_model_format(session)

                # 输出节点选择与 sigmoid 判定。
                # 默认沿用旧启发式（NCHW = logits 需要 sigmoid，CL Tagger 如此）；
                # output_kind 显式指定时按名称选节点：
                #   probability → 优先 prediction 节点，不做 sigmoid
                #   logits      → 优先 logits 节点，做 sigmoid
                output_names = [o.name for o in session.get_outputs()]
                output_index = 0
                apply_sigmoid = input_format == "NCHW"
                output_kind = cmd.get("output_kind", "auto")
                if output_kind == "probability":
                    if "prediction" in output_names:
                        output_index = output_names.index("prediction")
                    apply_sigmoid = False
                elif output_kind == "logits":
                    if "logits" in output_names:
                        output_index = output_names.index("logits")
                    apply_sigmoid = True

                # input_size
                override_size = cmd.get("input_size", 0)
                if override_size and override_size > 0:
                    input_size = override_size
                else:
                    input_size = detected_size if detected_size > 0 else 448

                log(f"✓ 模型已就绪 ({len(tags)} 标签, {input_size}x{input_size})")

                emit({"type": "ready"})

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
                        error(f"预处理失败: {e}", image_path=img_path)

                if not batch_data:
                    continue

                # 拼接 batch tensor: [N, C, H, W] or [N, H, W, C]
                # 拼接失败也必须给每张图一个终态消息——Rust 按图片数等结果，
                # 少发会让任务一直等不到批次的完整结果
                try:
                    batch_tensor = np.concatenate(batch_data, axis=0)
                except Exception as e:
                    for vi in valid_indices:
                        error(f"批量拼接失败: {e}", image_path=images[vi].get("image_path", ""))
                    continue

                all_probs = None
                fixed_batch = session.get_inputs()[0].shape[0]
                if fixed_batch != 1 or len(batch_data) == 1:
                    try:
                        outputs = session.run(None, {input_name: batch_tensor})
                        all_probs = outputs[output_index]
                    except Exception as e:
                        if len(batch_data) == 1:
                            error(f"推理失败: {e}", image_path=images[valid_indices[0]].get("image_path", ""))
                            continue
                        log(f"批量推理失败，降级为逐张推理重试: {type(e).__name__}")
                if all_probs is None:
                    all_probs = []
                    for bi, vi in enumerate(valid_indices):
                        img_path = images[vi].get("image_path", "")
                        try:
                            out_single = session.run(None, {input_name: batch_data[bi]})
                            all_probs.append(out_single[output_index][0])
                        except Exception as e2:
                            all_probs.append(None)
                            error(f"推理失败: {e2}", image_path=img_path)

                # 逐张处理结果
                for batch_idx, orig_idx in enumerate(valid_indices):
                    img_cmd = images[orig_idx]
                    image_path = img_cmd.get("image_path", "")
                    try:
                        probs = all_probs[batch_idx]
                        if probs is None:
                            continue  # 逐张重试已失败并报过 error

                        if apply_sigmoid:
                            probs = _sigmoid(probs)
                        result(**_write_outputs(image_path, probs, img_cmd, tags, category_thresholds))
                    except Exception as e:
                        error(traceback.format_exc(), image_path=image_path)

            elif cmd["cmd"] == "quit":
                break

            else:
                error(f"未知命令: {cmd['cmd']}")

        except Exception as e:
            err_payload = {"type": "error", "message": f"{traceback.format_exc()}"}
            # 无法定位到单张图片的批次错误由 Rust 终止当前批次。
            img_p = cmd.get("image_path", "") if isinstance(cmd, dict) else ""
            if img_p:
                err_payload["image_path"] = img_p
            emit(err_payload)

if __name__ == "__main__":
    main()
