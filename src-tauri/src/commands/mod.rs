use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

pub mod aesthetic;
pub mod alpha_convert;
pub mod api_config;
pub mod batch;
pub mod batch_rename;
pub mod blur_noise;
pub mod bucket_preview;
pub mod config_paths;
pub mod dedup_rename;
pub mod file_keeper;
pub mod fingerprint;
pub mod format_convert;
pub mod http_download;
pub mod huggingface_config;
pub mod image_cluster;
pub mod image_crop;
pub mod image_dedup;
pub mod image_flip;
pub mod image_io;
pub mod image_scale;
pub mod llm_batch;
pub mod llm_client;
pub mod person_crop;
pub mod perspective;
pub mod proxy_config;
pub mod python_env;
pub mod python_proc;
pub mod resolution_analyze;
pub mod resolution_filter;
pub mod sd_metadata;
pub mod tag_db;
pub mod tag_manager;
pub mod tag_refine;
pub mod tag_sort;
pub mod tagger;
pub mod thumbnail;
pub mod translator;
pub mod upscale;
pub mod workflow;

#[cfg(test)]
mod workflow_node_tests;

/// 下载临时文件辅助：返回 `{dest}.part` 临时路径，并清理上次中断遗留的旧残件。
/// 下载应先写入 .part 文件，完成校验后再用 `finalize_part_file` 原子替换到最终路径，
/// 避免中断产生的不完整文件被 `dest.exists()` 误判为已下载。
pub fn prepare_part_file(dest: &Path) -> std::path::PathBuf {
    let mut os = dest.as_os_str().to_os_string();
    os.push(".part");
    let part = std::path::PathBuf::from(os);
    if part.exists() {
        let _ = std::fs::remove_file(&part);
    }
    part
}

/// 完成下载：校验实际字节数（若服务器提供了 content-length），通过后把 .part 重命名为最终文件。
/// Windows 上 rename 不能覆盖已存在的目标，因此先删除旧的最终文件再重命名。
/// 任何失败路径都会清理 .part 残件。
pub fn finalize_part_file(
    part: &Path,
    dest: &Path,
    downloaded: u64,
    total_size: u64,
) -> Result<(), String> {
    if total_size > 0 && downloaded != total_size {
        let _ = std::fs::remove_file(part);
        return Err(format!(
            "下载不完整: 预期 {} 字节，实际 {} 字节",
            total_size, downloaded
        ));
    }
    if dest.exists() {
        if let Err(e) = std::fs::remove_file(dest) {
            let _ = std::fs::remove_file(part);
            return Err(format!("删除旧文件失败 {}: {}", dest.display(), e));
        }
    }
    if let Err(e) = std::fs::rename(part, dest) {
        let _ = std::fs::remove_file(part);
        return Err(format!("替换文件失败 {}: {}", dest.display(), e));
    }
    Ok(())
}

/// 前端是否已完成加载（启动看门狗用，见 lib.rs setup）
pub static FRONTEND_READY: AtomicBool = AtomicBool::new(false);

/// 前端加载完成后调用，通知后端界面已正常初始化
#[tauri::command]
pub fn frontend_ready() {
    FRONTEND_READY.store(true, Ordering::SeqCst);
}

/// 进度事件 payload（各命令的 `*-progress` 事件共用）
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProgressEvent {
    pub current: u32,
    pub total: u32,
    pub filename: String,
    /// "processing" | "success" | "error" | "done" | "info" | "warning" | "skipped"
    pub status: String,
    pub message: String,
    /// i18n key（Python 脚本发送的国际化 key，前端优先使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i18n_key: Option<String>,
    /// i18n 参数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i18n_params: Option<serde_json::Value>,
}

impl ProgressEvent {
    /// current/total 为 0、filename 为空的事件，按需再接 `.at()` / `.file()`
    pub fn new(status: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            message: message.into(),
            ..Default::default()
        }
    }

    pub fn at(mut self, current: u32, total: u32) -> Self {
        self.current = current;
        self.total = total;
        self
    }

    pub fn file(mut self, name: impl Into<String>) -> Self {
        self.filename = name.into();
        self
    }

    /// Python 协议里的 `{"type":"log", ...}` 行转成 info 事件。
    /// `i18n_params` 原样带过去：缺省时为 None，写成 null 时是 `Some(Null)`（序列化为 null）。
    pub fn python_log(msg: &serde_json::Value, current: u32, total: u32) -> Self {
        Self {
            current,
            total,
            filename: String::new(),
            status: "info".to_string(),
            message: msg
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            i18n_key: msg
                .get("i18n_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            i18n_params: msg.get("i18n_params").cloned(),
        }
    }

    /// 发给前端；与原先的 `let _ = app.emit(..)` 一样忽略发送失败
    pub fn emit<R: tauri::Runtime>(self, app: &tauri::AppHandle<R>, event: &str) {
        let _ = app.emit(event, self);
    }
}

/// 运行时放行 asset 协议目录。assetProtocol scope 已从 `**` 收窄为空，
/// 渲染进程只能读取显式放行的目录——前端在用户选择/加载数据集目录时调用。
#[tauri::command]
pub fn allow_asset_dir(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri::Manager;
    let dir = dir_of(Path::new(&path));
    app.asset_protocol_scope()
        .allow_directory(&dir, true)
        .map_err(|e| format!("放行目录失败: {}", e))
}

/// 数据集图片的扩展名（小写）
pub(crate) const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];

