use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod aesthetic;
pub mod alpha_convert;
pub mod api_config;
pub mod batch_rename;
pub mod blur_noise;
pub mod bucket_preview;
pub mod config_paths;
pub mod dedup_rename;
pub mod file_keeper;
pub mod format_convert;
pub mod huggingface_config;
pub mod image_cluster;
pub mod image_crop;
pub mod image_dedup;
pub mod image_flip;
pub mod image_scale;
pub mod person_crop;
pub mod perspective;
pub mod proxy_config;
pub mod python_env;
pub mod resolution_filter;
pub mod sd_metadata;
pub mod tag_db;
pub mod tag_manager;
pub mod tag_refine;
pub mod tag_sort;
pub mod tagger;
pub mod translator;
pub mod upscale;

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

/// 进度事件 payload
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub current: u32,
    pub total: u32,
    #[serde(default)]
    pub filename: String,
    /// "processing" | "success" | "error" | "done"
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    /// i18n key（Python 脚本发送的国际化 key，前端优先使用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i18n_key: Option<String>,
    /// i18n 参数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i18n_params: Option<serde_json::Value>,
}

/// 全局 LLM 请求起点节流。
///
/// 第一次请求立即放行；之后所有并发 worker 共享同一个时间戳，
/// 保证新请求之间至少间隔 `request_interval_ms`。
pub async fn wait_for_global_llm_slot(
    last_req_time: &tokio::sync::Mutex<Option<std::time::Instant>>,
    request_interval_ms: i64,
    cancel_flag: &AtomicBool,
) -> bool {
    if request_interval_ms <= 0 {
        return true;
    }

    let mut last = last_req_time.lock().await;
    if let Some(previous) = *last {
        let interval_ms = request_interval_ms as u128;
        let elapsed_ms = previous.elapsed().as_millis();
        if elapsed_ms < interval_ms {
            let mut remaining_ms = (interval_ms - elapsed_ms) as u64;
            while remaining_ms > 0 {
                if cancel_flag.load(Ordering::SeqCst) {
                    return false;
                }
                let sleep_ms = remaining_ms.min(200);
                tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                remaining_ms -= sleep_ms;
            }
        }
    }

    if cancel_flag.load(Ordering::SeqCst) {
        return false;
    }

    *last = Some(std::time::Instant::now());
    true
}

/// 扫描指定目录下的所有图片文件，返回文件路径列表
#[tauri::command]
pub fn scan_images(dir: String) -> Result<Vec<ImageInfo>, String> {
    let path = Path::new(&dir);
    if !path.exists() || !path.is_dir() {
        return Err(format!("目录不存在: {}", dir));
    }

    let mut images = Vec::new();
    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];

    for entry in walkdir::WalkDir::new(path)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if supported_exts.contains(&ext_lower.as_str()) {
                    let (width, height) = match image::image_dimensions(p) {
                        Ok((w, h)) => (w, h),
                        Err(_) => (0, 0),
                    };
                    images.push(ImageInfo {
                        path: p.to_string_lossy().to_string(),
                        name: p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        width,
                        height,
                        size_bytes: p.metadata().map(|m| m.len()).unwrap_or(0),
                    });
                }
            }
        }
    }

    images.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(images)
}

fn is_supported_image_file(path: &Path) -> bool {
    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
    path.extension()
        .map(|ext| {
            let ext_lower = ext.to_string_lossy().to_lowercase();
            supported_exts.contains(&ext_lower.as_str())
        })
        .unwrap_or(false)
}

