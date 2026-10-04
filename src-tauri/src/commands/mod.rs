use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
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
pub(crate) mod python_task;
pub mod resolution_analyze;
pub mod resolution_filter;
pub mod sd_metadata;
pub mod tag_db;
pub mod tag_manager;
pub mod tag_refine;
pub mod tag_sort;
pub(crate) mod tag_text;
pub mod tagger;
pub mod thumbnail;
pub mod translator;
pub mod upscale;
pub mod workflow;

#[cfg(test)]
pub(crate) mod test_support;
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

/// 完成下载：校验实际字节数（若服务器提供了 content-length），通过后把 .part 改名为最终文件。
/// 改名直接覆盖已有的最终文件（Windows 上 std 用 `MoveFileExW` + `MOVEFILE_REPLACE_EXISTING`），
/// 不先删旧文件——改名失败时旧文件原样保留。任何失败路径都会清理 .part 残件。
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

/// 运行 ID 计数器：全局递增、跨事件通道共用，前端按通道比较大小
static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);
/// 事件名 → 该通道当前一轮任务的运行 ID
static CURRENT_RUNS: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// 在 `event` 通道上开始新一轮任务，返回这一轮的运行 ID；之后该通道上的
/// `ProgressEvent::emit` 都自动带上它（`for_run` 显式指定的除外）。
///
/// 前端在新一轮开始时记下该通道见过的最大 run_id，丢弃不大于它的事件——上一轮迟到的事件
/// （典型是晚到的 done）因此不会串进这一轮。任务命令在拿到互斥锁之后、发第一条事件之前调用一次；
/// `FileBatch::run` 会自己调用。
pub(crate) fn begin_run(event: &str) -> u64 {
    let run_id = NEXT_RUN_ID.fetch_add(1, Ordering::SeqCst);
    CURRENT_RUNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(event.to_string(), run_id);
    run_id
}

fn current_run(event: &str) -> Option<u64> {
    CURRENT_RUNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(event)
        .copied()
}