/// 扩展名转小写后是否在 `exts` 里
pub(crate) fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .is_some_and(|ext| exts.contains(&ext.to_string_lossy().to_lowercase().as_str()))
}

pub(crate) fn is_supported_image_file(path: &Path) -> bool {
    has_extension(path, IMAGE_EXTS)
}

/// 模型因内容安全审核拒绝时的典型措辞（小写匹配）
const REFUSAL_MARKERS: [&str; 18] = [
    "i'm sorry",
    "i am sorry",
    "i cannot",
    "i can't",
    "i'm unable",
    "i am unable",
    "unable to assist",
    "cannot assist",
    "can't assist",
    "cannot help",
    "can't help",
    "cannot provide",
    "can't provide",
    "against my",
    "content policy",
    "抱歉",
    "无法处理",
    "不能提供",
];

/// 判断整段回复是不是"模型拒绝了"（NSFW 触发安全审核等）。
///
/// 用逗号数量区分标签列表和短拒绝语，减少包含拒绝词的正常标签列表被误判。
pub(crate) fn looks_like_refusal(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.matches(',').count() >= 3 || trimmed.matches('，').count() >= 3 {
        return false;
    }
    let lower = trimmed.to_lowercase();
    REFUSAL_MARKERS.iter().any(|m| lower.contains(m))
}

/// 工具自己产出的副本目录：失败/警告文件会被复制到这里备查。
/// 它们常常就落在数据集根目录内，递归扫描时必须跳过，否则副本会被当成新图反复处理。
/// 兼容 `Fail`、`Warn`、`_errors` 和 `_warnings` 目录。
pub const ARTIFACT_DIR_NAMES: [&str; 4] = ["Fail", "Warn", "_errors", "_warnings"];

pub(crate) const TAG_SIDECAR_EXTS: &[&str] = &["txt", "json", "caption"];

/// 出错文件的归集目录名（全应用统一）
pub const FAIL_DIR_NAME: &str = "Fail";
/// 有警告文件的归集目录名（全应用统一）
pub const WARN_DIR_NAME: &str = "Warn";

fn is_artifact_dir_name(name: &str) -> bool {
    ARTIFACT_DIR_NAMES
        .iter()
        .any(|d| name.eq_ignore_ascii_case(d))
}

/// 目录项是否是需要剪掉的产物目录。
/// 输入根自身叫这个名字时不算——否则用户直接选中 Fail 目录重跑会得到"没有图片"。
pub(crate) fn is_prunable_artifact_dir(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && entry.file_name().to_str().is_some_and(is_artifact_dir_name)
}

/// 取路径的"目录形态"：各页的输入都可以是单张图片，
/// 拿它当目录用（create_dir_all、join 子目录）会造出 `图片.png/xxx` 这种非法路径，
/// Windows 上直接报"当文件已存在时，无法创建该文件"。
pub fn dir_of(path: &Path) -> std::path::PathBuf {
    if path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

/// 把出问题的源文件复制到 `<root>/<dir_name>/`，递归模式下保留相对子目录结构。
/// 返回实际复制成功的数量；目录建不出来则返回 Err，单个文件复制失败只是不计数。
pub fn copy_files_into_artifact_dir(
    input_root: &Path,
    root: &Path,
    files: &[std::path::PathBuf],
    dir_name: &str,
    recursive: bool,
) -> Result<u32, String> {
    // root 可能是单张图片的路径，落到它所在的目录
    let target_root = dir_of(root).join(dir_name);
    std::fs::create_dir_all(&target_root)
        .map_err(|e| format!("创建 {} 文件夹失败: {}", dir_name, e))?;

    let mut copied = 0u32;
    for src in files {
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let Ok(dest) = output_path_for_input(input_root, src, &target_root, &name, recursive)
        else {
            continue;
        };
        if std::fs::copy(src, &dest).is_ok() {
            copied += 1;
        }
    }
    Ok(copied)
}

/// 把失败的源文件复制进 `<input_dir>/Fail/`（递归时保留子目录结构），再发一条进度事件：
/// 复制完是 info"已将 N 张失败图片复制到 Fail/ 文件夹"，目录建不出来是 error。
/// 事件的 current/total 都取 `total`。`failed` 为空时什么都不做。
pub(crate) fn report_failed_copies<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    event: &str,
    input_dir: &Path,
    failed: &[PathBuf],
    recursive: bool,
    total: u32,
) {
    if failed.is_empty() {
        return;
    }
    let (status, message) = match copy_files_into_artifact_dir(
        input_dir,
        input_dir,
        failed,
        FAIL_DIR_NAME,
        recursive,
    ) {
        Ok(copied) => (
            "info",
            format!("已将 {} 张失败图片复制到 Fail/ 文件夹", copied),
        ),
        Err(e) => ("error", e),
    };
    ProgressEvent::new(status, message)
        .at(total, total)
        .emit(app, event);
}

/// 生成不与已有文件冲突的复制目标路径（同名时追加 _1/_2 …）
pub(crate) fn unique_copy_destination(dir: &Path, filename: &str) -> PathBuf {
    let name = Path::new(filename);
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut dst = dir.join(filename);
    let mut counter = 1;
    while dst.exists() {
        dst = dir.join(format!("{}_{}{}", stem, counter, ext));
        counter += 1;
    }
    dst
}

