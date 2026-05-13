use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod image_scale;
pub mod image_crop;
pub mod image_flip;
pub mod person_crop;
pub mod resolution_filter;
pub mod file_keeper;
pub mod format_convert;
pub mod alpha_convert;
pub mod batch_rename;
pub mod tagger;
pub mod tag_manager;
pub mod translator;
pub mod tag_sort;
pub mod api_config;
pub mod proxy_config;
pub mod bucket_preview;
pub mod perspective;
pub mod blur_noise;
pub mod upscale;
pub mod python_env;
pub mod image_cluster;
pub mod image_dedup;

/// 进度事件 payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub current: u32,
    pub total: u32,
    pub filename: String,
    /// "processing" | "success" | "error" | "done"
    pub status: String,
    pub message: String,
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
                        name: p.file_name().unwrap_or_default().to_string_lossy().to_string(),
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
        let cpu_name = sys.cpus().first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown".into());
        let cpu_cores = sys.cpus().len();

        let memory_total = sys.total_memory();
        let memory_used = sys.used_memory();
        let memory_percent = if memory_total > 0 {
            (memory_used as f64 / memory_total as f64 * 100.0) as f32
        } else { 0.0 };

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
    ("未检测到 GPU".into(), -1.0, 0, 0, -1.0)
}

/// 通过 nvidia-smi 检测 NVIDIA 显卡（Windows 上隐藏控制台窗口）
fn detect_nvidia_gpu() -> Option<(String, f32, u64, u64, f32)> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"]);

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
    } else { 0.0 };

    Some((name, usage, vram_used, vram_total, vram_percent))
}

/// macOS: 检测 Apple Silicon GPU
#[cfg(target_os = "macos")]
fn detect_apple_gpu() -> Option<(String, f32, u64, u64, f32)> {
    // 获取 GPU 芯片名称
    let chip_output = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()?;

    let chip = String::from_utf8_lossy(&chip_output.stdout).trim().to_string();

    // 从 system_profiler 获取 GPU 名称
    let sp_output = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
        .ok()?;

    let sp_str = String::from_utf8_lossy(&sp_output.stdout);
    let gpu_name = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&sp_str) {
        json["SPDisplaysDataType"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|gpu| gpu["sppci_model"].as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if chip.contains("Apple") { format!("{} GPU", chip.split_whitespace().take(3).collect::<Vec<_>>().join(" ")) }
                else { "Apple GPU".into() }
            })
    } else {
        "Apple GPU".into()
    };

    // 通过 ioreg 获取 GPU 使用率和显存
    let gpu_usage = get_apple_gpu_utilization().unwrap_or(-1.0);

    // Apple Silicon 统一内存 — GPU 共享系统 RAM
    // 从 sysinfo 获取总内存，从 ioreg 获取 GPU 已用显存
    let (vram_used, vram_total) = get_apple_gpu_memory();

    let vram_percent = if vram_total > 0 {
        (vram_used as f64 / vram_total as f64 * 100.0) as f32
    } else { -1.0 };

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
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
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

    let client = reqwest::Client::builder()
        .user_agent("PurinBox")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    let resp = client.get(url).send().await
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

    let json: serde_json::Value = resp.json().await
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
    let parse = |s: &str| -> Vec<u32> {
        s.split('.').filter_map(|p| p.parse().ok()).collect()
    };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv > cv { return true; }
        if lv < cv { return false; }
    }
    false
}