fn is_false(value: &bool) -> bool {
    !*value
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
    /// 所属那一轮任务的运行 ID（见 `begin_run`）；该通道从没开始过任何一轮时不带
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<u64>,
    /// 用户取消的那一轮的终态 done 为 true；false 时不序列化
    #[serde(skip_serializing_if = "is_false")]
    pub cancelled: bool,
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

    /// 标记为用户取消的那一轮的终态（`cancelled: true`）
    pub fn cancelled(mut self) -> Self {
        self.cancelled = true;
        self
    }

    /// 显式指定运行 ID，`emit` 不再按通道取当前一轮的 ID
    pub fn for_run(mut self, run_id: u64) -> Self {
        self.run_id = Some(run_id);
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
            ..Default::default()
        }
    }

    /// 发给前端，忽略发送失败（窗口已关闭等）。没有 `for_run` 时带上 `event` 通道当前一轮的运行 ID
    pub fn emit<R: tauri::Runtime>(mut self, app: &tauri::AppHandle<R>, event: &str) {
        if self.run_id.is_none() {
            self.run_id = current_run(event);
        }
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

/// 工具自己产出的副本目录：失败/警告文件会被复制到这里备查。
/// 它们常常就落在数据集根目录内，递归扫描时必须跳过，否则副本会被当成新图反复处理。
/// 兼容 `Fail`、`Warn`、`_errors` 和 `_warnings` 目录。
const ARTIFACT_DIR_NAMES: [&str; 4] = ["Fail", "Warn", "_errors", "_warnings"];

pub(crate) const TAG_SIDECAR_EXTS: &[&str] = &["txt", "json", "caption"];

/// 图片的同名标签文件 `(扩展名, 路径)`，扩展名按 `TAG_SIDECAR_EXTS` 的顺序。只拼路径，不检查是否存在
pub(crate) fn tag_sidecars(path: &Path) -> impl Iterator<Item = (&'static str, PathBuf)> + '_ {
    TAG_SIDECAR_EXTS
        .iter()
        .map(move |&ext| (ext, path.with_extension(ext)))
}

/// 出错文件的归集目录名（全应用统一）
const FAIL_DIR_NAME: &str = "Fail";
/// 有警告文件的归集目录名（全应用统一）
const WARN_DIR_NAME: &str = "Warn";

fn is_artifact_dir_name(name: &str) -> bool {
    ARTIFACT_DIR_NAMES
        .iter()
        .any(|d| name.eq_ignore_ascii_case(d))
}

/// 目录项是否是需要剪掉的产物目录。
/// 输入根自身叫这个名字时不算——否则用户直接选中 Fail 目录重跑会得到"没有图片"。
fn is_prunable_artifact_dir(entry: &walkdir::DirEntry) -> bool {
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
/// 源文件本来就在目标位置（拿上次的 Fail/ 当输入重跑）时不复制，照样计数。
/// 返回归集成功的数量；目录建不出来则返回 Err，单个文件复制失败只是不计数。
fn copy_files_into_artifact_dir(
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
        if copy_file_safe(src, &dest).is_ok() {
            copied += 1;
        }
    }
    Ok(copied)
}

/// 出问题的源文件归集到输出根目录备查：失败的复制进 `<output_root>/Fail/`，有警告的复制进
/// `<output_root>/Warn/`，递归处理时保留相对 `input_root` 的子目录（见 `copy_files_into_artifact_dir`）。
///
/// 用法：`ProblemArchive::new(input, output, recursive).report(&app, EVENT, &failed, &warned)`；
/// `FileBatch::archive_failures` 与 `llm_batch::BatchOutcome::finish` 内部也走它。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProblemArchive<'a> {
    input_root: &'a Path,
    output_root: &'a Path,
    recursive: bool,
    skip_warnings: bool,
}

impl<'a> ProblemArchive<'a> {
    /// `output_root` 可以是单张图片的路径，此时落到它所在的目录
    pub(crate) fn new(input_root: &'a Path, output_root: &'a Path, recursive: bool) -> Self {
        Self {
            input_root,
            output_root,
            recursive,
            skip_warnings: false,
        }
    }

    /// `skip` 为 true 时不复制警告文件，改发一条说明。
    /// 标签细化就地输出（输出目录就是输入目录）时用：失败文件照常归集
    pub(crate) fn skip_warnings(mut self, skip: bool) -> Self {
        self.skip_warnings = skip;
        self
    }

    /// 复制并返回要发的事件，每类非空时一条（current/total 为 0，前端按日志处理，不动进度条）：
    /// - 复制完：info「已将 {n} 个失败文件复制到 Fail/ 文件夹」（警告文件对应 Warn/）；
    /// - 跳过警告文件：info「输出与输入目录相同，已跳过 Warn/ 复制」；
    /// - 目录建不出来：error，文案为 `copy_files_into_artifact_dir` 的错误。
    ///
    /// 要先给事件标上运行 ID 再发时用它，否则用 `report`。
    pub(crate) fn archive(&self, failed: &[PathBuf], warned: &[PathBuf]) -> Vec<ProgressEvent> {
        let mut events = Vec::new();
        for (files, dir_name, label, skip) in [
            (failed, FAIL_DIR_NAME, "失败", false),
            (warned, WARN_DIR_NAME, "警告", self.skip_warnings),
        ] {
            if files.is_empty() {
                continue;
            }
            if skip {
                events.push(ProgressEvent::new(
                    "info",
                    format!("输出与输入目录相同，已跳过 {}/ 复制", dir_name),
                ));
                continue;
            }
            events.push(
                match copy_files_into_artifact_dir(
                    self.input_root,
                    self.output_root,
                    files,
                    dir_name,
                    self.recursive,
                ) {
                    Ok(copied) => ProgressEvent::new(
                        "info",
                        format!("已将 {} 个{}文件复制到 {}/ 文件夹", copied, label, dir_name),
                    ),
                    Err(e) => ProgressEvent::new("error", e),
                },
            );
        }
        events
    }

    /// `archive` 之后把事件发到 `event` 通道。`failed` 和 `warned` 都为空时什么都不做
    pub(crate) fn report<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        event: &str,
        failed: &[PathBuf],
        warned: &[PathBuf],
    ) {
        for progress in self.archive(failed, warned) {
            progress.emit(app, event);
        }
    }
}

/// 文件名被占用时怎样换名
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameSuffix {
    /// 先试原名，再试 `a_1.png`、`a_2.png` …（复制进目标目录时用）
    Counter,
    /// 先试 `a_rename.png`，再试 `a_rename_1.png`、`a_rename_2.png` …（给同名文件让位时用）
    Rename,
}