/// 按背景色 `bg` 把任意像素格式拍平成 RGB8（整数混合）；已是 RGB8 的原样返回
pub(crate) fn flatten_onto(img: image::DynamicImage, bg: [u8; 3]) -> image::DynamicImage {
    if matches!(img, image::DynamicImage::ImageRgb8(_)) {
        return img;
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut rgb = image::RgbImage::new(w, h);
    for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = src[3] as u32;
        let blend = |c: u8, b: u8| ((c as u32 * a + b as u32 * (255 - a)) / 255) as u8;
        *dst = image::Rgb([
            blend(src[0], bg[0]),
            blend(src[1], bg[1]),
            blend(src[2], bg[2]),
        ]);
    }
    image::DynamicImage::ImageRgb8(rgb)
}

/// 把任意像素格式按白底拍平成 RGB8（JPEG 编码器不接受 RGBA；直接丢 alpha 会让透明区变成脏色）
pub(crate) fn flatten_to_rgb_white(img: image::DynamicImage) -> image::DynamicImage {
    flatten_onto(img, [255, 255, 255])
}

/// 大小写不敏感的路径键。Windows/macOS 默认文件系统大小写不敏感，
/// 路径相等/包含判定用它，防止 `C:\Data` vs `c:\data` 之类的变体绕过防覆盖守卫。
/// Linux 上偏保守：大小写变体被视为同一路径——宁可误跳过也不误覆盖。
pub(crate) fn path_key_ci(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// 复制文件到输出位置；目标与源是同一文件（含大小写变体）时安全跳过。
/// "未修改直接复制"的原地模式里，fs::copy 自拷贝会先截断目标，
/// 把源文件清成 0 字节（macOS/Linux；Windows 报共享冲突错误）。
pub(crate) fn copy_file_safe(src: &Path, dest: &Path) -> Result<(), String> {
    if path_key_ci(src) == path_key_ci(dest) {
        return Ok(());
    }
    std::fs::copy(src, dest)
        .map(|_| ())
        .map_err(|e| format!("复制失败: {}", e))
}

/// 命令级互斥闸：多数批处理命令用全局静态存取消标志/子进程句柄，
/// 并发启动（页面 + 工作流节点）会互相覆盖甚至互杀进程。
/// RAII：guard 存活期间占用，任何退出路径（含 ? 与 panic 展开）自动释放。
pub(crate) struct BusyGuard(&'static std::sync::atomic::AtomicBool);

impl BusyGuard {
    pub(crate) fn acquire(
        flag: &'static std::sync::atomic::AtomicBool,
        what: &str,
    ) -> Result<Self, String> {
        use std::sync::atomic::Ordering;
        // 并行单测会撞闸；互斥不是单测的被测对象，测试构型直接放行
        if cfg!(test) {
            return Ok(BusyGuard(flag));
        }
        if flag
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(format!("已有{}任务正在进行，请先等待完成或取消", what));
        }
        Ok(BusyGuard(flag))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

pub fn collect_image_files_with_recursive(
    input: &Path,
    recursive: bool,
) -> Result<Vec<PathBuf>, String> {
    collect_image_files_with_recursive_excluding(input, recursive, None)
}

pub fn collect_image_files_with_recursive_excluding(
    input: &Path,
    recursive: bool,
    excluded_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    collect_files_matching(input, recursive, excluded_dir, is_supported_image_file)
}

/// 按 `accept` 收集文件。
/// - `input` 是文件：直接返回它，不经过 `accept`；
/// - 是目录：非递归只看第一层；剪掉产物目录（见 `is_prunable_artifact_dir`）；
///   `excluded_dir` 位于输入目录内部时跳过其中的文件，与输入目录相同时不排除；
/// - 都不是：报"输入路径无效"。
///
/// 非递归按文件名排序，递归按完整路径字符串排序。
pub(crate) fn collect_files_matching(
    input: &Path,
    recursive: bool,
    excluded_dir: Option<&Path>,
    accept: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();

    if input.is_file() {
        if !accept(input) {
            return Err(format!("不是支持的图片文件: {}", input.display()));
        }
        files.push(input.to_path_buf());
    } else if input.is_dir() {
        let input_canonical = std::fs::canonicalize(input).ok();
        let excluded = excluded_dir
            .filter(|dir| dir.exists())
            .and_then(|dir| std::fs::canonicalize(dir).ok());
        let should_exclude_output = match (input_canonical.as_ref(), excluded.as_ref()) {
            (Some(input), Some(excluded)) => excluded != input && excluded.starts_with(input),
            _ => false,
        };
        let walker = if recursive {
            walkdir::WalkDir::new(input)
        } else {
            walkdir::WalkDir::new(input).max_depth(1)
        };

        for entry in walker
            .into_iter()
            .filter_entry(|e| !is_prunable_artifact_dir(e))
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if should_exclude_output {
                let excluded = excluded.as_ref().expect("checked by should_exclude_output");
                let normalized = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                if normalized.starts_with(excluded) {
                    continue;
                }
            }
            if accept(p) {
                files.push(p.to_path_buf());
            }
        }
    } else {
        return Err(format!("输入路径无效: {}", input.display()));
    }

    if recursive {
        files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    } else {
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    }
    Ok(files)
}

pub fn relative_dir_for_input(
    input_root: &Path,
    file_path: &Path,
    recursive: bool,
) -> Option<std::path::PathBuf> {
    if !recursive || !input_root.is_dir() {
        return None;
    }

    let relative = file_path.strip_prefix(input_root).ok()?;
    let parent = relative.parent()?;
    if parent.as_os_str().is_empty() {
        None
    } else {
        Some(parent.to_path_buf())
    }
}

pub fn output_dir_for_input(
    input_root: &Path,
    file_path: &Path,
    output_dir: &Path,
    recursive: bool,
) -> Result<std::path::PathBuf, String> {
    let target_dir = match relative_dir_for_input(input_root, file_path, recursive) {
        Some(relative) => output_dir.join(relative),
        None => output_dir.to_path_buf(),
    };
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("无法创建输出目录 {}: {}", target_dir.display(), e))?;
    Ok(target_dir)
}

pub fn output_path_for_input(
    input_root: &Path,
    file_path: &Path,
    output_dir: &Path,
    output_name: &str,
    recursive: bool,
) -> Result<std::path::PathBuf, String> {
    Ok(output_dir_for_input(input_root, file_path, output_dir, recursive)?.join(output_name))
}

/// 与输入同名的输出路径（递归时保留相对子目录，并建好所在目录）；取不到文件名时报"无效的文件名"
pub(crate) fn same_name_output(
    input_root: &Path,
    file: &Path,
    out_dir: &Path,
    recursive: bool,
) -> Result<PathBuf, String> {
    let name = file.file_name().ok_or("无效的文件名")?;
    output_path_for_input(
        input_root,
        file,
        out_dir,
        &name.to_string_lossy(),
        recursive,
    )
}

/// 文件名转字符串（非 UTF-8 部分有损替换）；没有文件名时为空串
pub(crate) fn file_name_lossy(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 概念文件夹扫描结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptFolderInfo {
    pub name: String,
    pub image_count: u32,
    pub repeats: u32,
    pub folder_name: String,
}

/// 扫描训练集目录下的概念文件夹
/// 支持 LoRA 常见命名格式: `{repeats}_{concept_name}` (如 `10_character`)
#[tauri::command]
pub fn scan_concept_folders(dir: String) -> Result<Vec<ConceptFolderInfo>, String> {
    let path = Path::new(&dir);
    if !path.is_dir() {
        return Err(format!("目录不存在: {}", dir));
    }

    let mut results = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(path)
        .map_err(|e| format!("读取目录失败: {}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let folder_name = entry.file_name().to_string_lossy().to_string();
        // 跳过隐藏文件夹和特殊文件夹
        if folder_name.starts_with('.') || folder_name.starts_with('_') {
            continue;
        }

        // 解析 repeats_name 格式
        let (repeats, concept_name) = if let Some(pos) = folder_name.find('_') {
            let prefix = &folder_name[..pos];
            if let Ok(r) = prefix.parse::<u32>() {
                (r, folder_name[pos + 1..].to_string())
            } else {
                (1, folder_name.clone())
            }
        } else {
            (1, folder_name.clone())
        };

        // 计算图片数量
        let image_count = std::fs::read_dir(entry.path())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file() && is_supported_image_file(&e.path()))
                    .count() as u32
            })
            .unwrap_or(0);

        results.push(ConceptFolderInfo {
            name: concept_name,
            image_count,
            repeats,
            folder_name,
        });
    }

    Ok(results)
}

