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

        // GPU 信息需要平台特定实现，这里暂时返回 N/A
        // 后续可以通过 NVML 获取 NVIDIA 显卡信息
        Ok(SystemStats {
            cpu_usage,
            cpu_name,
            cpu_cores,
            memory_used,
            memory_total,
            memory_percent,
            gpu_name: "检测中...".into(),
            gpu_usage: -1.0,
            vram_used: 0,
            vram_total: 0,
            vram_percent: -1.0,
        })
    })
    .await
    .map_err(|e| format!("获取系统信息失败: {}", e))?
}