/// 在 `dir` 下为 `filename` 找第一个空闲的路径：候选按 `suffix` 依次生成，`taken` 判断候选是否被占用
/// （一般传 `Path::exists`；改名还要带走标签文件时把标签文件的目标也算进去）
pub(crate) fn unique_destination(
    dir: &Path,
    filename: &str,
    suffix: NameSuffix,
    taken: impl Fn(&Path) -> bool,
) -> PathBuf {
    let name = Path::new(filename);
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let (first, numbered) = match suffix {
        NameSuffix::Counter => (filename.to_string(), format!("{}_", stem)),
        NameSuffix::Rename => (
            format!("{}_rename{}", stem, ext),
            format!("{}_rename_", stem),
        ),
    };
    std::iter::once(first)
        .chain((1u64..).map(|n| format!("{}{}{}", numbered, n, ext)))
        .map(|candidate| dir.join(candidate))
        .find(|candidate| !taken(candidate))
        .expect("候选文件名无穷多，总能找到空闲的")
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

/// 两个路径是否指向同一个文件或目录：只差大小写（`path_key_ci` 的保守口径），
/// 或规范化（解析 `.`、`..` 和符号链接）后相同。不存在的路径只按前者比较
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    path_key_ci(a) == path_key_ci(b)
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// 复制文件到输出位置；目标与源是同一文件（见 `same_path`）时安全跳过。
/// "未修改直接复制"的原地模式里，fs::copy 自拷贝会先截断目标，
/// 把源文件清成 0 字节（macOS/Linux；Windows 报共享冲突错误）。
pub(crate) fn copy_file_safe(src: &Path, dest: &Path) -> Result<(), String> {
    if same_path(src, dest) {
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
/// - `input` 是文件：原样返回它，不经过 `accept`——用户单选的文件即使扩展名不在列表里（如 `.jfif`），
///   也交给后续解码判断能否处理；
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

/// 运行短命令并收集标准输出（stdin、stderr 丢弃）。
/// 经 `python_proc::spawn_exclusive` 启动：并发启动的子进程可能继承这里的管道写端，读取会一直等到它退出
fn command_stdout(mut cmd: Command) -> std::io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    python_proc::spawn_exclusive(&mut cmd)?.wait_with_output()
}

/// 运行只看退出状态的短命令（输出全部丢弃，不建管道），返回是否成功退出
fn command_succeeds(mut cmd: Command) -> bool {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    python_proc::spawn_exclusive(&mut cmd)
        .and_then(|mut child| child.wait())
        .is_ok_and(|status| status.success())
}

/// 通过 nvidia-smi 检测 NVIDIA 显卡（Windows 上隐藏控制台窗口）
fn detect_nvidia_gpu() -> Option<(String, f32, u64, u64, f32)> {
    let mut cmd = python_proc::hidden_command("nvidia-smi");
    cmd.args([
        "--query-gpu=name,utilization.gpu,memory.used,memory.total",
        "--format=csv,noheader,nounits",
    ]);

    let output = command_stdout(cmd).ok()?;

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
            let mut sysctl = Command::new("sysctl");
            sysctl.args(["-n", "machdep.cpu.brand_string"]);
            let chip = command_stdout(sysctl)
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            // 从 system_profiler 获取 GPU 名称
            let mut profiler = Command::new("system_profiler");
            profiler.args(["SPDisplaysDataType", "-json"]);
            let sp_output = command_stdout(profiler).ok();

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

    let mut ioreg = Command::new("ioreg");
    ioreg.args(["-r", "-l", "-c", "IOAccelerator"]);
    let output = command_stdout(ioreg).ok();
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

    let client = http_download::api_client(std::time::Duration::from_secs(15))?;

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
        command_succeeds(cmd);
    }
    #[cfg(unix)]
    {
        // 先杀进程组（负 PID 表示进程组），失败再退化为单进程
        let kill = |target: String| {
            let mut cmd = Command::new("kill");
            cmd.args(["-9", &target]);
            command_succeeds(cmd)
        };
        if !kill(format!("-{}", pid)) {
            kill(pid.to_string());
        }
    }
}

#[cfg(test)]
mod artifact_dir_tests {
    use super::*;
    use crate::commands::test_support::TempDir;

    fn fixture(tag: &str) -> TempDir {
        let root = TempDir::new(tag);
        for rel in ["sub", "Fail", "Warn", "_errors", "_warnings"] {
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
        let root = fixture("artifact_prune");
        let files = collect_image_files_with_recursive(&root, true).unwrap();
        // 产物目录中的副本不属于数据集图片。
        assert_eq!(names(&files), vec!["a.png", "d.png"]);
    }

    #[test]
    fn artifact_dir_scanned_when_it_is_the_input_root() {
        let root = fixture("artifact_as_root");
        // 输入根本身是产物目录时仍允许重跑其中的图片。
        let files = collect_image_files_with_recursive(&root.join("Fail"), true).unwrap();
        assert_eq!(names(&files), vec!["b.png"]);
    }

    /// 输入选的是单张图片时，产物目录要落在它所在的文件夹，
    /// 而不是拼成 `图片.png/Fail`（Windows 上报"当文件已存在时，无法创建该文件"）
    #[test]
    fn artifact_dir_falls_back_to_parent_for_single_image() {
        let root = fixture("artifact_single");
        let single = root.join("a.png");
        assert_eq!(dir_of(&single), root.to_path_buf());
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
    }

    #[test]
    fn copy_into_artifact_dir_keeps_subdirs() {
        let root = fixture("artifact_copy");
        let failed = vec![root.join("a.png"), root.join("sub/d.png")];
        let copied = copy_files_into_artifact_dir(&root, &root, &failed, "Fail", true).unwrap();
        assert_eq!(copied, 2);
        assert!(root.join("Fail/a.png").exists());
        assert!(root.join("Fail/sub/d.png").exists());
        // 复制进去的副本下一轮扫描依然被剪掉，不会自我增殖
        let files = collect_image_files_with_recursive(&root, true).unwrap();
        assert_eq!(names(&files), vec!["a.png", "d.png"]);
    }

    /// 拿上次的 Fail/ 当输入、输出选回数据集根目录重跑：归集目标就是源文件自己，
    /// 不复制（自拷贝会把它截成 0 字节），照样计为已归集。递归时子目录同样
    #[test]
    fn archiving_files_already_in_fail_keeps_them_intact() {
        let root = TempDir::new("artifact_self");
        let input = root.join("Fail");
        std::fs::create_dir_all(input.join("sub")).unwrap();
        std::fs::write(input.join("bad.png"), b"bad bytes").unwrap();
        std::fs::write(input.join("sub/bad2.png"), b"bad2 bytes").unwrap();
        let failed = vec![input.join("bad.png"), input.join("sub/bad2.png")];

        for recursive in [false, true] {
            let files = if recursive { &failed[..] } else { &failed[..1] };
            let copied =
                copy_files_into_artifact_dir(&input, &root, files, FAIL_DIR_NAME, recursive)
                    .unwrap();
            assert_eq!(copied as usize, files.len(), "recursive={recursive}");
            assert_eq!(std::fs::read(input.join("bad.png")).unwrap(), b"bad bytes");
        }
        assert_eq!(
            std::fs::read(input.join("sub/bad2.png")).unwrap(),
            b"bad2 bytes"
        );
        // 归集走的就是这个函数：FileBatch::archive_failures、ProblemArchive::report
        let events = ProblemArchive::new(&input, &root, true).archive(&failed, &[]);
        assert_eq!(events[0].message, "已将 2 个失败文件复制到 Fail/ 文件夹");
        assert_eq!(std::fs::read(input.join("bad.png")).unwrap(), b"bad bytes");
    }

    /// 输入目录名是小写的 fail：大小写不敏感的文件系统上它就是 Fail/，源文件同样不能被截断
    #[test]
    fn archiving_from_a_lowercase_fail_dir_keeps_sources_intact() {
        let root = TempDir::new("artifact_self_lower");
        let input = root.join("fail");
        std::fs::create_dir_all(&input).unwrap();
        std::fs::write(input.join("bad.png"), b"bad bytes").unwrap();
        let copied = copy_files_into_artifact_dir(
            &input,
            &root,
            &[input.join("bad.png")],
            FAIL_DIR_NAME,
            false,
        )
        .unwrap();
        assert_eq!(copied, 1);
        assert_eq!(std::fs::read(input.join("bad.png")).unwrap(), b"bad bytes");
    }

    #[test]
    fn same_path_matches_case_variants_dots_and_links() {
        let root = TempDir::new("same_path");
        let dir = root.join("Data");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();

        assert!(same_path(&dir, &dir));
        assert!(same_path(&dir, &root.join("data")));
        assert!(same_path(&dir, &dir.join(".")));
        assert!(same_path(&dir.join("a.png"), &dir.join("sub/../a.png")));
        assert!(same_path(&dir.join("a.png"), &root.join("./Data/a.png")));
        assert!(!same_path(&dir, &root));
        assert!(!same_path(&dir.join("a.png"), &dir.join("b.png")));
        // 不存在的路径只按大小写不敏感的文本比较
        assert!(same_path(&root.join("Missing"), &root.join("missing")));
        assert!(!same_path(&root.join("missing"), &root.join("other")));
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            assert!(same_path(&dir, &link));
            assert!(same_path(&dir.join("a.png"), &link.join("a.png")));
        }
    }

    /// 自拷贝守卫认得规范化后相同的路径，源文件不会被截断
    #[test]
    fn copy_file_safe_skips_the_same_file_written_differently() {
        let root = TempDir::new("copy_safe_same");
        let file = root.join("a.png");
        std::fs::write(&file, b"source bytes").unwrap();
        copy_file_safe(&file, &root.join(".").join("a.png")).unwrap();
        copy_file_safe(&file, &root.join("A.PNG")).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"source bytes");
        let copy = root.join("copy.png");
        copy_file_safe(&file, &copy).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"source bytes");
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use image::DynamicImage;
    use serde_json::json;

    const EVENT: &str = "helper-test-progress";

    #[test]
    fn python_log_forwards_message_and_i18n_fields() {
        let full = json!({"type": "log", "message": "加载模型", "i18n_key": "tagger.loading", "i18n_params": {"n": 3}});
        assert_eq!(
            serde_json::to_value(ProgressEvent::python_log(&full, 5, 12).file("a.png")).unwrap(),
            json!({
                "current": 5, "total": 12, "filename": "a.png", "status": "info",
                "message": "加载模型", "i18n_key": "tagger.loading", "i18n_params": {"n": 3},
            })
        );
        let plain =
            json!({"current": 0, "total": 0, "filename": "", "status": "info", "message": "plain"});
        assert_eq!(
            serde_json::to_value(ProgressEvent::python_log(
                &json!({"type": "log", "message": "plain"}),
                0,
                0
            ))
            .unwrap(),
            plain
        );
        // 写成 null 的参数仍然原样带上
        assert_eq!(
            serde_json::to_value(ProgressEvent::python_log(
                &json!({"type": "log", "i18n_params": null}),
                0,
                0
            ))
            .unwrap(),
            json!({"current": 0, "total": 0, "filename": "", "status": "info", "message": "", "i18n_params": null})
        );
        // 类型不对的字段、非对象的消息都按缺省处理
        let empty =
            json!({"current": 1, "total": 2, "filename": "", "status": "info", "message": ""});
        for msg in [
            json!({"type": "log", "message": 42, "i18n_key": 7}),
            json!("not an object"),
        ] {
            assert_eq!(
                serde_json::to_value(ProgressEvent::python_log(&msg, 1, 2)).unwrap(),
                empty,
                "{}",
                msg
            );
        }
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
    fn unique_destination_counter_style() {
        let dir = TempDir::new("helpers_unique");
        let free = |name: &str| unique_destination(&dir, name, NameSuffix::Counter, Path::exists);
        assert_eq!(free("a.png"), dir.join("a.png"));
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        assert_eq!(free("a.png"), dir.join("a_1.png"));
        std::fs::write(dir.join("a_1.png"), b"x").unwrap();
        assert_eq!(free("a.png"), dir.join("a_2.png"));
        std::fs::write(dir.join("noext"), b"x").unwrap();
        assert_eq!(free("noext"), dir.join("noext_1"));
    }

    #[test]
    fn unique_destination_rename_style_with_custom_check() {
        let dir = TempDir::new("helpers_unique_rename");
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        let rename = |taken: &dyn Fn(&Path) -> bool| {
            unique_destination(&dir, "a.png", NameSuffix::Rename, taken)
        };
        assert_eq!(rename(&Path::exists), dir.join("a_rename.png"));
        std::fs::write(dir.join("a_rename.png"), b"x").unwrap();
        assert_eq!(rename(&Path::exists), dir.join("a_rename_1.png"));
        // 让位时标签文件的目标也要空闲
        std::fs::write(dir.join("a_rename_1.txt"), b"x").unwrap();
        let with_tags =
            |p: &Path| p.exists() || tag_sidecars(p).any(|(_, sidecar)| sidecar.exists());
        assert_eq!(rename(&with_tags), dir.join("a_rename_2.png"));
    }

    #[test]
    fn tag_sidecars_follow_the_image_stem() {
        let image = Path::new("/data/sub/a.b.png");
        assert_eq!(
            tag_sidecars(image).collect::<Vec<_>>(),
            vec![
                ("txt", PathBuf::from("/data/sub/a.b.txt")),
                ("json", PathBuf::from("/data/sub/a.b.json")),
                ("caption", PathBuf::from("/data/sub/a.b.caption")),
            ]
        );
    }

    #[test]
    fn flatten_white_blends_onto_white() {
        let rgba = DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(3, 1, vec![10, 20, 30, 0, 10, 20, 30, 128, 10, 20, 30, 255])
                .unwrap(),
        );
        // 全透明成白色，半透明按整数混合，不透明保持原色
        assert_eq!(
            flatten_to_rgb_white(rgba).to_rgb8().into_raw(),
            vec![255, 255, 255, 132, 137, 142, 10, 20, 30]
        );
        let luma_alpha = DynamicImage::ImageLumaA8(
            image::GrayAlphaImage::from_raw(2, 1, vec![100, 0, 100, 255]).unwrap(),
        );
        assert_eq!(
            flatten_to_rgb_white(luma_alpha).to_rgb8().into_raw(),
            vec![255, 255, 255, 100, 100, 100]
        );
        let rgba16 = DynamicImage::ImageRgba16(
            image::ImageBuffer::from_raw(1, 1, vec![65535u16, 0, 32896, 65535]).unwrap(),
        );
        assert_eq!(
            flatten_to_rgb_white(rgba16).to_rgb8().into_raw(),
            vec![255, 0, 128]
        );
        let rgb = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([1, 2, 3])));
        assert_eq!(flatten_to_rgb_white(rgb.clone()), rgb);
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

    fn collect_fixture(tag: &str) -> TempDir {
        let root = TempDir::new(tag);
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
    fn collect_image_files_filters_directories_by_extension() {
        let root = collect_fixture("helpers_collect");
        // 非递归只看第一层，扩展名不分大小写，按文件名排序
        assert_eq!(
            rel_names(
                &root,
                &collect_image_files_with_recursive(&root, false).unwrap()
            ),
            vec!["b.PNG", "z.webp"]
        );
        // 输入根叫 Fail 时照常扫描
        assert_eq!(
            collect_image_files_with_recursive(&root.join("Fail"), false).unwrap(),
            vec![root.join("Fail/g.png")]
        );
        assert_eq!(
            collect_image_files_with_recursive(&root.join("missing"), false).unwrap_err(),
            format!("输入路径无效: {}", root.join("missing").display())
        );
    }

    /// 单选的文件不按扩展名过滤，扩展名不在列表里（.jfif、.psd、甚至 .txt）也原样交给后续解码
    #[test]
    fn single_file_input_is_returned_as_is() {
        let root = collect_fixture("helpers_single");
        std::fs::write(root.join("photo.jfif"), b"x").unwrap();
        for name in ["photo.jfif", "b.PNG", "a.psd", "c.txt"] {
            assert_eq!(
                collect_image_files_with_recursive(&root.join(name), true).unwrap(),
                vec![root.join(name)]
            );
        }
    }

    #[test]
    fn collect_files_matching_with_custom_predicate() {
        let root = collect_fixture("helpers_predicate");
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

        assert_eq!(
            collect_files_matching(&root.join("missing"), false, None, |_| true).unwrap_err(),
            format!("输入路径无效: {}", root.join("missing").display())
        );
    }

    #[test]
    fn name_helpers() {
        assert_eq!(file_name_lossy(Path::new("/a/b.png")), "b.png");
        assert_eq!(file_name_lossy(Path::new("/")), "");
        assert!(has_extension(Path::new("x.JPeG"), IMAGE_EXTS));
        assert!(!has_extension(Path::new("x.psd"), IMAGE_EXTS));
        assert!(!has_extension(Path::new("png"), IMAGE_EXTS));

        let root = TempDir::new("helpers_same_name");
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
    }

    /// 进程组杀不掉（子进程不是组长）时按退出状态退化为单杀
    #[cfg(unix)]
    #[test]
    fn kill_process_tree_falls_back_to_the_single_process() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let mut child = python_proc::spawn_exclusive(&mut cmd).unwrap();
        kill_process_tree(child.id());
        let status = child.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
    }

    #[test]
    fn short_commands_report_status_and_output() {
        #[cfg(unix)]
        {
            assert!(command_succeeds(Command::new("true")));
            assert!(!command_succeeds(Command::new("false")));
            let mut echo = Command::new("echo");
            echo.arg("hello");
            assert_eq!(command_stdout(echo).unwrap().stdout, b"hello\n");
        }
        assert!(!command_succeeds(Command::new(
            "purinbox-command-that-does-not-exist"
        )));
    }
}