/// 应用配平结果：将概念文件夹重命名为新的 repeats_name 格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRepeatsItem {
    pub folder_name: String,
    pub new_repeats: u32,
    pub concept_name: String,
}

#[tauri::command]
pub fn apply_concept_repeats(
    dir: String,
    items: Vec<ApplyRepeatsItem>,
) -> Result<Vec<String>, String> {
    let base = Path::new(&dir);
    if !base.is_dir() {
        return Err(format!("目录不存在: {}", dir));
    }

    let mut renamed = Vec::new();
    for item in &items {
        let old_path = base.join(&item.folder_name);
        if !old_path.exists() {
            continue;
        }
        let new_name = format!("{}_{}", item.new_repeats, item.concept_name);
        if new_name == item.folder_name {
            continue; // 没有变化
        }
        let new_path = base.join(&new_name);
        if new_path.exists() {
            return Err(format!("目标文件夹已存在: {}", new_name));
        }
        std::fs::rename(&old_path, &new_path)
            .map_err(|e| format!("重命名失败 {} → {}: {}", item.folder_name, new_name, e))?;
        renamed.push(format!("{} → {}", item.folder_name, new_name));
    }

    Ok(renamed)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessResult {
    pub success_count: u32,
    pub fail_count: u32,
    pub total: u32,
    pub errors: Vec<String>,
}

/// 系统性能指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    pub cpu_usage: f32,
    pub cpu_name: String,
    pub cpu_cores: usize,
    pub memory_used: u64,
    pub memory_total: u64,
    pub memory_percent: f32,
    pub gpu_name: String,
    pub gpu_usage: f32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub vram_percent: f32,
}

/// 获取系统性能指标
#[tauri::command]
pub async fn get_system_stats() -> Result<SystemStats, String> {
    use sysinfo::System;
    static SYSTEM: std::sync::OnceLock<std::sync::Mutex<System>> = std::sync::OnceLock::new();

    tokio::task::spawn_blocking(|| {
        let mut sys = SYSTEM
            .get_or_init(|| std::sync::Mutex::new(System::new()))
            .lock()
            .map_err(|e| format!("获取系统信息失败: {}", e))?;
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let cpu_usage = sys.global_cpu_usage();
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown".into());
        let cpu_cores = sys.cpus().len();

        let memory_total = sys.total_memory();
        let memory_used = sys.used_memory();
        drop(sys);
        let memory_percent = if memory_total > 0 {
            (memory_used as f64 / memory_total as f64 * 100.0) as f32
        } else {
            0.0
        };

        // GPU 检测
        let (gpu_name, gpu_usage, vram_used, vram_total, vram_percent) = detect_gpu(memory_total);

        Ok(SystemStats {
            cpu_usage,
            cpu_name,
            cpu_cores,
            memory_used,
            memory_total,
            memory_percent,
            gpu_name,
            gpu_usage,
            vram_used,
            vram_total,
            vram_percent,
        })
    })
    .await
    .map_err(|e| format!("获取系统信息失败: {}", e))?
}

