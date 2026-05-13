#!/usr/bin/env python3
"""图片聚类脚本 — 基于 ResNet50 特征提取 + K-Means/HDBSCAN 聚类
依赖: torch, torchvision, scikit-learn, (umap-learn for HDBSCAN)
"""

import argparse, json, os, sys, shutil, traceback
import numpy as np

# ── JSON 输出 ──────────────────────────────────────

def emit(data):
    print(json.dumps(data, ensure_ascii=False), flush=True)

def emit_log(msg):
    emit({"type": "log", "message": msg})

def emit_error(msg):
    emit({"type": "error", "message": msg})

def emit_progress(cur, total, fname, status, msg=""):
    emit({"type": "progress", "current": cur, "total": total,
          "filename": fname, "status": status, "message": msg or fname})

def emit_done(msg):
    emit({"type": "done", "message": msg})

# ── 图片收集 ──────────────────────────────────────

IMAGE_EXTS = {'.jpg', '.jpeg', '.png', '.webp', '.bmp', '.tiff', '.tif'}

def collect_images(path):
    if os.path.isfile(path):
        return [path]
    files = []
    for f in sorted(os.listdir(path)):
        ext = os.path.splitext(f)[1].lower()
        if ext in IMAGE_EXTS:
            files.append(os.path.join(path, f))
    return files

# ── 设备检测 ──────────────────────────────────────

def detect_device():
    import torch
    if torch.cuda.is_available():
        name = torch.cuda.get_device_name(0)
        emit_log(f"使用 GPU: {name} (CUDA)")
        return "cuda"
    if hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
        emit_log("使用 GPU: Apple Silicon (MPS)")
        return "mps"
    emit_log("使用 CPU 推理（速度较慢）")
    return "cpu"

# ── 颜色直方图特征 ──────────────────────────────────────

