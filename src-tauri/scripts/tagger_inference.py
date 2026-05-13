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

def log(msg):
    """输出日志到 stdout (JSON line)"""
    print(json.dumps({"type": "log", "message": msg}), flush=True)

def error(msg):
    """输出错误到 stdout (JSON line)"""
    print(json.dumps({"type": "error", "message": msg}), flush=True)

def result(data):
    """输出结果到 stdout (JSON line)"""
    print(json.dumps(data), flush=True)

def preprocess_image(image_path, target_size, input_format):
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
        for row in reader:
            if len(row) >= 3:
                name = row[1]
                cat_id = int(row[2])
                category = category_map.get(cat_id, "general")
                tags.append({"name": name, "category": category})
    return tags

def load_tags_json(json_path):
    """从 JSON 加载标签定义 (CL Tagger 格式)"""
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    tags = []
    for idx_str in sorted(data.keys(), key=int):
        info = data[idx_str]
        tag_name = info.get("tag", "")
        category = info.get("category", "General").lower()
        tags.append({"name": tag_name, "category": category})
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

def main():
    # Windows Python 3.8+: 必须在 import onnxruntime 前注册 CUDA DLL 目录
    # 否则 onnxruntime_providers_cuda.dll 加载时找不到 CUDA/cuDNN 依赖
    if sys.platform == "win32" and hasattr(os, "add_dll_directory"):
        cuda_dirs = set()

        def add_dir_with_subdirs(d):
            """添加目录及其子目录"""
            if os.path.isdir(d):
                cuda_dirs.add(d)
                try:
                    for sub in os.listdir(d):
                        sub_path = os.path.join(d, sub)
                        if os.path.isdir(sub_path):
                            cuda_dirs.add(sub_path)
                except PermissionError:
                    pass

        def read_reg_env(name):
            """从 Windows 注册表读取环境变量（GUI 进程可能没有最新值）"""
            import subprocess
            for root in [r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment", r"HKCU\Environment"]:
                try:
                    result = subprocess.run(["reg", "query", root, "/v", name],
                                          capture_output=True, text=True, creationflags=0x08000000)
                    for line in result.stdout.splitlines():
                        line = line.strip()
                        if line.startswith(name):
                            parts = line.split(None, 2)
                            if len(parts) >= 3:
                                return parts[2]
                except Exception:
                    pass
            return None

        # 1. CUDA 路径：从环境变量读取 + 注册表回退
        cuda_paths = {}
        for key, val in os.environ.items():
            if key in ("CUDA_PATH", "CUDA_HOME") or key.startswith("CUDA_PATH_V"):
                cuda_paths[key] = val
        # 注册表补充
        if "CUDA_PATH" not in cuda_paths:
            reg_val = read_reg_env("CUDA_PATH")
            if reg_val:
                cuda_paths["CUDA_PATH"] = reg_val

        for key, val in cuda_paths.items():
            bin_dir = os.path.join(val, "bin")
            bin_x64 = os.path.join(val, "bin", "x64")  # cuDNN 9.x
            lib_x64 = os.path.join(val, "lib", "x64")
            for d in [bin_dir, bin_x64, lib_x64]:
                if os.path.isdir(d):
                    cuda_dirs.add(d)

        # 2. cuDNN 路径：从环境变量读取
        cudnn_path = os.environ.get("CUDNN_PATH", "")
        if cudnn_path:
            # cuDNN 9.x 结构: bin/12.x 子目录
            add_dir_with_subdirs(os.path.join(cudnn_path, "bin"))
            add_dir_with_subdirs(os.path.join(cudnn_path, "lib"))

        # 3. 搜索 PATH 中包含 CUDA/cuDNN DLL 的目录 + 子目录
        for p in os.environ.get("PATH", "").split(os.pathsep):
            if os.path.isdir(p):
                try:
                    for f in os.listdir(p):
                        fl = f.lower()
                        if fl.startswith("cudnn") or fl.startswith("cublas") or fl.startswith("cufft") or fl.startswith("nvinfer"):
                            add_dir_with_subdirs(p)
                            break
                except PermissionError:
                    pass

        # 注册所有目录：同时加到 PATH 和 add_dll_directory
        # onnxruntime 的 C++ 层用 LoadLibrary 加载 cuDNN，只看 PATH
        current_path = os.environ.get("PATH", "")
        new_dirs = []
        for d in sorted(cuda_dirs):
            try:
                os.add_dll_directory(d)
            except OSError:
                pass
            if d not in current_path:
                new_dirs.append(d)
        if new_dirs:
            os.environ["PATH"] = os.pathsep.join(new_dirs) + os.pathsep + current_path

    import onnxruntime as ort

    session = None
    tags = []
    input_format = "NHWC"
    input_size = 448
    input_name = None

    for line in sys.stdin:
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

                # === ONNX Runtime 后端 ===
                available = ort.get_available_providers()
                gpu_provider = None
                if use_gpu:
                    if "CUDAExecutionProvider" in available:
                        gpu_provider = "CUDAExecutionProvider"
                        log("使用 GPU (CUDA) 加速")
                    elif "CoreMLExecutionProvider" in available:
                        gpu_provider = "CoreMLExecutionProvider"
                        log("使用 Apple Neural Engine / GPU (CoreML) 加速")
                    else:
                        log("GPU 加速不可用，使用 CPU 推理")

                if gpu_provider:
                    providers = [gpu_provider, "CPUExecutionProvider"]
                else:
                    providers = ["CPUExecutionProvider"]
                    if not use_gpu:
                        log("使用 CPU 推理")

                # GPU 模式下打印诊断信息（仅 Windows CUDA）
                if use_gpu and sys.platform == "win32" and gpu_provider == "CUDAExecutionProvider":
                    import ctypes, glob
                    cudnn_found = False
                    for p in os.environ.get("PATH", "").split(os.pathsep):
                        cudnn_dlls = glob.glob(os.path.join(p, "cudnn*.dll"))
                        if cudnn_dlls:
                            cudnn_found = True
                            break
                    if not cudnn_found:
                        log("⚠ 未在 PATH 中找到 cuDNN DLL (cudnn*.dll)")

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
                img_data = preprocess_image(image_path, input_size, input_format)

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
                selected_tags = []      # (tag_name, category, prob) 用于分类
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
                    if replace_underscore:
                        tag_name = tag_name.replace("_", " ")
                    if tag_name in exclude_set or tags[best_idx]["name"] in exclude_set:
                        continue
                    selected_tags.append((tag_name, argmax_cat, best_prob))
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
                        if replace_underscore:
                            tag_name = tag_name.replace("_", " ")
                        if tag_name in exclude_set or tags[idx]["name"] in exclude_set:
                            continue
                        selected_tags.append((tag_name, cat, prob))
                        selected_flat.append(tag_name)

                # 追加标签
                if append_list:
                    if append_position == "prepend":
                        selected_flat = append_list + selected_flat
                        selected_tags = [(t, "general", 1.0) for t in append_list] + selected_tags
                    else:
                        selected_flat = selected_flat + append_list
                        selected_tags = selected_tags + [(t, "general", 1.0) for t in append_list]

                # 输出格式
                output_format = cmd.get("output_format", "txt")
                stem = Path(image_path).stem
                parent = Path(image_path).parent

                if output_format == "json":
                    json_simplified = cmd.get("json_simplified", False)
                    json_path = parent / f"{stem}.json"
                    if json_simplified:
                        data = _build_simplified_json(selected_tags)
                    else:
                        data = _build_structured_json(selected_tags)
                    with open(json_path, "w", encoding="utf-8") as f:
                        json.dump(data, f, ensure_ascii=False, indent=2)
                else:
                    txt_path = parent / f"{stem}.txt"
                    with open(txt_path, "w", encoding="utf-8") as f:
                        f.write(", ".join(selected_flat))

                result({
                    "type": "result",
                    "image_path": image_path,
                    "tags": selected_flat,
                    "tag_count": len(selected_flat),
                })

            elif cmd["cmd"] == "quit":
                log("推理进程退出")
                break

            else:
                error(f"未知命令: {cmd['cmd']}")

        except Exception as e:
            error(f"{traceback.format_exc()}")

if __name__ == "__main__":
    main()