/// 检测 GPU 信息，返回 (名称, 使用率%, 显存已用, 显存总量, 显存%)
fn detect_gpu(_memory_total: u64) -> (String, f32, u64, u64, f32) {
    // 1. 尝试 nvidia-smi（Windows + Linux 上有 NVIDIA 显卡时）
    if let Some(result) = detect_nvidia_gpu() {
        return result;
    }

    // 2. macOS: 检测 Apple Silicon GPU（通过 system_profiler）
    #[cfg(target_os = "macos")]
    if let Some(result) = detect_apple_gpu(_memory_total) {
        return result;
    }

    // 3. 未检测到
    (String::new(), -1.0, 0, 0, -1.0)
}

/// 通过 nvidia-smi 检测 NVIDIA 显卡（Windows 上隐藏控制台窗口）
fn detect_nvidia_gpu() -> Option<(String, f32, u64, u64, f32)> {
    let mut cmd = python_proc::hidden_command("nvidia-smi");
    cmd.args([
        "--query-gpu=name,utilization.gpu,memory.used,memory.total",
        "--format=csv,noheader,nounits",
    ]);

    let output = cmd.output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?.trim();
    let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();

    if parts.len() < 4 {
        return None;
    }

    let name = parts[0].to_string();
    let usage: f32 = parts[1].parse().unwrap_or(0.0);
    let vram_used_mb: f64 = parts[2].parse().unwrap_or(0.0);
    let vram_total_mb: f64 = parts[3].parse().unwrap_or(0.0);

    let vram_used = (vram_used_mb * 1024.0 * 1024.0) as u64;
    let vram_total = (vram_total_mb * 1024.0 * 1024.0) as u64;
    let vram_percent = if vram_total > 0 {
        (vram_used as f64 / vram_total as f64 * 100.0) as f32
    } else {
        0.0
    };

    Some((name, usage, vram_used, vram_total, vram_percent))
}

/// macOS: 缓存 GPU 名称（system_profiler 调用耗时 1-3 秒，只检测一次）
#[cfg(target_os = "macos")]
static CACHED_GPU_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// macOS: 检测 Apple Silicon GPU
#[cfg(target_os = "macos")]
fn detect_apple_gpu(memory_total: u64) -> Option<(String, f32, u64, u64, f32)> {
    let gpu_name = CACHED_GPU_NAME
        .get_or_init(|| {
            // 获取 GPU 芯片名称
            let chip = std::process::Command::new("sysctl")
                .args(["-n", "machdep.cpu.brand_string"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            // 从 system_profiler 获取 GPU 名称
            let sp_output = std::process::Command::new("system_profiler")
                .args(["SPDisplaysDataType", "-json"])
                .output()
                .ok();

            if let Some(output) = sp_output {
                let sp_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&sp_str) {
                    if let Some(name) = json["SPDisplaysDataType"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|gpu| gpu["sppci_model"].as_str())
                    {
                        return name.to_string();
                    }
                }
            }

            if chip.contains("Apple") {
                format!(
                    "{} GPU",
                    chip.split_whitespace()
                        .take(3)
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            } else {
                "Apple GPU".into()
            }
        })
        .clone();

    let output = std::process::Command::new("ioreg")
        .args(["-r", "-l", "-c", "IOAccelerator"])
        .output()
        .ok();
    let stdout = output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout))
        .unwrap_or_default();
    let gpu_usage = extract_ioreg_perf_value(&stdout, "Device Utilization %")
        .map(|v| v as f32)
        .unwrap_or(-1.0);

    // Apple Silicon 统一内存 — GPU 共享系统 RAM
    let vram_total = memory_total;
    let vram_used = extract_ioreg_perf_value(&stdout, "In use system memory")
        .map(|v| v as u64)
        .unwrap_or(0);

    let vram_percent = if vram_total > 0 {
        (vram_used as f64 / vram_total as f64 * 100.0) as f32
    } else {
        -1.0
    };

    Some((gpu_name, gpu_usage, vram_used, vram_total, vram_percent))
}

/// macOS: 从 ioreg PerformanceStatistics 字典中提取指定 key 的数值
#[cfg(target_os = "macos")]
fn extract_ioreg_perf_value(stdout: &str, key: &str) -> Option<f64> {
    for line in stdout.lines() {
        if !line.contains("PerformanceStatistics") {
            continue;
        }
        // 格式: ..."Device Utilization %"=17,...
        // 搜索 "key"= 后面的数字
        let search = format!("\"{}\"=", key);
        if let Some(pos) = line.find(&search) {
            let after = &line[pos + search.len()..];
            // 取到逗号或 } 之前的数字
            let num_str: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(val) = num_str.parse::<f64>() {
                return Some(val);
            }
        }
    }
    None
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn system_stats_parse_one_ioreg_snapshot() {
    let snapshot = r#"| "PerformanceStatistics" = {"Device Utilization %"=17.5,"In use system memory"=1048576}"#;
    assert_eq!(
        extract_ioreg_perf_value(snapshot, "Device Utilization %"),
        Some(17.5)
    );
    assert_eq!(
        extract_ioreg_perf_value(snapshot, "In use system memory"),
        Some(1048576.0)
    );
    assert_eq!(extract_ioreg_perf_value(snapshot, "missing"), None);
}

/// 版本更新检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub latest_version: String,
    pub release_url: String,
}