def extract_color_histogram(img_path, bins=64):
    """提取 HSV 颜色直方图作为颜色特征"""
    from PIL import Image
    try:
        img = Image.open(img_path).convert("RGB")
    except Exception:
        return None

    img = img.resize((224, 224))
    arr = np.array(img, dtype=np.float32) / 255.0

    # 简易 RGB → HSV
    r, g, b = arr[:,:,0], arr[:,:,1], arr[:,:,2]
    cmax = np.max(arr, axis=2)
    cmin = np.min(arr, axis=2)
    diff = cmax - cmin + 1e-10

    # Hue
    h = np.zeros_like(cmax)
    mask_r = (cmax == r)
    mask_g = (cmax == g) & ~mask_r
    mask_b = ~mask_r & ~mask_g
    h[mask_r] = (60 * ((g[mask_r] - b[mask_r]) / diff[mask_r]) + 360) % 360
    h[mask_g] = (60 * ((b[mask_g] - r[mask_g]) / diff[mask_g]) + 120) % 360
    h[mask_b] = (60 * ((r[mask_b] - g[mask_b]) / diff[mask_b]) + 240) % 360

    # Saturation
    s = np.where(cmax > 0, diff / (cmax + 1e-10), 0)

    # Value
    v = cmax

    # 计算直方图
    h_hist, _ = np.histogram(h.ravel(), bins=bins, range=(0, 360))
    s_hist, _ = np.histogram(s.ravel(), bins=bins // 2, range=(0, 1))
    v_hist, _ = np.histogram(v.ravel(), bins=bins // 2, range=(0, 1))

    hist = np.concatenate([h_hist, s_hist, v_hist]).astype(np.float32)
    norm = np.linalg.norm(hist)
    if norm > 0:
        hist = hist / norm
    return hist

# ── 特征提取 ──────────────────────────────────────

class FeatureExtractor:
    """使用 ResNet50 提取图片特征"""

    def __init__(self, feature_type, device, weights=None):
        """
        feature_type: "style" | "semantic" | "color" | "fusion"
        weights: dict with keys "style", "semantic", "color" (0.0~1.0) for fusion mode
        """
        self.feature_type = feature_type
        self.device = device
        self.weights = weights or {"style": 0.5, "semantic": 0.5, "color": 0.0}

        # 颜色模式不需要 ResNet
        if feature_type == "color":
            emit_log("使用颜色直方图特征（无需 GPU）")
            self.model = None
            return

        import torch
        import torchvision.models as models
        import torchvision.transforms as T

        # 加载预训练 ResNet50
        emit_log("加载 ResNet50 预训练模型...")
        self.model = models.resnet50(weights=models.ResNet50_Weights.DEFAULT)
        self.model.eval().to(device)

        # 注册 hook
        self.style_features = []
        self.semantic_feature = None

        need_style = feature_type == "style" or (feature_type == "fusion" and self.weights.get("style", 0) > 0)
        need_semantic = feature_type == "semantic" or (feature_type == "fusion" and self.weights.get("semantic", 0) > 0)

        if need_style:
            self.model.layer3.register_forward_hook(self._style_hook)
        if need_semantic:
            self.model.avgpool.register_forward_hook(self._semantic_hook)

        self.transform = T.Compose([
            T.Resize(256),
            T.CenterCrop(224),
            T.ToTensor(),
            T.Normalize(mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]),
        ])

    def _style_hook(self, module, input, output):
        self.style_features = output

    def _semantic_hook(self, module, input, output):
        self.semantic_feature = output

    def _gram_matrix(self, feat):
        """计算 Gram Matrix 作为风格特征"""
        import torch
        b, c, h, w = feat.shape
        feat = feat.view(b, c, -1)
        gram = torch.bmm(feat, feat.transpose(1, 2))
        gram = gram / (c * h * w)
        idx = torch.triu_indices(c, c)
        return gram[:, idx[0], idx[1]]

    def extract(self, img_path):
        """提取单张图片特征，返回 numpy 向量"""
        import torch

        # 纯颜色模式
        if self.feature_type == "color":
            return extract_color_histogram(img_path)

        from PIL import Image
        try:
            img = Image.open(img_path).convert("RGB")
        except Exception:
            return None

        tensor = self.transform(img).unsqueeze(0).to(self.device)

        with torch.no_grad():
            self.model(tensor)

        if self.feature_type == "style":
            gram = self._gram_matrix(self.style_features)
            vec = gram.squeeze(0).cpu().numpy()
            norm = np.linalg.norm(vec)
            return vec / norm if norm > 0 else vec

        if self.feature_type == "semantic":
            vec = self.semantic_feature.squeeze().cpu().numpy()
            norm = np.linalg.norm(vec)
            return vec / norm if norm > 0 else vec

        # fusion: 加权拼接
        parts = []
        w_style = self.weights.get("style", 0)
        w_semantic = self.weights.get("semantic", 0)
        w_color = self.weights.get("color", 0)

        if w_style > 0 and self.style_features is not None and len(self.style_features) > 0:
            gram = self._gram_matrix(self.style_features)
            vec = gram.squeeze(0).cpu().numpy()
            norm = np.linalg.norm(vec)
            if norm > 0:
                vec = vec / norm
            parts.append(vec * w_style)

        if w_semantic > 0 and self.semantic_feature is not None:
            vec = self.semantic_feature.squeeze().cpu().numpy()
            norm = np.linalg.norm(vec)
            if norm > 0:
                vec = vec / norm
            parts.append(vec * w_semantic)

        if w_color > 0:
            color_vec = extract_color_histogram(img_path)
            if color_vec is not None:
                parts.append(color_vec * w_color)

        if not parts:
            return None
        return np.concatenate(parts)

# ── 聚类 ──────────────────────────────────────

def cluster_kmeans(features, n_clusters):
    from sklearn.cluster import KMeans
    emit_log(f"K-Means 聚类 (k={n_clusters})...")
    km = KMeans(n_clusters=n_clusters, random_state=42, n_init=10)
    labels = km.fit_predict(features)
    return labels

def cluster_hdbscan(features, min_cluster_size=5):
    # HDBSCAN 在高维效果差，先用 UMAP 降维
    emit_log("UMAP 降维中...")
    try:
        from umap import UMAP
    except ImportError:
        emit_log("umap-learn 未安装，尝试直接高维聚类...")
        from sklearn.decomposition import PCA
        if features.shape[1] > 50:
            pca = PCA(n_components=50, random_state=42)
            features = pca.fit_transform(features)
            emit_log(f"PCA 降维到 {features.shape[1]} 维")
    else:
        n_components = min(50, features.shape[1], features.shape[0] - 1)
        n_neighbors = min(15, features.shape[0] - 1)  # 不能超过样本数-1
        if n_neighbors < 2 or features.shape[0] < 5:
            # 数据太少，UMAP 无意义，用 PCA
            emit_log(f"样本数过少 ({features.shape[0]})，改用 PCA 降维")
            from sklearn.decomposition import PCA
            n_pca = min(50, features.shape[1], features.shape[0] - 1)
            if n_pca > 0 and features.shape[1] > n_pca:
                pca = PCA(n_components=n_pca, random_state=42)
                features = pca.fit_transform(features)
                emit_log(f"PCA 降维到 {features.shape[1]} 维")
        else:
            try:
                reducer = UMAP(n_components=n_components, n_neighbors=n_neighbors,
                               min_dist=0.1, spread=1.0, random_state=42)
                features = reducer.fit_transform(features)
                emit_log(f"UMAP 降维到 {features.shape[1]} 维")
            except Exception as e:
                emit_log(f"⚠ UMAP 降维失败: {e}")
                emit_log("改用 PCA 降维...")
                from sklearn.decomposition import PCA
                n_pca = min(50, features.shape[1], features.shape[0] - 1)
                if n_pca > 0 and features.shape[1] > n_pca:
                    pca = PCA(n_components=n_pca, random_state=42)
                    features = pca.fit_transform(features)
                    emit_log(f"PCA 降维到 {features.shape[1]} 维")

    emit_log(f"HDBSCAN 聚类 (min_cluster_size={min_cluster_size})...")
    try:
        from sklearn.cluster import HDBSCAN as SkHDBSCAN
        hdb = SkHDBSCAN(min_cluster_size=min_cluster_size)
    except ImportError:
        from hdbscan import HDBSCAN
        hdb = HDBSCAN(min_cluster_size=min_cluster_size)

    labels = hdb.fit_predict(features)
    return labels

# ── 主流程 ──────────────────────────────────────

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True)
    ap.add_argument("--output", required=True)
    ap.add_argument("--algorithm", default="kmeans", choices=["kmeans", "hdbscan"])
    ap.add_argument("--feature", default="semantic", choices=["style", "semantic", "color", "fusion"])
    ap.add_argument("--n-clusters", type=int, default=8)
    ap.add_argument("--min-cluster-size", type=int, default=5)
    ap.add_argument("--device", default="auto", choices=["auto", "cpu"])
    ap.add_argument("--weight-style", type=float, default=0.5)
    ap.add_argument("--weight-semantic", type=float, default=0.5)
    ap.add_argument("--weight-color", type=float, default=0.0)
    ap.add_argument("--model-dir", default="", help="PyTorch 模型缓存目录")
    ap.add_argument("--map-theme", default="light", choices=["light", "dark"], help="分布图主题")
    args = ap.parse_args()

    # 设置模型缓存目录
    if args.model_dir:
        os.makedirs(args.model_dir, exist_ok=True)
        os.environ["TORCH_HOME"] = args.model_dir
        emit_log(f"模型缓存目录: {args.model_dir}")

    # 收集图片
    files = collect_images(args.input)
    if not files:
        emit_error("未找到图片文件")
        sys.exit(1)

    total = len(files)
    emit_log(f"找到 {total} 张图片")
    emit_log(f"算法: {args.algorithm} | 特征: {args.feature}")

    if args.feature == "fusion":
        emit_log(f"融合权重: 风格={args.weight_style:.1f} 语义={args.weight_semantic:.1f} 颜色={args.weight_color:.1f}")

    # 检测设备
    if args.device == "cpu" or args.feature == "color":
        device = "cpu"
        if args.feature == "color":
            emit_log("颜色特征不需要 GPU")
        else:
            emit_log("使用 CPU 推理")
    else:
        device = detect_device()

    # 提取特征
    weights = {"style": args.weight_style, "semantic": args.weight_semantic, "color": args.weight_color}
    extractor = FeatureExtractor(args.feature, device, weights)

    features = []
    valid_files = []
    # 总步数 = 提取 + 聚类(1) + 分布图(1) + 复制文件数
    # 先用提取阶段的 total，后续更新
    phase_total = total + 3  # 预估：提取 + 聚类 + 分布图 + 完成

    for i, fpath in enumerate(files):
        fname = os.path.basename(fpath)
        emit_progress(i + 1, phase_total, fname, "processing", f"[{i+1}/{total}] 提取特征: {fname}")

        vec = extractor.extract(fpath)
        if vec is not None:
            features.append(vec)
            valid_files.append(fpath)
        else:
            emit_progress(i + 1, phase_total, fname, "error", f"[{i+1}/{total}] ✗ 无法读取: {fname}")

    if len(valid_files) < 2:
        emit_error("有效图片不足 2 张，无法聚类")
        sys.exit(1)

    features = np.array(features)
    emit_log(f"特征维度: {features.shape[1]}，有效图片: {len(valid_files)}")

    # 更新总步数：提取完成的 + 聚类 + 分布图 + 复制文件
    phase_total = total + 2 + len(valid_files)
    step = total  # 当前步骤：提取已完成 total 步

    # 聚类
    step += 1
    emit_progress(step, phase_total, "", "processing", "聚类计算中...")
    if args.algorithm == "kmeans":
        k = min(args.n_clusters, len(valid_files))
        labels = cluster_kmeans(features, k)
    else:
        labels = cluster_hdbscan(features, args.min_cluster_size)

        # HDBSCAN 失败兜底：0 个有效分组时自动切换 K-Means
        n_valid_clusters = len([l for l in set(labels) if l >= 0])
        if n_valid_clusters == 0:
            fallback_k = max(2, min(8, len(valid_files) // 3))
            emit_log(f"⚠ HDBSCAN 未找到有效分组（可能数据量太少），自动切换 K-Means (k={fallback_k})")
            labels = cluster_kmeans(features, fallback_k)

    # 统计各簇
    unique_labels = sorted(set(labels))
    n_clusters = len([l for l in unique_labels if l >= 0])
    n_noise = sum(1 for l in labels if l < 0)

    emit_log(f"聚类完成: {n_clusters} 个分组" + (f", {n_noise} 个噪声点" if n_noise > 0 else ""))

    for label in unique_labels:
        count = sum(1 for l in labels if l == label)
        name = f"noise" if label < 0 else f"cluster_{label}"
        emit_log(f"  {name}: {count} 张")

    # 生成聚类分布图
    step += 1
    emit_progress(step, phase_total, "", "processing", "生成聚类分布图...")
    try:
        generate_distribution_map(features, labels, valid_files, args.output, theme=args.map_theme)
        emit_log("✓ 聚类分布图已保存")
    except Exception as e:
        emit_log(f"⚠ 分布图生成失败: {traceback.format_exc()}")

    # 复制文件到输出目录
    emit_log("复制文件到分组目录...")
    os.makedirs(args.output, exist_ok=True)

    success_count = 0
    fail_count = 0
    errors = []

    for i, (fpath, label) in enumerate(zip(valid_files, labels)):
        fname = os.path.basename(fpath)
        folder_name = "noise" if label < 0 else f"cluster_{label}"
        dest_dir = os.path.join(args.output, folder_name)
        os.makedirs(dest_dir, exist_ok=True)
        dest_path = os.path.join(dest_dir, fname)

        step += 1
        try:
            shutil.copy2(fpath, dest_path)
            success_count += 1
            emit_progress(step, phase_total, fname, "success",
                          f"[{i+1}/{len(valid_files)}] ✓ {fname} → {folder_name}/")
        except Exception as e:
            fail_count += 1
            err_msg = f"{fname}: {e}"
            errors.append(err_msg)
            emit_progress(step, phase_total, fname, "error",
                          f"[{i+1}/{len(valid_files)}] ✗ {err_msg}")

    emit_done(f"完成: {n_clusters} 个分组, 成功 {success_count}, 失败 {fail_count}, 共 {len(valid_files)}")

    emit({"type": "result", "success_count": success_count, "fail_count": fail_count,
          "total": len(valid_files), "n_clusters": n_clusters, "errors": errors})


def generate_distribution_map(features, labels, file_paths, output_dir, theme="light"):
    """使用 t-SNE 降维到 2D，生成高分辨率聚类分布图"""
    from PIL import Image, ImageDraw, ImageFont

    is_dark = (theme == "dark")

    n_samples = features.shape[0]
    features = features.copy()  # 不修改原始特征

    # 降到 2D 坐标
    from sklearn.decomposition import PCA

    try:
        # 1) 高维先 PCA 降到 min(50, n-1)
        n_pca = min(50, features.shape[1], n_samples - 1)
        if n_pca > 2 and features.shape[1] > n_pca:
            pca = PCA(n_components=n_pca, random_state=42)
            features = pca.fit_transform(features)

        # 2) 降到 2D
        if n_samples <= 3:
            pca2 = PCA(n_components=min(2, features.shape[1]), random_state=42)
            coords_2d = pca2.fit_transform(features)
        else:
            from sklearn.manifold import TSNE
            perplexity = max(2, min(30, (n_samples - 1) // 3))
            tsne = TSNE(n_components=2, perplexity=perplexity, random_state=42,
                         n_iter=1000, init='random', learning_rate='auto')
            coords_2d = tsne.fit_transform(features)
    except Exception:
        # t-SNE 失败，直接用 PCA 到 2D
        pca_fallback = PCA(n_components=min(2, features.shape[1]), random_state=42)
        coords_2d = pca_fallback.fit_transform(features)

    # 画布参数（横图 16:9）
    canvas_w = 6144
    canvas_h = 3456
    thumb_size = 112
    margin = 120       # 边距
    border_w = 4       # 缩略图边框宽度

    # 簇颜色调色板（最多 20 种 + noise 灰色）
    palette = [
        (99, 102, 241),   # indigo
        (244, 63, 94),    # rose
        (34, 197, 94),    # green
        (251, 146, 60),   # orange
        (59, 130, 246),   # blue
        (168, 85, 247),   # purple
        (20, 184, 166),   # teal
        (245, 158, 11),   # amber
        (236, 72, 153),   # pink
        (16, 185, 129),   # emerald
        (139, 92, 246),   # violet
        (6, 182, 212),    # cyan
        (217, 70, 239),   # fuchsia
        (132, 204, 22),   # lime
        (234, 88, 12),    # deep orange
        (79, 70, 229),    # deep indigo
        (225, 29, 72),    # crimson
        (13, 148, 136),   # dark teal
        (202, 138, 4),    # dark amber
        (124, 58, 237),   # deep violet
    ]
    noise_color = (120, 120, 120)  # 灰色

    def get_cluster_color(label):
        if label < 0:
            return noise_color
        return palette[label % len(palette)]

    # 坐标归一化到画布区域
    draw_w = canvas_w - 2 * margin - thumb_size
    draw_h = canvas_h - 2 * margin - thumb_size
    x_min, x_max = coords_2d[:, 0].min(), coords_2d[:, 0].max()
    y_min, y_max = coords_2d[:, 1].min(), coords_2d[:, 1].max()

    # 防止除零
    x_range = x_max - x_min if x_max > x_min else 1.0
    y_range = y_max - y_min if y_max > y_min else 1.0

    def to_canvas(x, y):
        cx = margin + int((x - x_min) / x_range * draw_w)
        cy = margin + int((y - y_min) / y_range * draw_h)
        return cx, cy

    # 创建画布
    if is_dark:
        bg_color = (24, 24, 32)
        grid_color = (45, 45, 58)
        title_color = (200, 200, 220)
        legend_text_color = (180, 180, 200)
    else:
        bg_color = (248, 248, 252)
        grid_color = (225, 225, 235)
        title_color = (40, 40, 60)
        legend_text_color = (60, 60, 80)

    canvas = Image.new("RGB", (canvas_w, canvas_h), bg_color)
    draw = ImageDraw.Draw(canvas)

    # 绘制网格线
    grid_count = 10
    for i in range(grid_count + 1):
        # 水平线
        y_pos = margin + int(draw_h * i / grid_count) + thumb_size // 2
        draw.line([(margin, y_pos), (canvas_w - margin, y_pos)], fill=grid_color, width=1)
        # 垂直线
        x_pos = margin + int(draw_w * i / grid_count) + thumb_size // 2
        draw.line([(x_pos, margin), (x_pos, canvas_h - margin)], fill=grid_color, width=1)

    # 绘制簇连线（同簇图片之间画淡色线条）
    unique_labels = sorted(set(labels))
    for label in unique_labels:
        if label < 0:
            continue
        indices = [i for i, l in enumerate(labels) if l == label]
        if len(indices) < 2:
            continue
        color = get_cluster_color(label)
        # 连线颜色：根据主题调整透明度
        if is_dark:
            line_color = (min(255, color[0] // 2 + 40), min(255, color[1] // 2 + 40), min(255, color[2] // 2 + 40))
        else:
            line_color = (min(255, color[0] // 2 + 180), min(255, color[1] // 2 + 180), min(255, color[2] // 2 + 180))
        # 画到簇中心的连线
        cx_sum = sum(coords_2d[i, 0] for i in indices) / len(indices)
        cy_sum = sum(coords_2d[i, 1] for i in indices) / len(indices)
        center = to_canvas(cx_sum, cy_sum)
        center = (center[0] + thumb_size // 2, center[1] + thumb_size // 2)
        for idx in indices:
            pt = to_canvas(coords_2d[idx, 0], coords_2d[idx, 1])
            pt = (pt[0] + thumb_size // 2, pt[1] + thumb_size // 2)
            draw.line([center, pt], fill=line_color, width=2)

    # 绘制缩略图
    for i in range(n_samples):
        x, y = to_canvas(coords_2d[i, 0], coords_2d[i, 1])
        color = get_cluster_color(labels[i])

        try:
            img = Image.open(file_paths[i]).convert("RGB")
            img.thumbnail((thumb_size - border_w * 2, thumb_size - border_w * 2), Image.LANCZOS)

            # 创建带边框的缩略图
            thumb_w = img.width + border_w * 2
            thumb_h = img.height + border_w * 2
            bordered = Image.new("RGB", (thumb_w, thumb_h), color)
            bordered.paste(img, (border_w, border_w))

            canvas.paste(bordered, (x, y))
        except Exception:
            # 图片读取失败，画一个色块
            draw.rectangle([x, y, x + thumb_size, y + thumb_size], fill=color, outline=color)

    # 绘制图例 — 使用支持中文的字体
    legend_x = margin
    legend_y = canvas_h - margin + 20

    import platform
    _sys = platform.system()
    font = None
    font_small = None

    # 按平台尝试中文字体
    font_candidates = []
    if _sys == "Darwin":
        font_candidates = [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ]
    elif _sys == "Windows":
        font_candidates = [
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\simhei.ttf",
        ]
    else:
        font_candidates = [
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        ]

    for fp in font_candidates:
        try:
            font = ImageFont.truetype(fp, 28)
            font_small = ImageFont.truetype(fp, 22)
            break
        except Exception:
            continue

    if font is None:
        font = ImageFont.load_default()
        font_small = font

    # 标题
    title = f"聚类分布图 — {n_samples} 张图片, {len([l for l in unique_labels if l >= 0])} 个分组"
    draw.text((margin, 30), title, fill=title_color, font=font)

    # 簇图例
    lx = legend_x
    for label in unique_labels:
        count = sum(1 for l in labels if l == label)
        color = get_cluster_color(label)
        name = "noise" if label < 0 else f"cluster_{label}"
        draw.rectangle([lx, legend_y, lx + 20, legend_y + 20], fill=color)
        draw.text((lx + 26, legend_y - 2), f"{name} ({count})", fill=legend_text_color, font=font_small)
        lx += 220
        if lx > canvas_w - 250:
            lx = legend_x
            legend_y += 32

    # 保存
    os.makedirs(output_dir, exist_ok=True)
    out_path = os.path.join(output_dir, "cluster_distribution.png")
    canvas.save(out_path, quality=95)
    emit_log(f"分布图: {out_path}")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        emit_error(f"致命错误: {traceback.format_exc()}")
        sys.exit(1)
