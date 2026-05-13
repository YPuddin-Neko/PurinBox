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

// ============================================================
// JSON 结构化标签管理 — AnimaLoraStudio 完整格式
// ============================================================

/// fixed: 固定字段，不参与 shuffle
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonFixed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
}

/// character: 角色信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonCharacter {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub variant: String,
}

/// from_path: 从目录路径自动提取的外观标签
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonFromPath {
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "deserialize_string_or_array")]
    pub appearance: Vec<String>,
}

/// 自定义反序列化：支持 JSON 数组 ["a","b"] 或逗号字符串 "a, b"
fn deserialize_string_or_array<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where D: serde::Deserializer<'de> {
    use serde::de;
    struct StringOrArray;
    impl<'de> de::Visitor<'de> for StringOrArray {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or array of strings")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<String>, E> {
            Ok(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut v = Vec::new();
            while let Some(s) = seq.next_element::<String>()? { v.push(s); }
            Ok(v)
        }
    }
    deserializer.deserialize_any(StringOrArray)
}

/// ai_output: VLM/Tagger 打标输出
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonAiOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "deserialize_string_or_array")]
    pub appearance: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "deserialize_string_or_array")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "deserialize_string_or_array")]
    pub environment: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nl: Option<String>,
}

/// 完整 JSON 标签结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonTagData {
    #[serde(default)]
    pub fixed: JsonFixed,
    #[serde(default)]
    pub character: JsonCharacter,
    #[serde(default)]
    pub from_path: JsonFromPath,
    #[serde(default)]
    pub ai_output: JsonAiOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonImageItem {
    pub path: String,
    pub filename: String,
    pub data: JsonTagData,
    pub has_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonDataset {
    pub folder: String,
    pub images: Vec<JsonImageItem>,
    /// "full" | "simplified" | "unknown"
    pub detected_format: String,
}

/// 加载 JSON 标签数据集
#[tauri::command]
pub fn load_json_dataset(folder: String) -> Result<JsonDataset, String> {
    let dir = Path::new(&folder);
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("目录不存在: {}", folder));
    }

    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp", "tiff", "tif", "gif"];
    let mut images: Vec<JsonImageItem> = Vec::new();
    let mut detected_format = "unknown".to_string();

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
        let json_path = p.with_extension("json");

        let (data, has_json, fmt) = if json_path.exists() {
            match std::fs::read_to_string(&json_path) {
                Ok(content) => {
                    // 先检查顶层 keys 判断格式
                    let is_full = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        v.as_object().map(|o| o.contains_key("ai_output") || o.contains_key("fixed") || o.contains_key("from_path")).unwrap_or(false)
                    } else { false };

                    if is_full {
                        match serde_json::from_str::<JsonTagData>(&content) {
                            Ok(d) => (d, true, "full"),
                            Err(_) => (JsonTagData::default(), true, "unknown"),
                        }
                    } else {
                        match parse_simplified_format(&content) {
                            Some(d) => (d, true, "simplified"),
                            None => (JsonTagData::default(), true, "unknown"),
                        }
                    }
                }
                Err(_) => (JsonTagData::default(), false, "unknown"),
            }
        } else {
            (JsonTagData::default(), false, "unknown")
        };

        if has_json && detected_format == "unknown" {
            detected_format = fmt.to_string();
        }

        images.push(JsonImageItem { path: p.to_string_lossy().to_string(), filename, data, has_json });
    }

    images.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(JsonDataset { folder, images, detected_format })
}

