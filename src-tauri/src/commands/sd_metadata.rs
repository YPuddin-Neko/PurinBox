use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::Emitter;

use super::ProgressEvent;

// ── Data types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdImageMeta {
    pub path: String,
    pub filename: String,
    pub positive: String,
    pub negative: String,
    pub params: String,
    pub artist: String,
    /// a1111 / comfyui / novelai / unknown
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdScanResult {
    pub items: Vec<SdImageMeta>,
    pub total_images: u32,
    pub has_meta_count: u32,
    pub no_meta_count: u32,
    pub no_meta_files: Vec<String>,
    pub source_counts: std::collections::HashMap<String, u32>,
    pub scan_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTagItem {
    pub source_path: String,
    pub positive: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTagsOptions {
    /// "same" = 同路径, "custom" = 指定文件夹
    pub mode: String,
    /// 当 mode="custom" 时的目标文件夹
    pub dest_folder: Option<String>,
    #[serde(default)]
    pub input_root: Option<String>,
    pub items: Vec<ExportTagItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub success_count: u32,
    pub fail_count: u32,
    pub skip_count: u32,
    pub errors: Vec<String>,
}

// ── Commands ──

#[tauri::command]
pub async fn scan_sd_metadata(
    app: tauri::AppHandle,
    input_path: String,
    recursive: Option<bool>,
) -> Result<SdScanResult, String> {
    tokio::task::spawn_blocking(move || scan_sync(&app, &input_path, recursive.unwrap_or(false)))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub async fn export_sd_tags(
    app: tauri::AppHandle,
    options: ExportTagsOptions,
) -> Result<ExportResult, String> {
    tokio::task::spawn_blocking(move || export_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 读取单个文件的元数据
#[tauri::command]
pub fn read_single_sd_metadata(file_path: String) -> Result<Option<SdImageMeta>, String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", file_path));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext != "png" {
        return Ok(None);
    }
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok(read_png_metadata(&path).map(|meta| SdImageMeta {
        path: file_path,
        filename,
        positive: meta.positive,
        negative: meta.negative,
        params: meta.params,
        artist: meta.artist,
        source: meta.source,
    }))
}

// ── Scan logic ──

fn scan_sync(
    app: &tauri::AppHandle,
    input_path: &str,
    recursive: bool,
) -> Result<SdScanResult, String> {
    let start = std::time::Instant::now();
    let dir = Path::new(input_path);
    let files = super::collect_image_files_with_recursive(dir, recursive)?;
    let total = files.len() as u32;

    let mut items = Vec::new();
    let mut no_meta_files: Vec<String> = Vec::new();
    let mut has_meta = 0u32;
    let mut no_meta = 0u32;
    let mut source_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    for (i, file_path) in files.iter().enumerate() {
        let filename = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let _ = app.emit(
            "sd-metadata-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: filename.clone(),
                status: "processing".to_string(),
                message: format!("[{}/{}] {}", i + 1, total, filename),
                ..Default::default()
            },
        );

        // Only parse PNG files for tEXt metadata
        let ext = file_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if ext != "png" {
            no_meta += 1;
            no_meta_files.push(filename);
            continue;
        }

        match read_png_metadata(file_path) {
            Some(meta) => {
                *source_counts.entry(meta.source.clone()).or_insert(0) += 1;
                has_meta += 1;
                items.push(SdImageMeta {
                    path: file_path.to_string_lossy().to_string(),
                    filename,
                    positive: meta.positive,
                    negative: meta.negative,
                    params: meta.params,
                    artist: meta.artist,
                    source: meta.source,
                });
            }
            None => {
                no_meta += 1;
                no_meta_files.push(filename);
            }
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    let _ = app.emit(
        "sd-metadata-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: format!("扫描完成: {} 张图片, {} 有元数据", total, has_meta),
            ..Default::default()
        },
    );

    Ok(SdScanResult {
        items,
        total_images: total,
        has_meta_count: has_meta,
        no_meta_count: no_meta,
        no_meta_files,
        source_counts,
        scan_time_ms: elapsed,
    })
}

// ── Export logic ──

fn export_sync(
    app: &tauri::AppHandle,
    options: &ExportTagsOptions,
) -> Result<ExportResult, String> {
    let total = options.items.len() as u32;
    let mut success = 0u32;
    let mut fail = 0u32;
    let mut skip = 0u32;
    let mut errors = Vec::new();

    for (i, item) in options.items.iter().enumerate() {
        if item.positive.trim().is_empty() {
            skip += 1;
            continue;
        }

        let src_path = Path::new(&item.source_path);
        let stem = src_path.file_stem().unwrap_or_default().to_string_lossy();

        let txt_path = match options.mode.as_str() {
            "custom" => {
                let dest = options.dest_folder.as_deref().unwrap_or(".");
                let mut dest_dir = Path::new(dest).to_path_buf();
                if let Some(root) = options.input_root.as_deref() {
                    let root_path = Path::new(root);
                    if root_path.is_dir() {
                        if let Ok(relative) = src_path.strip_prefix(root_path) {
                            if let Some(parent) = relative.parent() {
                                if !parent.as_os_str().is_empty() {
                                    dest_dir = dest_dir.join(parent);
                                }
                            }
                        }
                    }
                }
                if !dest_dir.exists() {
                    let _ = std::fs::create_dir_all(&dest_dir);
                }
                dest_dir.join(format!("{}.txt", stem))
            }
            _ => {
                // same path
                let parent = src_path.parent().unwrap_or(Path::new("."));
                parent.join(format!("{}.txt", stem))
            }
        };

        let _ = app.emit(
            "sd-metadata-progress",
            ProgressEvent {
                current: i as u32 + 1,
                total,
                filename: format!("{}.txt", stem),
                status: "processing".to_string(),
                message: format!("[{}/{}] {}.txt", i + 1, total, stem),
                ..Default::default()
            },
        );

        match std::fs::write(&txt_path, &item.positive) {
            Ok(_) => success += 1,
            Err(e) => {
                fail += 1;
                errors.push(format!("{}.txt: {}", stem, e));
            }
        }
    }

    let _ = app.emit(
        "sd-metadata-progress",
        ProgressEvent {
            current: total,
            total,
            filename: String::new(),
            status: "done".to_string(),
            message: format!("导出完成: 成功 {}, 失败 {}, 跳过 {}", success, fail, skip),
            ..Default::default()
        },
    );

    Ok(ExportResult {
        success_count: success,
        fail_count: fail,
        skip_count: skip,
        errors,
    })
}

// ── PNG tEXt chunk parser ──

struct RawMeta {
    positive: String,
    negative: String,
    params: String,
    artist: String,
    source: String,
}

fn read_png_metadata(path: &PathBuf) -> Option<RawMeta> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;

    // Verify PNG signature
    if buf.len() < 8 || &buf[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }

    let mut text_chunks: Vec<(String, String)> = Vec::new();
    let mut pos = 8usize;

    while pos + 12 <= buf.len() {
        let length =
            u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
        let chunk_type = &buf[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start + length;

        if data_end > buf.len() {
            break;
        }

        // tEXt chunk (uncompressed)
        if chunk_type == b"tEXt" {
            let data = &buf[data_start..data_end];
            if let Some(null_pos) = data.iter().position(|&b| b == 0) {
                let key = String::from_utf8_lossy(&data[..null_pos]).to_string();
                let value = String::from_utf8_lossy(&data[null_pos + 1..]).to_string();
                text_chunks.push((key, value));
            }
        }
        // zTXt chunk (compressed text — most A1111/ComfyUI images use this)
        else if chunk_type == b"zTXt" {
            let data = &buf[data_start..data_end];
            if let Some(null_pos) = data.iter().position(|&b| b == 0) {
                let key = String::from_utf8_lossy(&data[..null_pos]).to_string();
                // byte after null is compression method (0 = zlib), then compressed data
                if null_pos + 2 <= data.len() {
                    let compressed = &data[null_pos + 2..];
                    let mut decoder = ZlibDecoder::new(compressed);
                    let mut decompressed = String::new();
                    if decoder.read_to_string(&mut decompressed).is_ok() {
                        text_chunks.push((key, decompressed));
                    }
                }
            }
        }
        // iTXt chunk (UTF-8 international text)
        else if chunk_type == b"iTXt" {
            if let Some(meta) = parse_itxt_chunk(&buf[data_start..data_end]) {
                text_chunks.push(meta);
            }
        }

        // IEND = end of PNG
        if chunk_type == b"IEND" {
            break;
        }

        pos = data_end + 4; // skip CRC
    }

    if text_chunks.is_empty() {
        return None;
    }

    // Try A1111 format: key="parameters"
    for (key, value) in &text_chunks {
        if key == "parameters" && !value.is_empty() {
            return Some(parse_a1111(value));
        }
    }

    // Try ComfyUI format: key="prompt" (JSON)
    for (key, value) in &text_chunks {
        if key == "prompt" && value.trim_start().starts_with('{') {
            if let Some(meta) = parse_comfyui(value, &text_chunks) {
                return Some(meta);
            }
        }
    }

    // Try NovelAI format: key="Comment" (JSON) or "Description"
    for (key, value) in &text_chunks {
        if key == "Comment" && value.trim_start().starts_with('{') {
            let source_ver = text_chunks
                .iter()
                .find(|(k, _)| k == "Source")
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            if let Some(meta) = parse_novelai(value, source_ver) {
                return Some(meta);
            }
        }
    }

    // Fallback: "Description" key
    for (key, value) in &text_chunks {
        if key == "Description" && !value.is_empty() {
            return Some(RawMeta {
                positive: value.clone(),
                negative: String::new(),
                params: String::new(),
                artist: String::new(),
                source: "unknown".to_string(),
            });
        }
    }

    None
}

fn parse_itxt_chunk(data: &[u8]) -> Option<(String, String)> {
    // iTXt format:
    // keyword \0 compression_flag(1) compression_method(1) language_tag \0 translated_keyword \0 text
    let null1 = data.iter().position(|&b| b == 0)?;
    let key = String::from_utf8_lossy(&data[..null1]).to_string();

    let rest = &data[null1 + 1..];
    if rest.len() < 2 {
        return None;
    }

    let comp_flag = rest[0];
    let _comp_method = rest[1];
    let rest = &rest[2..];

    // language tag (may be empty) terminated by \0
    let null2 = rest.iter().position(|&b| b == 0)?;
    let rest = &rest[null2 + 1..];

    // translated keyword (may be empty) terminated by \0
    let null3 = rest.iter().position(|&b| b == 0)?;
    let text_data = &rest[null3 + 1..];

    let text = if comp_flag == 1 {
        // Compressed: decompress with zlib
        let mut decoder = ZlibDecoder::new(text_data);
        let mut decompressed = String::new();
        decoder.read_to_string(&mut decompressed).ok()?;
        decompressed
    } else {
        // Uncompressed
        String::from_utf8_lossy(text_data).to_string()
    };

    Some((key, text))
}

// ── Format parsers ──

fn parse_a1111(raw: &str) -> RawMeta {
    // Format:
    // positive prompt text
    // Negative prompt: negative text
    // Steps: 20, Sampler: Euler a, ...
    let positive;
    let mut negative = String::new();
    let mut params = String::new();

    if let Some(neg_idx) = raw.find("Negative prompt:") {
        positive = raw[..neg_idx].trim().to_string();
        let after_neg = &raw[neg_idx + "Negative prompt:".len()..];
        // Find the params line (starts with "Steps:")
        if let Some(steps_idx) = after_neg.find("\nSteps:") {
            negative = after_neg[..steps_idx].trim().to_string();
            params = after_neg[steps_idx + 1..].trim().to_string();
        } else {
            negative = after_neg.trim().to_string();
        }
    } else if let Some(steps_idx) = raw.find("\nSteps:") {
        positive = raw[..steps_idx].trim().to_string();
        params = raw[steps_idx + 1..].trim().to_string();
    } else {
        positive = raw.trim().to_string();
    }

    RawMeta {
        positive,
        negative,
        params,
        artist: String::new(),
        source: "a1111".to_string(),
    }
}

fn parse_comfyui(prompt_json: &str, _all_chunks: &[(String, String)]) -> Option<RawMeta> {
    // ComfyUI prompt JSON: { "node_id": { "class_type": "...", "inputs": { ... } } }
    let parsed: serde_json::Value = serde_json::from_str(prompt_json).ok()?;
    let obj = parsed.as_object()?;

    let mut positive_parts: Vec<String> = Vec::new();
    let mut negative_parts: Vec<String> = Vec::new();

    for (_node_id, node) in obj {
        let class_type = node.get("class_type")?.as_str().unwrap_or("");
        let inputs = node.get("inputs")?;

        // CLIPTextEncode is the standard prompt node
        if class_type == "CLIPTextEncode" {
            if let Some(text) = inputs.get("text").and_then(|t| t.as_str()) {
                if !text.trim().is_empty() {
                    // Try to determine if positive or negative by checking connections
                    // Simple heuristic: check if node connects to a "negative" conditioning
                    positive_parts.push(text.to_string());
                }
            }
        }
    }

    // Better heuristic: look for KSampler nodes to determine positive/negative
    for (_node_id, node) in obj {
        let class_type = node
            .get("class_type")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if class_type == "KSampler" || class_type == "KSamplerAdvanced" {
            let inputs = node.get("inputs")?;
            // positive input links to a node
            if let Some(pos_link) = inputs.get("positive").and_then(|v| v.as_array()) {
                if let Some(pos_node_id) = pos_link
                    .first()
                    .and_then(|v| v.as_str())
                    .or_else(|| pos_link.first().and_then(|v| v.as_u64()).map(|_| ""))
                {
                    let _ = pos_node_id; // we already collected all CLIPTextEncode
                }
            }
            if let Some(neg_link) = inputs.get("negative").and_then(|v| v.as_array()) {
                if let Some(neg_id) = neg_link.first() {
                    let neg_id_str = if neg_id.is_string() {
                        neg_id.as_str().unwrap_or("").to_string()
                    } else {
                        neg_id.to_string()
                    };
                    // Find the CLIPTextEncode node linked as negative
                    if let Some(neg_node) = obj.get(&neg_id_str) {
                        if let Some(text) = neg_node
                            .get("inputs")
                            .and_then(|i| i.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            // Move this text from positive to negative
                            positive_parts.retain(|p| p != text);
                            negative_parts.push(text.to_string());
                        }
                    }
                }
            }
        }
    }

    if positive_parts.is_empty() && negative_parts.is_empty() {
        return None;
    }

    Some(RawMeta {
        positive: positive_parts.join(", "),
        negative: negative_parts.join(", "),
        params: String::new(),
        artist: String::new(),
        source: "comfyui".to_string(),
    })
}

fn parse_novelai(comment: &str, source_ver: &str) -> Option<RawMeta> {
    let parsed: serde_json::Value = serde_json::from_str(comment).ok()?;
    let obj = parsed.as_object()?;

    // Try V4.5 nested prompt format first, fallback to legacy
    let positive = obj
        .get("v4_prompt")
        .and_then(|v| v.get("caption"))
        .and_then(|v| v.get("base_caption"))
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("prompt").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let negative = obj
        .get("v4_negative_prompt")
        .and_then(|v| v.get("caption"))
        .and_then(|v| v.get("base_caption"))
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("uc").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let mut param_parts = Vec::new();
    if !source_ver.is_empty() {
        // Strip trailing build hash (e.g. "NovelAI Diffusion V4.5 1229B44F" -> "NovelAI Diffusion V4.5")
        let clean_ver = source_ver
            .rsplit_once(' ')
            .filter(|(_, hash)| hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|(prefix, _)| prefix)
            .unwrap_or(source_ver);
        param_parts.push(format!("Model: {}", clean_ver));
    }
    if let Some(steps) = obj.get("steps").and_then(|v| v.as_u64()) {
        param_parts.push(format!("Steps: {}", steps));
    }
    if let Some(sampler) = obj.get("sampler").and_then(|v| v.as_str()) {
        param_parts.push(format!("Sampler: {}", sampler));
    }
    if let Some(scale) = obj.get("scale").and_then(|v| v.as_f64()) {
        param_parts.push(format!("CFG: {}", scale));
    }
    if let Some(cfg_rescale) = obj.get("cfg_rescale").and_then(|v| v.as_f64()) {
        if cfg_rescale > 0.0 {
            param_parts.push(format!("CFG Rescale: {}", cfg_rescale));
        }
    }
    if let Some(seed) = obj.get("seed").and_then(|v| v.as_u64()) {
        param_parts.push(format!("Seed: {}", seed));
    }
    if let (Some(w), Some(h)) = (
        obj.get("width").and_then(|v| v.as_u64()),
        obj.get("height").and_then(|v| v.as_u64()),
    ) {
        param_parts.push(format!("Size: {}x{}", w, h));
    }
    if let Some(noise) = obj.get("noise_schedule").and_then(|v| v.as_str()) {
        param_parts.push(format!("Noise: {}", noise));
    }
    // V4.5 char_captions from v4_prompt (画师串/角色串)
    let mut artist_parts: Vec<String> = Vec::new();
    if let Some(chars) = obj
        .get("v4_prompt")
        .and_then(|v| v.get("caption"))
        .and_then(|v| v.get("char_captions"))
        .and_then(|v| v.as_array())
    {
        for ch in chars {
            if let Some(cc) = ch.get("char_caption").and_then(|v| v.as_str()) {
                if !cc.is_empty() {
                    artist_parts.push(cc.to_string());
                }
            }
        }
    }
    // V4.5 director references
    if let Some(refs) = obj
        .get("director_reference_descriptions")
        .and_then(|v| v.as_array())
    {
        for r in refs {
            if let Some(cap) = r
                .get("caption")
                .and_then(|c| c.get("base_caption"))
                .and_then(|v| v.as_str())
            {
                if !cap.is_empty() {
                    param_parts.push(format!("Ref: {}", cap));
                }
            }
        }
    }

    if positive.is_empty() && artist_parts.is_empty() {
        return None;
    }

    // Merge artist/char captions into positive
    let full_positive = if artist_parts.is_empty() {
        positive
    } else {
        let mut parts = artist_parts;
        if !positive.is_empty() {
            parts.push(positive);
        }
        parts.join(", ")
    };

    Some(RawMeta {
        positive: full_positive,
        negative,
        params: param_parts.join(", "),
        artist: String::new(),
        source: "novelai".to_string(),
    })
}
