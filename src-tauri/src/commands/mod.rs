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