/// 解析简化格式 JSON（扁平结构）转为完整格式
fn parse_simplified_format(content: &str) -> Option<JsonTagData> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    let obj = v.as_object()?;

    let mut data = JsonTagData::default();

    // fixed
    data.fixed.quality = obj.get("quality").and_then(|v| v.as_str()).map(|s| s.to_string());
    data.fixed.series = obj.get("series").and_then(|v| v.as_str()).map(|s| s.to_string());
    data.fixed.artist = obj.get("artist").and_then(|v| v.as_str()).map(|s| s.to_string());

    // character — 简化格式中是 string
    if let Some(ch) = obj.get("character").and_then(|v| v.as_str()) {
        data.character.name = ch.to_string();
    }

    // ai_output 扁平字段
    data.ai_output.count = obj.get("count").and_then(|v| v.as_str()).map(|s| s.to_string());
    data.ai_output.appearance = extract_string_array(obj.get("appearance"));
    data.ai_output.tags = extract_string_array(obj.get("tags"));
    data.ai_output.environment = extract_string_array(obj.get("environment"));
    data.ai_output.nl = obj.get("nl").and_then(|v| v.as_str()).map(|s| s.to_string());

    Some(data)
}

fn extract_string_array(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }
        // 兼容：值是逗号分隔的字符串而非数组
        Some(serde_json::Value::String(s)) => {
            s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveJsonItem {
    pub path: String,
    pub data: JsonTagData,
}

/// 将完整格式转为简化格式 JSON Value
fn to_simplified(data: &JsonTagData) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    if let Some(q) = &data.fixed.quality { out.insert("quality".into(), serde_json::Value::String(q.clone())); }
    if let Some(s) = &data.fixed.series { out.insert("series".into(), serde_json::Value::String(s.clone())); }
    if let Some(a) = &data.fixed.artist { out.insert("artist".into(), serde_json::Value::String(a.clone())); }
    if !data.character.name.is_empty() { out.insert("character".into(), serde_json::Value::String(data.character.name.clone())); }
    if let Some(c) = &data.ai_output.count { out.insert("count".into(), serde_json::Value::String(c.clone())); }
    let mut appearance: Vec<String> = data.from_path.appearance.clone();
    for t in &data.ai_output.appearance { if !appearance.contains(t) { appearance.push(t.clone()); } }
    if !appearance.is_empty() { out.insert("appearance".into(), serde_json::Value::Array(appearance.into_iter().map(serde_json::Value::String).collect())); }
    if !data.ai_output.tags.is_empty() { out.insert("tags".into(), serde_json::Value::Array(data.ai_output.tags.iter().map(|s| serde_json::Value::String(s.clone())).collect())); }
    if !data.ai_output.environment.is_empty() { out.insert("environment".into(), serde_json::Value::Array(data.ai_output.environment.iter().map(|s| serde_json::Value::String(s.clone())).collect())); }
    if let Some(nl) = &data.ai_output.nl { out.insert("nl".into(), serde_json::Value::String(nl.clone())); }
    serde_json::Value::Object(out)
}

fn serialize_json(data: &JsonTagData, simplified: bool) -> Result<String, String> {
    if simplified {
        serde_json::to_string_pretty(&to_simplified(data))
    } else {
        serde_json::to_string_pretty(data)
    }.map_err(|e| format!("JSON 序列化失败: {}", e))
}

#[tauri::command]
pub fn save_single_json_file(image_path: String, data: JsonTagData, simplified: bool) -> Result<(), String> {
    let img = Path::new(&image_path);
    if !img.exists() { return Err(format!("图片不存在: {}", image_path)); }
    let json_path = img.with_extension("json");
    let content = serialize_json(&data, simplified)?;
    std::fs::write(&json_path, &content).map_err(|e| format!("写入失败 {}: {}", json_path.display(), e))?;
    Ok(())
}

#[tauri::command]
pub fn save_all_json_files(items: Vec<SaveJsonItem>, simplified: bool) -> Result<u32, String> {
    let mut saved = 0u32;
    for item in &items {
        let img = Path::new(&item.path);
        let json_path = img.with_extension("json");
        let content = match serialize_json(&item.data, simplified) {
            Ok(c) => c, Err(e) => { eprintln!("{}", e); continue; }
        };
        if let Err(e) = std::fs::write(&json_path, &content) { eprintln!("保存失败 {}: {}", json_path.display(), e); continue; }
        saved += 1;
    }
    Ok(saved)
}