#[cfg(test)]
mod run_and_archive_tests {
    use super::*;
    use crate::commands::batch::{capture_events, capture_raw_events};
    use crate::commands::test_support::TempDir;
    use serde_json::json;

    #[test]
    fn begin_run_scopes_ids_per_channel() {
        let (a, b) = ("run-test-channel-a", "run-test-channel-b");
        let app = tauri::test::mock_app();
        let log_a = capture_raw_events(app.handle(), a);
        let log_b = capture_raw_events(app.handle(), b);

        // 没开始过任何一轮的通道不带 run_id
        ProgressEvent::new("info", "before").emit(app.handle(), a);
        let first = begin_run(a);
        ProgressEvent::new("info", "a1").emit(app.handle(), a);
        let other = begin_run(b);
        ProgressEvent::new("info", "b1").emit(app.handle(), b);
        ProgressEvent::new("info", "a1 again").emit(app.handle(), a);
        let second = begin_run(a);
        ProgressEvent::new("done", "a2").emit(app.handle(), a);
        // 显式指定的不被覆盖
        ProgressEvent::new("done", "late")
            .for_run(first)
            .emit(app.handle(), a);

        assert!(first < other && other < second);
        let ids =
            |log: &std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>| -> Vec<Option<u64>> {
                log.lock()
                    .unwrap()
                    .iter()
                    .map(|e| e["run_id"].as_u64())
                    .collect()
            };
        assert_eq!(
            ids(&log_a),
            [None, Some(first), Some(first), Some(second), Some(first)]
        );
        assert_eq!(ids(&log_b), [Some(other)]);
        assert!(log_a.lock().unwrap()[0].get("run_id").is_none());
    }