/// 收集目录中的图片文件路径
pub fn collect_image_files(input: &Path) -> Result<Vec<std::path::PathBuf>, String> {
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

/// 递归收集目录中的图片文件路径（包括子目录）
pub fn collect_image_files_recursive(input: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    collect_image_files_with_recursive(input, true)
}

pub fn collect_image_files_with_recursive(
    input: &Path,
    recursive: bool,
) -> Result<Vec<std::path::PathBuf>, String> {
    collect_image_files_with_recursive_excluding(input, recursive, None)
}

pub fn collect_image_files_with_recursive_excluding(
    input: &Path,
    recursive: bool,
    excluded_dir: Option<&Path>,
) -> Result<Vec<std::path::PathBuf>, String> {
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

        for entry in walker.into_iter().filter_map(|e| e.ok()) {
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
            if is_supported_image_file(p) {
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
    if !path.exists() || !path.is_dir() {
        return Err(format!("目录不存在: {}", dir));
    }

    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
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
                    .filter(|e| {
                        e.path().is_file()
                            && e.path()
                                .extension()
                                .map(|ext| {
                                    supported_exts
                                        .contains(&ext.to_string_lossy().to_lowercase().as_str())
                                })
                                .unwrap_or(false)
                    })
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
    if !base.exists() || !base.is_dir() {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub path: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    tokio::task::spawn_blocking(|| {
        let mut sys = System::new_all();
        // 需要短暂等待以获取准确的 CPU 数据
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_all();

        let cpu_usage = sys.global_cpu_usage();
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown".into());
        let cpu_cores = sys.cpus().len();

        let memory_total = sys.total_memory();
        let memory_used = sys.used_memory();
        let memory_percent = if memory_total > 0 {
            (memory_used as f64 / memory_total as f64 * 100.0) as f32
        } else {
            0.0
        };

        // GPU 检测
        let (gpu_name, gpu_usage, vram_used, vram_total, vram_percent) = detect_gpu();

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
fn detect_gpu() -> (String, f32, u64, u64, f32) {
    // 1. 尝试 nvidia-smi（Windows + Linux 上有 NVIDIA 显卡时）
    if let Some(result) = detect_nvidia_gpu() {
        return result;
    }

    // 2. macOS: 检测 Apple Silicon GPU（通过 system_profiler）
    #[cfg(target_os = "macos")]
    if let Some(result) = detect_apple_gpu() {
        return result;
    }

    // 3. 未检测到
    (String::new(), -1.0, 0, 0, -1.0)
}

/// 通过 nvidia-smi 检测 NVIDIA 显卡（Windows 上隐藏控制台窗口）
fn detect_nvidia_gpu() -> Option<(String, f32, u64, u64, f32)> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=name,utilization.gpu,memory.used,memory.total",
        "--format=csv,noheader,nounits",
    ]);

    // Windows: 添加 CREATE_NO_WINDOW 标志，防止弹出 CMD 窗口
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

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
fn detect_apple_gpu() -> Option<(String, f32, u64, u64, f32)> {
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

    // 通过 ioreg 获取 GPU 使用率和显存（这些是动态数值，每次都要读取）
    let gpu_usage = get_apple_gpu_utilization().unwrap_or(-1.0);

    // Apple Silicon 统一内存 — GPU 共享系统 RAM
    let (vram_used, vram_total) = get_apple_gpu_memory();

    let vram_percent = if vram_total > 0 {
        (vram_used as f64 / vram_total as f64 * 100.0) as f32
    } else {
        -1.0
    };

    Some((gpu_name, gpu_usage, vram_used, vram_total, vram_percent))
}

/// macOS: 从 ioreg PerformanceStatistics 字典中提取指定 key 的数值
#[cfg(target_os = "macos")]
fn extract_ioreg_perf_value(key: &str) -> Option<f64> {
    let output = std::process::Command::new("ioreg")
        .args(["-r", "-l", "-c", "IOAccelerator"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
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

/// macOS: 从 ioreg 获取 GPU Device Utilization %
#[cfg(target_os = "macos")]
fn get_apple_gpu_utilization() -> Option<f32> {
    extract_ioreg_perf_value("Device Utilization %").map(|v| v as f32)
}

/// macOS: 从 ioreg 获取 GPU 显存使用量，返回 (used, total)
#[cfg(target_os = "macos")]
fn get_apple_gpu_memory() -> (u64, u64) {
    let total = {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.total_memory()
    };

    let used = extract_ioreg_perf_value("In use system memory")
        .map(|v| v as u64)
        .unwrap_or(0);

    (used, total)
}

/// 版本更新检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub release_notes: String,
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
            current_version: current.to_string(),
            latest_version: current.to_string(),
            release_url: String::new(),
            release_notes: String::new(),
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
    let body = json["body"].as_str().unwrap_or("").to_string();

    let has_update = version_compare(latest, current);

    Ok(UpdateCheckResult {
        has_update,
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        release_url: html_url,
        release_notes: body,
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
