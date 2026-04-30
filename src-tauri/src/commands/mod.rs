use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod image_scale;
pub mod image_flip;
pub mod resolution_filter;
pub mod file_keeper;
pub mod format_convert;
pub mod alpha_convert;
pub mod batch_rename;
pub mod tagger;

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

/// 通过 nvidia-smi 检测 NVIDIA 显卡
fn detect_nvidia_gpu() -> Option<(String, f32, u64, u64, f32)> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

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
                // Apple Silicon 的 GPU 名称通常包含芯片名
                if chip.contains("Apple") { format!("{} GPU", chip.split_whitespace().take(3).collect::<Vec<_>>().join(" ")) }
                else { "Apple GPU".into() }
            })
    } else {
        "Apple GPU".into()
    };

    // Apple Silicon 统一内存架构 — GPU 和 CPU 共享内存
    // 无法单独获取 GPU 使用率和显存（需要 Metal API 或 IOKit）
    // 返回 -1 表示不支持独立 GPU 监控
    Some((gpu_name, -1.0, 0, 0, -1.0))
}

