use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagImageItem {
    pub path: String,
    pub filename: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDataset {
    pub folder: String,
    pub images: Vec<TagImageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveTagItem {
    pub path: String,
    pub tags: Vec<String>,
}

/// 加载标签数据集：扫描文件夹中的图片，读取对应 .txt 文件的标签
#[tauri::command]
pub fn load_tag_dataset(folder: String) -> Result<TagDataset, String> {
    let dir = Path::new(&folder);
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("目录不存在: {}", folder));
    }

    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
    let mut images: Vec<TagImageItem> = Vec::new();

    for entry in walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if !p.is_file() { continue; }
        let ext = match p.extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => continue,
        };
        if !supported_exts.contains(&ext.as_str()) { continue; }

        let filename = p.file_name().unwrap_or_default().to_string_lossy().to_string();

        // 读取对应 .txt 文件
        let txt_path = p.with_extension("txt");
        let tags = if txt_path.exists() {
            match std::fs::read_to_string(&txt_path) {
                Ok(content) => content
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        images.push(TagImageItem {
            path: p.to_string_lossy().to_string(),
            filename,
            tags,
        });
    }

    images.sort_by(|a, b| a.filename.cmp(&b.filename));

    Ok(TagDataset {
        folder: folder.clone(),
        images,
    })
}

/// 自然语言描述数据集加载（读取原始文本，不按逗号分割）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionImageItem {
    pub path: String,
    pub filename: String,
    pub caption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionDataset {
    pub folder: String,
    pub images: Vec<CaptionImageItem>,
}

#[tauri::command]
pub fn load_caption_dataset(folder: String) -> Result<CaptionDataset, String> {
    let dir = Path::new(&folder);
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("目录不存在: {}", folder));
    }

    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
    let mut images: Vec<CaptionImageItem> = Vec::new();

    for entry in walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if !p.is_file() { continue; }
        let ext = match p.extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => continue,
        };
        if !supported_exts.contains(&ext.as_str()) { continue; }

        let filename = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        let txt_path = p.with_extension("txt");
        let caption = if txt_path.exists() {
            std::fs::read_to_string(&txt_path).unwrap_or_default()
        } else {
            String::new()
        };

        images.push(CaptionImageItem { path: p.to_string_lossy().to_string(), filename, caption });
    }

    images.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(CaptionDataset { folder, images })
}

/// 保存单个图片的标签到 .txt 文件
#[tauri::command]
pub fn save_single_tag_file(image_path: String, tags: Vec<String>) -> Result<(), String> {
    let img = Path::new(&image_path);
    if !img.exists() {
        return Err(format!("图片不存在: {}", image_path));
    }
    let txt_path = img.with_extension("txt");
    let content = tags.join(", ");
    std::fs::write(&txt_path, &content)
        .map_err(|e| format!("写入失败 {}: {}", txt_path.display(), e))?;
    Ok(())
}

/// 批量保存多个图片的标签
#[tauri::command]
pub fn save_all_tag_files(items: Vec<SaveTagItem>) -> Result<u32, String> {
    let mut saved = 0u32;
    for item in &items {
        let img = Path::new(&item.path);
        let txt_path = img.with_extension("txt");
        let content = item.tags.join(", ");
        if let Err(e) = std::fs::write(&txt_path, &content) {
            eprintln!("保存失败 {}: {}", txt_path.display(), e);
            continue;
        }
        saved += 1;
    }
    Ok(saved)
}

/// 保存单个图片的自然语言描述到 .txt 文件
#[tauri::command]
pub fn save_caption_file(image_path: String, content: String) -> Result<(), String> {
    let img = Path::new(&image_path);
    if !img.exists() {
        return Err(format!("图片不存在: {}", image_path));
    }
    let txt_path = img.with_extension("txt");
    std::fs::write(&txt_path, &content)
        .map_err(|e| format!("写入失败 {}: {}", txt_path.display(), e))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveCaptionItem {
    pub path: String,
    pub content: String,
}

/// 批量保存多个图片的自然语言描述
#[tauri::command]
pub fn save_all_caption_files(items: Vec<SaveCaptionItem>) -> Result<u32, String> {
    let mut saved = 0u32;
    for item in &items {
        let img = Path::new(&item.path);
        let txt_path = img.with_extension("txt");
        if let Err(e) = std::fs::write(&txt_path, &item.content) {
            eprintln!("保存失败 {}: {}", txt_path.display(), e);
            continue;
        }
        saved += 1;
    }
    Ok(saved)
}