    #[test]
    fn cancelled_flag_is_serialized_only_when_set() {
        assert_eq!(
            serde_json::to_value(ProgressEvent::new("done", "已取消").at(1, 2).cancelled())
                .unwrap(),
            json!({"current": 1, "total": 2, "filename": "", "status": "done",
                   "message": "已取消", "cancelled": true})
        );
        let plain = serde_json::to_value(ProgressEvent::new("done", "完成").for_run(7)).unwrap();
        assert_eq!(plain["run_id"], 7);
        assert!(plain.get("cancelled").is_none());
    }

    const EVENT: &str = "archive-test-progress";

    fn info(message: &str) -> serde_json::Value {
        json!({"current": 0, "total": 0, "filename": "", "status": "info", "message": message})
    }

    /// 归集到输出根（不是输入目录），递归时保留子目录；单张图片的输出路径落到所在目录
    #[test]
    fn problem_archive_copies_into_output_root() {
        let root = TempDir::new("archive_output");
        let (input, output) = (root.join("in"), root.join("out"));
        std::fs::create_dir_all(input.join("sub")).unwrap();
        for rel in ["a.png", "sub/b.png", "sub/c.png"] {
            std::fs::write(input.join(rel), rel).unwrap();
        }
        let app = tauri::test::mock_app();
        let log = capture_events(app.handle(), EVENT);

        ProblemArchive::new(&input, &output, true).report(
            app.handle(),
            EVENT,
            &[input.join("a.png"), input.join("sub/b.png")],
            &[input.join("sub/c.png")],
        );
        ProblemArchive::new(&input, &output, true).report(app.handle(), EVENT, &[], &[]);

        assert_eq!(
            *log.lock().unwrap(),
            [
                info("已将 2 个失败文件复制到 Fail/ 文件夹"),
                info("已将 1 个警告文件复制到 Warn/ 文件夹"),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(output.join("Fail/sub/b.png")).unwrap(),
            "sub/b.png"
        );
        assert!(output.join("Fail/a.png").is_file());
        assert!(output.join("Warn/sub/c.png").is_file());
        assert!(!input.join("Fail").exists() && !input.join("Warn").exists());

        // 非递归：都放在 Fail/ 第一层；输出根是单张图片时落到它所在的目录
        let single = input.join("a.png");
        let events =
            ProblemArchive::new(&single, &single, false).archive(&[input.join("sub/b.png")], &[]);
        assert_eq!(events.len(), 1);
        assert!(input.join("Fail/b.png").is_file());
        assert!(!single.join("Fail").exists());
    }

    #[test]
    fn problem_archive_skips_warnings_and_reports_errors() {
        let root = TempDir::new("archive_skip");
        std::fs::write(root.join("a.png"), b"a").unwrap();
        std::fs::write(root.join("w.png"), b"w").unwrap();
        let archive = ProblemArchive::new(&root, &root, false).skip_warnings(true);
        let events: Vec<_> = archive
            .archive(&[root.join("a.png")], &[root.join("w.png")])
            .into_iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        assert_eq!(
            events,
            [
                info("已将 1 个失败文件复制到 Fail/ 文件夹"),
                info("输出与输入目录相同，已跳过 Warn/ 复制"),
            ]
        );
        assert!(!root.join("Warn").exists());
        // 只有警告、但跳过时也只发说明
        assert_eq!(archive.archive(&[], &[root.join("w.png")]).len(), 1);

        // 同名普通文件占住了 Warn 目录的位置
        std::fs::write(root.join("Warn"), b"x").unwrap();
        let events = ProblemArchive::new(&root, &root, false).archive(&[], &[root.join("w.png")]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, "error");
        assert!(
            events[0].message.starts_with("创建 Warn 文件夹失败: "),
            "{}",
            events[0].message
        );
    }

    #[test]
    fn finalize_part_file_replaces_existing_file() {
        let root = TempDir::new("finalize_part");
        let dest = root.join("model.onnx");
        std::fs::write(&dest, b"old").unwrap();
        let part = prepare_part_file(&dest);
        std::fs::write(&part, b"new bytes").unwrap();
        finalize_part_file(&part, &dest, 9, 9).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new bytes");
        assert!(!part.exists());

        // 没有 content-length（total 为 0）时不校验大小，目标不存在也能落盘
        let fresh = root.join("fresh.bin");
        let part = prepare_part_file(&fresh);
        std::fs::write(&part, b"x").unwrap();
        finalize_part_file(&part, &fresh, 1, 0).unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"x");
    }

    #[test]
    fn finalize_part_file_failures_keep_the_old_file() {
        let root = TempDir::new("finalize_part_fail");
        let dest = root.join("model.onnx");
        std::fs::write(&dest, b"old").unwrap();
        let part = prepare_part_file(&dest);
        std::fs::write(&part, b"short").unwrap();
        assert_eq!(
            finalize_part_file(&part, &dest, 5, 10).unwrap_err(),
            "下载不完整: 预期 10 字节，实际 5 字节"
        );
        assert!(!part.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");

        // 改名失败（目标位置是非空目录）：旧内容原样保留，.part 被清理
        let dir_dest = root.join("occupied");
        std::fs::create_dir_all(dir_dest.join("inner")).unwrap();
        let part = prepare_part_file(&dir_dest);
        std::fs::write(&part, b"data").unwrap();
        let err = finalize_part_file(&part, &dir_dest, 4, 4).unwrap_err();
        assert!(err.starts_with("替换文件失败"), "{err}");
        assert!(!part.exists());
        assert!(dir_dest.join("inner").is_dir());
    }
}