/// 检查 GitHub 最新 Release 版本
#[tauri::command]
pub async fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let current = env!("CARGO_PKG_VERSION");
    let url = "https://api.github.com/repos/YPuddin-Neko/PurinBox/releases/latest";

    let client = proxy_config::build_http_client()
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        // 仓库还没有任何 Release
        return Ok(UpdateCheckResult {
            has_update: false,
            latest_version: current.to_string(),
            release_url: String::new(),
        });
    }
    if !status.is_success() {
        return Err(format!("GitHub API 返回 {}", status));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let tag = json["tag_name"].as_str().unwrap_or("v0.0.0");
    let latest = tag.trim_start_matches('v');
    let html_url = json["html_url"].as_str().unwrap_or("").to_string();

    let has_update = version_compare(latest, current);

    Ok(UpdateCheckResult {
        has_update,
        latest_version: latest.to_string(),
        release_url: html_url,
    })
}

/// 简单版本号比较: 如果 latest > current 返回 true
fn version_compare(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv > cv {
            return true;
        }
        if lv < cv {
            return false;
        }
    }
    false
}

/// 强制终止子进程树（按 PID）。
///
/// 普通的 `Child::kill()` 只终止直接子进程，Python 派生的工作进程
/// （onnxruntime 线程池、torch worker 等）会存活成为孤儿进程，
/// 表现为「强制结束不起作用」。这里按进程树整体终止。
pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut cmd = python_proc::hidden_command("taskkill");
        cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
        let _ = cmd.output();
    }
    #[cfg(unix)]
    {
        // 先杀进程组（负 PID 表示进程组），失败再退化为单进程
        let killed_group = std::process::Command::new("kill")
            .args(["-9", &format!("-{}", pid)])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !killed_group {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .output();
        }
    }
}

/// 强制终止 `Child` 句柄对应的进程树并回收句柄。
pub fn kill_child_tree(slot: &std::sync::Mutex<Option<std::process::Child>>) {
    if let Ok(mut guard) = slot.lock() {
        if let Some(mut child) = guard.take() {
            kill_process_tree(child.id());
            // taskkill/kill 之后仍需 kill+wait 回收句柄
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod artifact_dir_tests {
    use super::*;

    fn fixture(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("purinbox_artifact_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for rel in ["", "sub", "Fail", "Warn", "_errors", "_warnings"] {
            std::fs::create_dir_all(root.join(rel)).unwrap();
        }
        // 兼容历史产物目录名。
        for rel in [
            "a.png",
            "sub/d.png",
            "Fail/b.png",
            "Warn/f.png",
            "_errors/c.png",
            "_warnings/e.png",
        ] {
            std::fs::write(root.join(rel), b"x").unwrap();
        }
        root
    }

    fn names(files: &[std::path::PathBuf]) -> Vec<String> {
        let mut v: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn recursive_scan_prunes_artifact_dirs() {
        let root = fixture("prune");
        let files = collect_image_files_with_recursive(&root, true).unwrap();
        // 产物目录中的副本不属于数据集图片。
        assert_eq!(names(&files), vec!["a.png", "d.png"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn artifact_dir_scanned_when_it_is_the_input_root() {
        let root = fixture("as_root");
        // 输入根本身是产物目录时仍允许重跑其中的图片。
        let files = collect_image_files_with_recursive(&root.join("Fail"), true).unwrap();
        assert_eq!(names(&files), vec!["b.png"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 输入选的是单张图片时，产物目录要落在它所在的文件夹，
    /// 而不是拼成 `图片.png/Fail`（Windows 上报"当文件已存在时，无法创建该文件"）
    #[test]
    fn artifact_dir_falls_back_to_parent_for_single_image() {
        let root = fixture("single");
        let single = root.join("a.png");
        assert_eq!(dir_of(&single), root);
        // root 传成图片路径也不该炸
        let copied = copy_files_into_artifact_dir(
            &single,
            &single,
            std::slice::from_ref(&single),
            "Fail",
            false,
        )
        .unwrap();
        assert_eq!(copied, 1);
        assert!(root.join("Fail/a.png").exists());
        assert!(!root.join("a.png/Fail").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_into_artifact_dir_keeps_subdirs() {
        let root = fixture("copy");
        let failed = vec![root.join("a.png"), root.join("sub/d.png")];
        let copied = copy_files_into_artifact_dir(&root, &root, &failed, "Fail", true).unwrap();
        assert_eq!(copied, 2);
        assert!(root.join("Fail/a.png").exists());
        assert!(root.join("Fail/sub/d.png").exists());
        // 复制进去的副本下一轮扫描依然被剪掉，不会自我增殖
        let files = collect_image_files_with_recursive(&root, true).unwrap();
        assert_eq!(names(&files), vec!["a.png", "d.png"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;
    use image::DynamicImage;
    use serde_json::json;

    const EVENT: &str = "helper-test-progress";

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("purinbox_helpers_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// 改用构造器之前，9 处 Python log 转发块各自的写法
    fn legacy_python_log(msg: &serde_json::Value, current: u32, total: u32) -> ProgressEvent {
        let text = msg
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let i18n_key = msg
            .get("i18n_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let i18n_params = msg.get("i18n_params").cloned();
        ProgressEvent {
            current,
            total,
            filename: String::new(),
            status: "info".to_string(),
            message: text,
            i18n_key,
            i18n_params,
        }
    }

    #[test]
    fn python_log_matches_legacy_forwarding() {
        let cases = [
            json!({"type": "log", "message": "加载模型", "i18n_key": "tagger.loading", "i18n_params": {"n": 3}}),
            json!({"type": "log", "message": "plain"}),
            json!({"type": "log", "i18n_params": null}),
            json!({"type": "log", "message": 42, "i18n_key": 7}),
            json!("not an object"),
        ];
        for msg in &cases {
            for (current, total) in [(0, 0), (0, 12), (5, 12)] {
                assert_eq!(
                    serde_json::to_string(&ProgressEvent::python_log(msg, current, total)).unwrap(),
                    serde_json::to_string(&legacy_python_log(msg, current, total)).unwrap(),
                    "{}",
                    msg
                );
            }
        }
        // 写成 null 的参数仍然原样带上
        assert_eq!(
            serde_json::to_value(ProgressEvent::python_log(&cases[2], 0, 0)).unwrap(),
            json!({"current": 0, "total": 0, "filename": "", "status": "info", "message": "", "i18n_params": null})
        );
        // 逐图日志带文件名的两处
        let mut legacy = legacy_python_log(&cases[0], 3, 9);
        legacy.filename = "a.png".to_string();
        assert_eq!(
            serde_json::to_string(&ProgressEvent::python_log(&cases[0], 3, 9).file("a.png"))
                .unwrap(),
            serde_json::to_string(&legacy).unwrap()
        );
    }

    #[test]
    fn builder_serializes_like_struct_literal() {
        let literal = ProgressEvent {
            current: 1,
            total: 3,
            filename: "a.png".to_string(),
            status: "success".to_string(),
            message: "[水平翻转] a.png ✓".to_string(),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(
                &ProgressEvent::new("success", "[水平翻转] a.png ✓")
                    .at(1, 3)
                    .file("a.png")
            )
            .unwrap(),
            serde_json::to_string(&literal).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&ProgressEvent::new("done", "完成")).unwrap(),
            r#"{"current":0,"total":0,"filename":"","status":"done","message":"完成"}"#
        );
    }

    #[test]
    fn emit_reaches_listeners() {
        let app = tauri::test::mock_app();
        let log = batch::capture_events(app.handle(), EVENT);
        ProgressEvent::new("info", "hi")
            .at(1, 2)
            .emit(app.handle(), EVENT);
        // llm_tagger 持有的是 Arc<AppHandle>，传引用即可自动解引用
        let shared = std::sync::Arc::new(app.handle().clone());
        ProgressEvent::new("done", "bye").emit(&shared, EVENT);
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                json!({"current": 1, "total": 2, "filename": "", "status": "info", "message": "hi"}),
                json!({"current": 0, "total": 0, "filename": "", "status": "done", "message": "bye"}),
            ]
        );
    }

    #[test]
    fn report_failed_copies_copies_then_reports() {
        let root = temp_root("report");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        for rel in ["a.png", "sub/b.png"] {
            std::fs::write(root.join(rel), b"x").unwrap();
        }
        let app = tauri::test::mock_app();
        let log = batch::capture_events(app.handle(), EVENT);

        let failed = vec![root.join("a.png"), root.join("sub/b.png")];
        report_failed_copies(app.handle(), EVENT, &root, &failed, true, 5);
        report_failed_copies(app.handle(), EVENT, &root, &[], true, 5);

        assert_eq!(
            *log.lock().unwrap(),
            vec![json!({
                "current": 5, "total": 5, "filename": "", "status": "info",
                "message": "已将 2 张失败图片复制到 Fail/ 文件夹",
            })]
        );
        assert!(root.join("Fail/a.png").exists());
        assert!(root.join("Fail/sub/b.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn report_failed_copies_reports_dir_error() {
        let root = temp_root("report_err");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.png"), b"x").unwrap();
        // 同名普通文件占住了 Fail 目录的位置
        std::fs::write(root.join("Fail"), b"x").unwrap();
        let app = tauri::test::mock_app();
        let log = batch::capture_events(app.handle(), EVENT);

        report_failed_copies(app.handle(), EVENT, &root, &[root.join("a.png")], false, 1);

        let events = log.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["status"], "error");
        assert_eq!(
            (&events[0]["current"], &events[0]["total"]),
            (&json!(1), &json!(1))
        );
        let message = events[0]["message"].as_str().unwrap();
        assert!(message.starts_with("创建 Fail 文件夹失败: "), "{}", message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unique_copy_destination_appends_counter() {
        let dir = temp_root("unique");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_copy_destination(&dir, "a.png"), dir.join("a.png"));
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        assert_eq!(unique_copy_destination(&dir, "a.png"), dir.join("a_1.png"));
        std::fs::write(dir.join("a_1.png"), b"x").unwrap();
        assert_eq!(unique_copy_destination(&dir, "a.png"), dir.join("a_2.png"));
        std::fs::write(dir.join("noext"), b"x").unwrap();
        assert_eq!(unique_copy_destination(&dir, "noext"), dir.join("noext_1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 改成调用 flatten_onto 之前的白底拍平
    fn legacy_flatten_white(img: DynamicImage) -> DynamicImage {
        if matches!(img, DynamicImage::ImageRgb8(_)) {
            return img;
        }
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let mut rgb = image::RgbImage::new(w, h);
        for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
            let a = src[3] as u32;
            let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
            *dst = image::Rgb([blend(src[0]), blend(src[1]), blend(src[2])]);
        }
        DynamicImage::ImageRgb8(rgb)
    }

    #[test]
    fn flatten_white_is_pixel_identical() {
        // x 遍历颜色值、y 遍历 alpha，覆盖全部组合
        let rgba = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(256, 256, |x, y| {
            image::Rgba([x as u8, (255 - x) as u8, (x * 7 % 256) as u8, y as u8])
        }));
        let luma_alpha =
            DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_fn(256, 256, |x, y| {
                image::LumaA([x as u8, y as u8])
            }));
        let rgba16 = DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(64, 64, |x, y| {
            image::Rgba([(x * 1031) as u16, 40_000, 7, (y * 1039) as u16])
        }));
        let rgb = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([1, 2, 3])));
        for img in [rgba, luma_alpha, rgba16, rgb] {
            assert_eq!(flatten_to_rgb_white(img.clone()), legacy_flatten_white(img));
        }
    }

    #[test]
    fn flatten_onto_uses_background() {
        let img = DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 0, 10, 20, 30, 255]).unwrap(),
        );
        assert_eq!(
            flatten_onto(img, [0, 0, 0]).to_rgb8().into_raw(),
            vec![0, 0, 0, 10, 20, 30]
        );
        let rgb = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([9, 9, 9])));
        assert_eq!(flatten_onto(rgb.clone(), [0, 0, 0]), rgb);
    }

    /// 改成薄包装之前的 collect_image_files
    fn legacy_collect_image_files(input: &Path) -> Result<Vec<PathBuf>, String> {
        let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
        let mut files = Vec::new();
        if input.is_file() {
            files.push(input.to_path_buf());
        } else if input.is_dir() {
            for entry in walkdir::WalkDir::new(input)
                .max_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if p.is_file() {
                    if let Some(ext) = p.extension() {
                        let ext_lower = ext.to_string_lossy().to_lowercase();
                        if supported_exts.contains(&ext_lower.as_str()) {
                            files.push(p.to_path_buf());
                        }
                    }
                }
            }
        } else {
            return Err(format!("输入路径无效: {}", input.display()));
        }
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        Ok(files)
    }

    fn collect_fixture(tag: &str) -> PathBuf {
        let root = temp_root(tag);
        for dir in ["sub", "out", "Fail"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for rel in [
            "b.PNG",
            "a.psd",
            "c.txt",
            "z.webp",
            "sub/d.jpg",
            "sub/e.psd",
            "out/f.png",
            "Fail/g.png",
        ] {
            std::fs::write(root.join(rel), b"x").unwrap();
        }
        root
    }

    fn rel_names(root: &Path, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|f| {
                f.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn collect_image_files_matches_legacy() {
        let root = collect_fixture("legacy");
        for input in [
            root.clone(),
            root.join("b.PNG"),
            root.join("Fail"),
            root.join("missing"),
        ] {
            assert_eq!(
                collect_image_files_with_recursive(&input, false),
                legacy_collect_image_files(&input),
                "{}",
                input.display()
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_files_matching_with_custom_predicate() {
        let root = collect_fixture("predicate");
        let out = root.join("out");
        let image_or_psd = |p: &Path| is_supported_image_file(p) || has_extension(p, &["psd"]);

        let flat = collect_files_matching(&root, false, Some(&out), image_or_psd).unwrap();
        assert_eq!(rel_names(&root, &flat), vec!["a.psd", "b.PNG", "z.webp"]);

        let deep = collect_files_matching(&root, true, Some(&out), image_or_psd).unwrap();
        assert_eq!(
            rel_names(&root, &deep),
            vec!["a.psd", "b.PNG", "sub/d.jpg", "sub/e.psd", "z.webp"]
        );

        let images = collect_image_files_with_recursive_excluding(&root, true, Some(&out)).unwrap();
        assert_eq!(
            rel_names(&root, &images),
            vec!["b.PNG", "sub/d.jpg", "z.webp"]
        );

        assert!(
            collect_files_matching(&root.join("c.txt"), true, None, is_supported_image_file)
                .unwrap_err()
                .contains("不是支持的图片文件")
        );
        assert_eq!(
            collect_files_matching(&root.join("a.psd"), false, None, image_or_psd).unwrap(),
            vec![root.join("a.psd")]
        );
        assert_eq!(
            collect_files_matching(&root.join("missing"), false, None, |_| true).unwrap_err(),
            format!("输入路径无效: {}", root.join("missing").display())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn name_helpers() {
        assert_eq!(file_name_lossy(Path::new("/a/b.png")), "b.png");
        assert_eq!(file_name_lossy(Path::new("/")), "");
        assert!(has_extension(Path::new("x.JPeG"), IMAGE_EXTS));
        assert!(!has_extension(Path::new("x.psd"), IMAGE_EXTS));
        assert!(!has_extension(Path::new("png"), IMAGE_EXTS));

        let root = temp_root("same_name");
        let (input, out) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(input.join("sub")).unwrap();
        let file = input.join("sub").join("x.png");
        assert_eq!(
            same_name_output(&input, &file, &out, true).unwrap(),
            out.join("sub").join("x.png")
        );
        assert!(out.join("sub").is_dir());
        assert_eq!(
            same_name_output(&input, &file, &out, false).unwrap(),
            out.join("x.png")
        );
        assert_eq!(
            same_name_output(&input, Path::new("/"), &out, false).unwrap_err(),
            "无效的文件名"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
