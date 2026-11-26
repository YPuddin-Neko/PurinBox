use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::batch::{BatchCounts, RunEvents};
use super::ProgressEvent;

const EVENT: &str = "sd-metadata-progress";

// ── Data types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdImageMeta {
    pub path: String,
    pub filename: String,
    pub positive: String,
    pub negative: String,
    pub params: String,
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
pub async fn scan_sd_metadata<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    input_path: String,
    recursive: Option<bool>,
) -> Result<SdScanResult, String> {
    tokio::task::spawn_blocking(move || scan_sync(&app, &input_path, recursive.unwrap_or(false)))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

#[tauri::command]
pub async fn export_sd_tags<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
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
    if !is_supported_metadata_ext(&ext) {
        return Ok(None);
    }
    let filename = super::file_name_lossy(&path);
    Ok(read_image_metadata(&path, &ext).map(|meta| meta.into_meta(file_path, filename)))
}

// ── Scan logic ──

fn scan_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    input_path: &str,
    recursive: bool,
) -> Result<SdScanResult, String> {
    let start = std::time::Instant::now();
    let dir = Path::new(input_path);
    let files = super::collect_image_files_with_recursive(dir, recursive)?;
    let total = files.len() as u32;
    let run = RunEvents::begin(app, EVENT);

    let mut items = Vec::new();
    let mut no_meta_files: Vec<String> = Vec::new();
    let mut has_meta = 0u32;
    let mut no_meta = 0u32;
    let mut source_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    for (i, file_path) in files.iter().enumerate() {
        let filename = super::file_name_lossy(file_path);

        run.emit(
            ProgressEvent::new("processing", format!("[{}/{}] {}", i + 1, total, filename))
                .at(i as u32 + 1, total)
                .file(filename.clone()),
        );

        let ext = file_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if !is_supported_metadata_ext(&ext) {
            no_meta += 1;
            no_meta_files.push(filename);
            continue;
        }

        match read_image_metadata(file_path, &ext) {
            Some(meta) => {
                *source_counts.entry(meta.source.clone()).or_insert(0) += 1;
                has_meta += 1;
                items.push(meta.into_meta(file_path.to_string_lossy().to_string(), filename));
            }
            None => {
                no_meta += 1;
                no_meta_files.push(filename);
            }
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    let counts = BatchCounts {
        success: has_meta,
        skipped: no_meta,
        total,
        ..Default::default()
    };
    run.finish(&counts, false, |c| {
        format!("扫描完成: {} 张图片, {} 有元数据", c.total, c.success)
    });

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

fn export_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &ExportTagsOptions,
) -> Result<ExportResult, String> {
    let run = RunEvents::begin(app, EVENT);
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
                let dest = Path::new(options.dest_folder.as_deref().unwrap_or("."));
                // 没有 input_root 时传空路径：它不是目录，不会拼相对子目录
                let root = Path::new(options.input_root.as_deref().unwrap_or(""));
                super::output_path_for_input(root, src_path, dest, &format!("{}.txt", stem), true)
            }
            _ => {
                // same path
                let parent = src_path.parent().unwrap_or(Path::new("."));
                Ok(parent.join(format!("{}.txt", stem)))
            }
        };

        run.emit(
            ProgressEvent::new("processing", format!("[{}/{}] {}.txt", i + 1, total, stem))
                .at(i as u32 + 1, total)
                .file(format!("{}.txt", stem)),
        );

        match txt_path
            .and_then(|path| std::fs::write(path, &item.positive).map_err(|e| e.to_string()))
        {
            Ok(_) => success += 1,
            Err(e) => {
                fail += 1;
                errors.push(format!("{}.txt: {}", stem, e));
            }
        }
    }

    let counts = BatchCounts {
        success,
        failed: fail,
        skipped: skip,
        total,
        ..Default::default()
    };
    run.finish(&counts, false, |c| {
        format!(
            "导出完成: 成功 {}, 失败 {}, 跳过 {}",
            c.success, c.failed, c.skipped
        )
    });

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
    source: String,
}

impl RawMeta {
    fn into_meta(self, path: String, filename: String) -> SdImageMeta {
        SdImageMeta {
            path,
            filename,
            positive: self.positive,
            negative: self.negative,
            params: self.params,
            source: self.source,
        }
    }
}

const MAX_EXIF_VALUE_BYTES: usize = 16 * 1024 * 1024;

fn is_supported_metadata_ext(ext: &str) -> bool {
    matches!(ext, "png" | "webp" | "jpg" | "jpeg")
}

fn read_image_metadata(path: &Path, ext: &str) -> Option<RawMeta> {
    match ext {
        "png" => read_png_metadata(path),
        "webp" => read_webp_metadata(path),
        "jpg" | "jpeg" => read_jpeg_metadata(path),
        _ => None,
    }
}

fn read_png_metadata(path: &Path) -> Option<RawMeta> {
    let png_chunks = super::sd_metadata_edit::read_png_metadata_chunks(path).ok()?;
    if let Some(meta) = parse_text_metadata(&png_chunks.text_chunks) {
        return Some(meta);
    }
    if !png_chunks.exif_blobs.is_empty() {
        let mut exif_chunks = Vec::new();
        for blob in &png_chunks.exif_blobs {
            parse_exif_to_chunks(blob, &mut exif_chunks);
        }
        return parse_text_metadata(&exif_chunks);
    }
    None
}

fn read_webp_metadata(path: &Path) -> Option<RawMeta> {
    let bytes = super::sd_metadata_edit::read_bytes(path).ok()?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut chunks = Vec::new();
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let fourcc = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let data_start = pos + 8;
        let Some(data_end) = data_start.checked_add(size) else {
            break;
        };
        if data_end > bytes.len() {
            break;
        }
        if fourcc == b"EXIF" {
            parse_exif_to_chunks(&bytes[data_start..data_end], &mut chunks);
        } else if fourcc == b"XMP " {
            parse_xmp_to_chunks(&bytes[data_start..data_end], &mut chunks);
        }
        let padded = size.checked_add(size & 1)?;
        let Some(next) = data_start.checked_add(padded) else {
            break;
        };
        pos = next;
    }
    parse_text_metadata(&chunks)
}

fn read_jpeg_metadata(path: &Path) -> Option<RawMeta> {
    let bytes = super::sd_metadata_edit::read_bytes(path).ok()?;
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut chunks = Vec::new();
    let mut pos = 2usize;
    while pos + 2 <= bytes.len() {
        if bytes[pos] != 0xFF {
            break;
        }
        while pos < bytes.len() && bytes[pos] == 0xFF {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let marker = bytes[pos];
        pos += 1;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if matches!(marker, 0xD0..=0xD8 | 0x01) {
            continue;
        }
        if pos + 2 > bytes.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > bytes.len() {
            break;
        }
        let payload = &bytes[pos + 2..pos + seg_len];
        if marker == 0xE1 {
            if let Some(exif) = payload.strip_prefix(b"Exif\0\0") {
                parse_exif_to_chunks(exif, &mut chunks);
            } else if let Some(xmp) = payload.strip_prefix(b"http://ns.adobe.com/xap/1.0/\0") {
                parse_xmp_to_chunks(xmp, &mut chunks);
            }
        } else if marker == 0xFE {
            let text = decode_text_bytes(payload);
            classify_exif_text(0xFE, &text, &mut chunks);
        }
        pos += seg_len;
    }
    parse_text_metadata(&chunks)
}

fn decode_text_bytes(bytes: &[u8]) -> String {
    let trimmed = match bytes.iter().rposition(|&b| b != 0) {
        Some(end) => &bytes[..=end],
        None => &[],
    };
    String::from_utf8(trimmed.to_vec())
        .unwrap_or_else(|_| trimmed.iter().map(|b| char::from(*b)).collect())
}

fn decode_utf16_bytes(bytes: &[u8], default_le: bool) -> String {
    let (data, little_endian) = if bytes.starts_with(&[0xFE, 0xFF]) {
        (&bytes[2..], false)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (&bytes[2..], true)
    } else {
        let sample_len = bytes.len().min(64) & !1;
        let mut even_zeros = 0usize;
        let mut odd_zeros = 0usize;
        for pair in bytes[..sample_len].chunks_exact(2) {
            if pair[0] == 0 && pair[1] != 0 {
                even_zeros += 1;
            } else if pair[1] == 0 && pair[0] != 0 {
                odd_zeros += 1;
            }
        }
        let le = if even_zeros > odd_zeros {
            false
        } else if odd_zeros > even_zeros {
            true
        } else {
            default_le
        };
        (bytes, le)
    };
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    let trimmed = match units.iter().rposition(|&u| u != 0) {
        Some(end) => &units[..=end],
        None => &[],
    };
    String::from_utf16_lossy(trimmed)
}

fn decode_exif_user_comment(raw: &[u8], tiff_le: bool) -> String {
    if let Some(rest) = raw.strip_prefix(b"UNICODE\0") {
        decode_utf16_bytes(rest, tiff_le)
    } else if let Some(rest) = raw.strip_prefix(b"ASCII\0\0\0") {
        decode_text_bytes(rest)
    } else if let Some(rest) = raw.strip_prefix(b"\0\0\0\0\0\0\0\0") {
        decode_text_bytes(rest)
    } else {
        decode_text_bytes(raw)
    }
}

fn strip_ascii_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

fn classify_exif_text(tag: u16, raw: &str, out: &mut Vec<(String, String)>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some(rest) = strip_ascii_prefix(trimmed, "prompt:") {
        let val = rest.trim();
        if !val.is_empty() {
            out.push(("prompt".to_string(), val.to_string()));
        }
        return;
    }
    if let Some(rest) = strip_ascii_prefix(trimmed, "workflow:") {
        let val = rest.trim();
        if !val.is_empty() {
            out.push(("workflow".to_string(), val.to_string()));
        }
        return;
    }
    if let Some(rest) = strip_ascii_prefix(trimmed, "parameters:") {
        let val = rest.trim();
        if !val.is_empty() {
            out.push(("parameters".to_string(), val.to_string()));
        }
        return;
    }
    if trimmed.starts_with('{') {
        if let Some(parsed) = parse_json_lenient(trimmed) {
            if let Some(obj) = parsed.as_object() {
                if is_comfyui_prompt_graph(obj)
                    || obj
                        .get("prompt")
                        .and_then(|v| v.as_object())
                        .is_some_and(is_comfyui_prompt_graph)
                {
                    out.push(("prompt".to_string(), trimmed.to_string()));
                    return;
                }
                if obj.get("nodes").and_then(|v| v.as_array()).is_some() {
                    out.push(("workflow".to_string(), trimmed.to_string()));
                    return;
                }
                if obj.contains_key("v4_prompt")
                    || obj.contains_key("prompt")
                    || obj.contains_key("uc")
                {
                    out.push(("Comment".to_string(), trimmed.to_string()));
                    return;
                }
            }
        }
    }
    if matches!(tag, 0x9286 | 0x9C9C | 0x010E | 0xFE)
        && (trimmed.contains("Steps:") || trimmed.contains("Negative prompt:"))
    {
        out.push(("parameters".to_string(), trimmed.to_string()));
    }
}

fn parse_exif_to_chunks(raw: &[u8], out: &mut Vec<(String, String)>) {
    let tiff = raw.strip_prefix(b"Exif\0\0").unwrap_or(raw);
    if tiff.len() < 8 {
        return;
    }
    let little_endian = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return,
    };
    let read_u16 = |off: usize| -> Option<u16> {
        let s: [u8; 2] = tiff.get(off..off + 2)?.try_into().ok()?;
        Some(if little_endian {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let read_u32 = |off: usize| -> Option<u32> {
        let s: [u8; 4] = tiff.get(off..off + 4)?.try_into().ok()?;
        Some(if little_endian {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    };
    if read_u16(2) != Some(42) {
        return;
    }
    let Some(ifd0_offset) = read_u32(4).map(|v| v as usize) else {
        return;
    };

    let mut queue = vec![(ifd0_offset, false)];
    let mut visited = HashSet::new();
    while let Some((offset, is_exif_subifd)) = queue.pop() {
        if !visited.insert(offset) {
            continue;
        }
        let Some(count) = read_u16(offset).map(|c| (c as usize).min(256)) else {
            continue;
        };
        for i in 0..count {
            let entry_off = offset + 2 + i * 12;
            if entry_off + 12 > tiff.len() {
                break;
            }
            let Some(tag) = read_u16(entry_off) else {
                continue;
            };
            let Some(field_type) = read_u16(entry_off + 2) else {
                continue;
            };
            let Some(value_count) = read_u32(entry_off + 4).map(|v| v as usize) else {
                continue;
            };

            if !is_exif_subifd && tag == 0x8769 && value_count == 1 {
                if let Some(sub_off) = read_u32(entry_off + 8).map(|v| v as usize) {
                    queue.push((sub_off, true));
                }
                continue;
            }

            let wanted = if is_exif_subifd {
                tag == 0x9286
            } else {
                matches!(tag, 0x010D | 0x010E | 0x010F | 0x0131 | 0x013B | 0x9C9C)
            };
            if !wanted {
                continue;
            }

            let type_size = match field_type {
                1 | 2 | 6 | 7 => 1usize,
                3 | 8 => 2,
                4 | 9 | 11 => 4,
                5 | 10 | 12 => 8,
                _ => continue,
            };
            let Some(byte_len) = value_count.checked_mul(type_size) else {
                continue;
            };
            if byte_len == 0 || byte_len > MAX_EXIF_VALUE_BYTES {
                continue;
            }
            let data = if byte_len <= 4 {
                &tiff[entry_off + 8..entry_off + 8 + byte_len]
            } else {
                let Some(data_off) = read_u32(entry_off + 8).map(|v| v as usize) else {
                    continue;
                };
                let Some(end) = data_off.checked_add(byte_len) else {
                    continue;
                };
                if end > tiff.len() {
                    continue;
                }
                &tiff[data_off..end]
            };

            let decoded = match tag {
                0x9286 => decode_exif_user_comment(data, little_endian),
                0x9C9C => decode_utf16_bytes(data, true),
                _ => decode_text_bytes(data),
            };
            classify_exif_text(tag, &decoded, out);
        }
    }
}

fn unescape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        if let Some(semi) = after.find(';') {
            let entity = &after[..semi];
            let replaced = match entity {
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    u32::from_str_radix(&entity[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if entity.starts_with('#') => entity[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32),
                _ => None,
            };
            if let Some(ch) = replaced {
                out.push(ch);
                rest = &after[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn parse_xmp_to_chunks(raw: &[u8], out: &mut Vec<(String, String)>) {
    let text = String::from_utf8_lossy(raw);
    for segment in text.split('>') {
        let body = segment.split('<').next().unwrap_or("").trim();
        if body.len() < 8 {
            continue;
        }
        if strip_ascii_prefix(body, "prompt:").is_some()
            || strip_ascii_prefix(body, "workflow:").is_some()
            || strip_ascii_prefix(body, "parameters:").is_some()
        {
            let unescaped = unescape_xml(body);
            classify_exif_text(0x010E, &unescaped, out);
        }
    }
}

pub(super) fn summarize_text_metadata(
    path: &Path,
    text_chunks: &[(String, String)],
) -> Option<SdImageMeta> {
    parse_text_metadata(text_chunks).map(|meta| {
        meta.into_meta(
            path.to_string_lossy().into_owned(),
            super::file_name_lossy(path),
        )
    })
}

fn parse_text_metadata(text_chunks: &[(String, String)]) -> Option<RawMeta> {
    if text_chunks.is_empty() {
        return None;
    }

    // Try A1111 format: key="parameters"
    for (key, value) in text_chunks {
        if key.eq_ignore_ascii_case("parameters") && !value.trim().is_empty() {
            return Some(parse_a1111(value));
        }
    }

    let workflow_json = text_chunks
        .iter()
        .find(|(k, v)| k.eq_ignore_ascii_case("workflow") && v.trim_start().starts_with('{'))
        .map(|(_, v)| v.as_str());

    // Try ComfyUI format: key="prompt" (JSON)
    for (key, value) in text_chunks {
        if key.eq_ignore_ascii_case("prompt") && value.trim_start().starts_with('{') {
            if let Some(mut meta) = parse_comfyui_with_workflow(value, workflow_json) {
                if meta.positive.is_empty() && meta.negative.is_empty() {
                    if let Some(wf) = workflow_json {
                        if let Some(wf_meta) = parse_comfyui_workflow(wf) {
                            if !wf_meta.positive.is_empty() {
                                meta.positive = wf_meta.positive;
                            }
                            if !wf_meta.negative.is_empty() {
                                meta.negative = wf_meta.negative;
                            }
                            if meta.params.is_empty() && !wf_meta.params.is_empty() {
                                meta.params = wf_meta.params;
                            }
                        }
                    }
                }
                return Some(meta);
            }
        }
    }

    // Try NovelAI format: key="Comment" (JSON)
    for (key, value) in text_chunks {
        if key.eq_ignore_ascii_case("Comment") && value.trim_start().starts_with('{') {
            let source_ver = text_chunks
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("Source"))
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            if let Some(meta) = parse_novelai(value, source_ver) {
                return Some(meta);
            }
        }
    }

    // Try ComfyUI workflow-only fallback: key="workflow" (JSON)
    for (key, value) in text_chunks {
        if key.eq_ignore_ascii_case("workflow") && value.trim_start().starts_with('{') {
            if let Some(meta) = parse_comfyui_workflow(value) {
                return Some(meta);
            }
        }
    }

    // Fallback: "Description" key
    for (key, value) in text_chunks {
        if key.eq_ignore_ascii_case("Description") && !value.trim().is_empty() {
            return Some(RawMeta {
                positive: value.clone(),
                negative: String::new(),
                params: String::new(),
                source: "unknown".to_string(),
            });
        }
    }

    None
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
        source: "a1111".to_string(),
    }
}

fn sanitize_nonstandard_json(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            let ch = raw[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if prev_ok {
            let rest = &raw[i..];
            let matched = if rest.starts_with("-Infinity") || rest.starts_with("+Infinity") {
                Some(9)
            } else if rest.starts_with("Infinity") {
                Some(8)
            } else if rest.starts_with("-NaN") {
                Some(4)
            } else if rest.starts_with("NaN") {
                Some(3)
            } else {
                None
            };
            if let Some(len) = matched {
                let end = i + len;
                let next_ok = end == bytes.len()
                    || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
                if next_ok {
                    out.push_str("null");
                    i = end;
                    continue;
                }
            }
        }
        let ch = raw[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn parse_json_lenient(raw: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return Some(v);
    }
    let sanitized = sanitize_nonstandard_json(raw);
    serde_json::from_str::<Value>(&sanitized).ok()
}

fn is_comfyui_prompt_graph(obj: &Map<String, Value>) -> bool {
    obj.values().any(|v| {
        v.as_object()
            .and_then(|node| node.get("class_type"))
            .and_then(|c| c.as_str())
            .is_some()
    })
}

fn push_unique(vec: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() && !vec.iter().any(|existing| existing == trimmed) {
        vec.push(trimmed.to_string());
    }
}

fn push_param(params: &mut Vec<String>, label: &str, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        let part = format!("{label}: {trimmed}");
        if !params.iter().any(|p| p == &part) {
            params.push(part);
        }
    }
}

fn link_target(value: &Value) -> Option<(String, Option<u64>)> {
    let arr = value.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    let slot = arr[1].as_u64()?;
    let id = match &arr[0] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Some((id, Some(slot)))
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

struct ComfyContext<'a> {
    prompt: &'a Map<String, Value>,
    workflow_nodes: HashMap<String, &'a Value>,
}

impl<'a> ComfyContext<'a> {
    fn new(prompt: &'a Map<String, Value>, workflow: Option<&'a Value>) -> Self {
        let mut workflow_nodes = HashMap::new();
        if let Some(wf) = workflow.and_then(|v| v.as_object()) {
            index_workflow_nodes(wf, &mut workflow_nodes);
            if let Some(subgraphs) = wf
                .get("definitions")
                .and_then(|d| d.get("subgraphs"))
                .and_then(|s| s.as_array())
            {
                for sg in subgraphs {
                    if let Some(sg_obj) = sg.as_object() {
                        index_workflow_nodes(sg_obj, &mut workflow_nodes);
                    }
                }
            }
        }
        Self {
            prompt,
            workflow_nodes,
        }
    }

    fn workflow_node(&self, node_id: &str) -> Option<&'a Value> {
        self.workflow_nodes.get(node_id).copied().or_else(|| {
            node_id
                .rsplit_once(':')
                .and_then(|(_, suffix)| self.workflow_nodes.get(suffix).copied())
        })
    }
}

fn index_workflow_nodes<'a>(graph: &'a Map<String, Value>, out: &mut HashMap<String, &'a Value>) {
    let Some(nodes) = graph.get("nodes").and_then(|v| v.as_array()) else {
        return;
    };
    for node in nodes {
        if let Some(id) = workflow_node_id(node) {
            out.entry(id).or_insert(node);
        }
    }
}

fn scalar_key_aliases(preferred: &str) -> &'static [&'static str] {
    match preferred {
        "steps" => &["steps", "steps_total", "step"],
        "seed" | "noise_seed" => &["seed", "noise_seed"],
        "cfg" => &["cfg", "cfg_scale", "cfg_conds"],
        "sampler_name" | "sampler" => &["sampler_name", "sampler"],
        "scheduler" => &["scheduler"],
        "denoise" => &["denoise"],
        "guidance" => &["guidance"],
        "width" => &["width", "empty_latent_width"],
        "height" => &["height", "empty_latent_height"],
        _ => &[],
    }
}

fn resolve_workflow_panel_slot(
    ctx: &ComfyContext,
    panel_id: &str,
    slot: u64,
) -> Option<String> {
    let wf_node = ctx.workflow_node(panel_id)?;
    let params = wf_node
        .get("properties")
        .and_then(|p| p.get("parameters"))
        .and_then(|v| v.as_array())?;
    let value_entries: Vec<&Value> = params
        .iter()
        .filter(|entry| {
            entry
                .get("type")
                .and_then(|t| t.as_str())
                .is_none_or(|t| !t.eq_ignore_ascii_case("separator"))
        })
        .collect();
    let item = value_entries.get(slot as usize)?;
    scalar_string(item.get("value")?)
}

fn resolve_scalar(
    ctx: &ComfyContext,
    value: &Value,
    preferred_key: Option<&str>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Option<String> {
    if depth > 16 {
        return None;
    }
    if let Some(s) = scalar_string(value) {
        return Some(s);
    }
    let (target_id, slot) = link_target(value)?;
    let visit_key = format!(
        "{target_id}:{}:{}",
        slot.unwrap_or(0),
        preferred_key.unwrap_or("")
    );
    if !visited.insert(visit_key.clone()) {
        return None;
    }
    let res = resolve_scalar_from_node(ctx, &target_id, slot, preferred_key, depth + 1, visited);
    visited.remove(&visit_key);
    res
}

fn resolve_scalar_from_node(
    ctx: &ComfyContext,
    target_id: &str,
    slot: Option<u64>,
    preferred_key: Option<&str>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Option<String> {
    let node = ctx.prompt.get(target_id)?;
    let class_type = node
        .get("class_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lower = class_type.to_ascii_lowercase();
    let inputs = node.get("inputs").and_then(|i| i.as_object())?;

    // YP_ParameterBreak / custom parameter unpackers backed by workflow panel properties
    if lower.contains("parameterbreak") || inputs.contains_key("parameters") {
        if let Some(slot_idx) = slot {
            if let Some((panel_id, _)) = inputs.get("parameters").and_then(link_target) {
                if let Some(val) = resolve_workflow_panel_slot(ctx, &panel_id, slot_idx) {
                    return Some(val);
                }
            }
            if let Some(val) = resolve_workflow_panel_slot(ctx, target_id, slot_idx) {
                return Some(val);
            }
        }
    }

    // Check preferred key & its aliases first (crucial for multi-output nodes like width/height or KSampler Config)
    if let Some(pref) = preferred_key {
        for &alias in scalar_key_aliases(pref) {
            if let Some(child) = inputs.get(alias) {
                if let Some(resolved) = resolve_scalar(ctx, child, Some(pref), depth, visited) {
                    return Some(resolved);
                }
            }
        }
        if let Some(child) = inputs.get(pref) {
            if let Some(resolved) = resolve_scalar(ctx, child, Some(pref), depth, visited) {
                return Some(resolved);
            }
        }
    }

    // rgthree Context / Context Switch / Context Merge bus traversal
    if lower.contains("context") {
        for ctx_key in ["ctx_01", "ctx_02", "ctx_03", "base_ctx", "context", "ctx"] {
            if let Some(child) = inputs.get(ctx_key) {
                if let Some(resolved) = resolve_scalar(ctx, child, preferred_key, depth, visited) {
                    return Some(resolved);
                }
            }
        }
    }

    for key in [
        "value",
        "seed",
        "noise_seed",
        "int",
        "float",
        "number",
        "steps",
        "steps_total",
        "cfg",
        "guidance",
        "sampler_name",
        "scheduler",
        "denoise",
        "width",
        "height",
        "ckpt_name",
        "unet_name",
        "model_name",
        "string",
        "text",
    ] {
        if let Some(child) = inputs.get(key) {
            if let Some(resolved) = resolve_scalar(ctx, child, preferred_key, depth, visited) {
                return Some(resolved);
            }
        }
    }

    None
}

fn resolve_text(
    obj: &Map<String, Value>,
    value: &Value,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Option<String> {
    if depth > 24 {
        return None;
    }
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    let (target_id, slot) = link_target(value)?;
    if !visited.insert(target_id.clone()) {
        return None;
    }
    let result = resolve_text_from_node(obj, &target_id, slot, depth + 1, visited);
    visited.remove(&target_id);
    result
}

fn join_prompt_parts(parts: &[String], delimiter: &str) -> String {
    let delim_trim = delimiter.trim();
    let sep = if delim_trim == "," { ", " } else { delimiter };
    let mut acc = String::new();
    for part in parts {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if acc.is_empty() {
            acc.push_str(p);
            continue;
        }
        if !delim_trim.is_empty() {
            let trimmed_end = acc.trim_end();
            if trimmed_end.ends_with(delim_trim) {
                acc.truncate(trimmed_end.len());
                if delim_trim == "," {
                    acc.push(' ');
                }
                acc.push_str(p.trim_start_matches(delim_trim).trim_start());
                continue;
            }
        }
        acc.push_str(sep);
        acc.push_str(p);
    }
    acc
}

fn extract_toggle_trigger_words(inputs: &Map<String, Value>) -> Option<String> {
    let items = inputs
        .get("toggle_trigger_words")
        .and_then(|v| v.get("__value__"))
        .and_then(|v| v.as_array())?;
    let mut active = Vec::new();
    for item in items {
        if item.get("active").and_then(|v| v.as_bool()) == Some(true) {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                push_unique(&mut active, text);
            }
        }
    }
    if active.is_empty() {
        None
    } else {
        Some(active.join(", "))
    }
}

fn resolve_text_from_node(
    obj: &Map<String, Value>,
    node_id: &str,
    slot: Option<u64>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Option<String> {
    let node = obj.get(node_id)?;
    let class_type = node
        .get("class_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lower = class_type.to_ascii_lowercase();
    let inputs = node.get("inputs").and_then(|v| v.as_object())?;

    // LoraManager TriggerWord Toggle: prefer only active trigger words
    if inputs.contains_key("toggle_trigger_words") {
        return extract_toggle_trigger_words(inputs);
    }

    // String replace nodes
    if lower.contains("replace") && (lower.contains("string") || lower.contains("text")) {
        let base = ["string", "text", "input"]
            .iter()
            .find_map(|k| inputs.get(*k).and_then(|v| resolve_text(obj, v, depth, visited)));
        if let Some(base) = base {
            let find = ["find", "target", "search"]
                .iter()
                .find_map(|k| inputs.get(*k).and_then(|v| resolve_text(obj, v, depth, visited)))
                .unwrap_or_default();
            let repl = ["replace", "replacement"]
                .iter()
                .find_map(|k| inputs.get(*k).and_then(|v| resolve_text(obj, v, depth, visited)))
                .unwrap_or_default();
            return Some(if find.is_empty() {
                base
            } else {
                base.replace(&find, &repl)
            });
        }
    }

    // String concatenate / join / combine nodes
    if (lower.contains("string") || lower.contains("text") || lower.contains("prompt"))
        && (lower.contains("concat")
            || lower.contains("join")
            || lower.contains("combine")
            || lower.contains("append")
            || lower.contains("merge"))
    {
        let delimiter = ["delimiter", "separator", "sep"]
            .iter()
            .find_map(|k| inputs.get(*k).and_then(|v| resolve_text(obj, v, depth, visited)))
            .unwrap_or_else(|| {
                if lower.contains("prompt") {
                    ", ".to_string()
                } else {
                    String::new()
                }
            });
        let ordered_keys = [
            "prefix",
            "text_prefix",
            "prompt1",
            "prompt2",
            "prompt3",
            "prompt4",
            "prompt_1",
            "prompt_2",
            "prompt_3",
            "prompt_4",
            "string_a",
            "string_b",
            "string_c",
            "string_d",
            "text_a",
            "text_b",
            "text_c",
            "text_d",
            "text_1",
            "text_2",
            "text_3",
            "text_4",
            "text1",
            "text2",
            "text3",
            "text4",
            "part1",
            "part2",
            "part3",
            "part4",
            "input_a",
            "input_b",
            "a",
            "b",
            "c",
            "d",
            "text",
            "prompt",
            "suffix",
            "text_suffix",
        ];
        let mut seen_keys = HashSet::new();
        let mut parts = Vec::new();
        for key in ordered_keys {
            if let Some(val) = inputs.get(key) {
                seen_keys.insert(key);
                if let Some(resolved) = resolve_text(obj, val, depth, visited) {
                    if !resolved.trim().is_empty() {
                        parts.push(resolved);
                    }
                }
            }
        }
        // Catch dynamic numbered inputs like string_1, text_5, etc.
        for (key, val) in inputs {
            if seen_keys.contains(key.as_str()) {
                continue;
            }
            let kl = key.to_ascii_lowercase();
            if kl.starts_with("string") || kl.starts_with("text") || kl.starts_with("part") || kl.starts_with("prompt") {
                if let Some(resolved) = resolve_text(obj, val, depth, visited) {
                    if !resolved.trim().is_empty() {
                        parts.push(resolved);
                    }
                }
            }
        }
        if !parts.is_empty() {
            return Some(join_prompt_parts(&parts, &delimiter));
        }
    }

    // Switch / selector nodes
    if lower.contains("switch") {
        if let Some(b) = inputs.get("boolean").and_then(|v| v.as_bool()) {
            let keys = if b {
                ["on_true", "true", "text_true", "a"]
            } else {
                ["on_false", "false", "text_false", "b"]
            };
            for k in keys {
                if let Some(v) = inputs.get(k) {
                    if let Some(s) = resolve_text(obj, v, depth, visited) {
                        if !s.trim().is_empty() {
                            return Some(s);
                        }
                    }
                }
            }
        }
        if let Some(idx) = inputs
            .get("select")
            .or_else(|| inputs.get("index"))
            .and_then(|v| v.as_u64())
        {
            for candidate in [
                format!("input{idx}"),
                format!("input_{idx}"),
                format!("text_{idx}"),
                format!("string_{idx}"),
                format!("any_{idx:02}"),
                format!("value{idx}"),
            ] {
                if let Some(v) = inputs.get(&candidate) {
                    if let Some(s) = resolve_text(obj, v, depth, visited) {
                        if !s.trim().is_empty() {
                            return Some(s);
                        }
                    }
                }
            }
        }
        for key in [
            "any_01", "any_02", "input1", "input2", "on_true", "on_false", "a", "b", "text_a",
            "text_b",
        ] {
            if let Some(v) = inputs.get(key) {
                if let Some(s) = resolve_text(obj, v, depth, visited) {
                    if !s.trim().is_empty() {
                        return Some(s);
                    }
                }
            }
        }
    }

    // Dual positive/negative styler nodes
    if slot == Some(1) {
        for key in ["text_negative", "negative"] {
            if let Some(v) = inputs.get(key) {
                if let Some(s) = resolve_text(obj, v, depth, visited) {
                    return Some(s);
                }
            }
        }
    } else if slot == Some(0) && inputs.contains_key("text_positive") {
        if let Some(s) = resolve_text(obj, &inputs["text_positive"], depth, visited) {
            return Some(s);
        }
    }

    // Standard text / primitive / wildcard / passthrough keys
    for key in [
        "populated_text",
        "selected_prompts",
        "text",
        "string",
        "value",
        "prompt",
        "STRING",
        "Text",
        "Prompt",
        "text_0",
        "text_1",
        "wildcard_text",
        "caption",
        "content",
        "multiline",
        "text_multiline",
        "raw_text",
        "input",
        "any",
        "text_positive",
        "positive",
    ] {
        if let Some(v) = inputs.get(key) {
            if let Some(s) = resolve_text(obj, v, depth, visited) {
                if !s.trim().is_empty() {
                    return Some(s);
                }
            }
        }
    }

    // Fallback: inspect remaining string or link inputs that are not control parameters
    for (key, val) in inputs {
        let kl = key.to_ascii_lowercase();
        if matches!(
            kl.as_str(),
            "mode"
                | "action"
                | "seed"
                | "noise_seed"
                | "steps"
                | "cfg"
                | "denoise"
                | "delimiter"
                | "separator"
                | "sep"
                | "strip_newlines"
                | "model"
                | "clip"
                | "vae"
                | "latent_image"
                | "samples"
                | "image"
                | "mask"
                | "ckpt_name"
                | "unet_name"
                | "lora_name"
                | "sampler_name"
                | "scheduler"
                | "filename_prefix"
                | "control_after_generate"
                | "add_noise"
                | "return_with_leftover_noise"
        ) {
            continue;
        }
        if val.is_string() || link_target(val).is_some() {
            if let Some(s) = resolve_text(obj, val, depth, visited) {
                if !s.trim().is_empty() {
                    return Some(s);
                }
            }
        }
    }

    None
}

fn is_text_encoder_class(lower: &str) -> bool {
    lower.contains("cliptextencode")
        || lower.contains("textencode")
        || lower.starts_with("power prompt")
        || lower.contains("promptencode")
        || lower.contains("encodeprompt")
        || lower.contains("wildcardencode")
}

fn is_negative_title(node: &Value) -> bool {
    let title = node
        .get("_meta")
        .and_then(|m| m.get("title"))
        .and_then(|t| t.as_str())
        .or_else(|| node.get("title").and_then(|t| t.as_str()))
        .unwrap_or("");
    let lower = title.to_ascii_lowercase();
    lower.contains("negative")
        || lower.contains("neg prompt")
        || lower == "neg"
        || title.contains("反向")
        || title.contains("负面")
        || title.contains("负向")
}

fn extract_encoder_texts(
    ctx: &ComfyContext,
    node_id: &str,
    node: &Value,
    out: &mut Vec<String>,
    visited_encoders: &mut HashSet<String>,
    params: &mut Vec<String>,
) -> bool {
    let Some(class_type) = node.get("class_type").and_then(|v| v.as_str()) else {
        return false;
    };
    let lower = class_type.to_ascii_lowercase();
    if !is_text_encoder_class(&lower) {
        return false;
    }
    visited_encoders.insert(node_id.to_string());
    let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
        return true;
    };

    if let Some(g) = inputs
        .get("guidance")
        .and_then(|v| resolve_scalar(ctx, v, Some("guidance"), 0, &mut HashSet::new()))
    {
        push_param(params, "Guidance", &g);
    }

    let obj = ctx.prompt;
    let mut found = false;
    // Lumina2 has system_prompt + user_prompt; prefer user_prompt when present
    if let Some(user_prompt) = inputs.get("user_prompt") {
        if let Some(s) = resolve_text(obj, user_prompt, 0, &mut HashSet::new()) {
            if !s.trim().is_empty() {
                push_unique(out, &s);
                found = true;
            }
        }
    }

    for key in [
        "populated_text",
        "text",
        "prompt",
        "t5xxl",
        "clip_l",
        "clip_g",
        "llama",
        "text_g",
        "text_l",
        "bert",
        "mt5xl",
        "wildcard_text",
    ] {
        if let Some(val) = inputs.get(key) {
            if let Some(s) = resolve_text(obj, val, 0, &mut HashSet::new()) {
                if !s.trim().is_empty() {
                    push_unique(out, &s);
                    found = true;
                }
            }
        }
    }

    if !found {
        if let Some(sys) = inputs.get("system_prompt") {
            if let Some(s) = resolve_text(obj, sys, 0, &mut HashSet::new()) {
                push_unique(out, &s);
            }
        }
    }

    true
}

fn trace_conditioning(
    ctx: &ComfyContext,
    value: &Value,
    prefer_negative: bool,
    out: &mut Vec<String>,
    visited_slots: &mut HashSet<String>,
    visited_encoders: &mut HashSet<String>,
    params: &mut Vec<String>,
    depth: usize,
) {
    if depth > 24 {
        return;
    }
    if let Some(s) = value.as_str() {
        push_unique(out, s);
        return;
    }
    let Some((target_id, slot)) = link_target(value) else {
        return;
    };
    let visit_key = format!("{target_id}:{}:{}", slot.unwrap_or(0), prefer_negative);
    if !visited_slots.insert(visit_key) {
        return;
    }
    let obj = ctx.prompt;
    let Some(node) = obj.get(&target_id) else {
        return;
    };
    let class_type = node
        .get("class_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lower = class_type.to_ascii_lowercase();

    // ConditioningZeroOut deliberately clears conditioning (common for Flux/Qwen negative)
    if lower == "conditioningzeroout" || lower.contains("zeroout") {
        return;
    }

    if extract_encoder_texts(ctx, &target_id, node, out, visited_encoders, params) {
        return;
    }

    let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
        return;
    };

    if lower.contains("fluxguidance") {
        if let Some(g) = inputs
            .get("guidance")
            .and_then(|v| resolve_scalar(ctx, v, Some("guidance"), 0, &mut HashSet::new()))
        {
            push_param(params, "Guidance", &g);
        }
    }

    // Impact FromBasicPipe outputs: 0=model, 1=clip, 2=vae, 3=positive, 4=negative
    if lower.contains("frombasicpipe") {
        if let Some(pipe_val) = inputs.get("basic_pipe") {
            if let Some((pipe_id, _)) = link_target(pipe_val) {
                if let Some(pipe_inputs) = obj
                    .get(&pipe_id)
                    .and_then(|n| n.get("inputs"))
                    .and_then(|i| i.as_object())
                {
                    let pick_neg = slot == Some(4) || prefer_negative;
                    let key = if pick_neg { "negative" } else { "positive" };
                    if let Some(next) = pipe_inputs.get(key) {
                        trace_conditioning(
                            ctx,
                            next,
                            pick_neg,
                            out,
                            visited_slots,
                            visited_encoders,
                            params,
                            depth + 1,
                        );
                    }
                }
            }
        }
        return;
    }

    // Loader / Context / dual-conditioning nodes that carry both positive and negative
    if inputs.contains_key("positive") && inputs.contains_key("negative") {
        let pick_negative = if lower.contains("context") {
            prefer_negative || slot == Some(5)
        } else if lower.contains("loader") {
            slot == Some(2)
        } else {
            slot == Some(1)
        };
        let key = if pick_negative { "negative" } else { "positive" };
        if let Some(next) = inputs.get(key) {
            trace_conditioning(
                ctx,
                next,
                pick_negative,
                out,
                visited_slots,
                visited_encoders,
                params,
                depth + 1,
            );
        }
        return;
    }

    // rgthree Context Switch / Context Merge bus nodes
    if lower.contains("context") {
        let branch_neg = prefer_negative || slot == Some(5);
        for ctx_key in ["ctx_01", "ctx_02", "ctx_03", "base_ctx", "context", "ctx"] {
            if let Some(next) = inputs.get(ctx_key) {
                if link_target(next).is_some() {
                    trace_conditioning(
                        ctx,
                        next,
                        branch_neg,
                        out,
                        visited_slots,
                        visited_encoders,
                        params,
                        depth + 1,
                    );
                }
            }
        }
        return;
    }

    // Switch / Reroute conditioning nodes
    if lower.contains("switch") {
        if let Some(b) = inputs.get("boolean").and_then(|v| v.as_bool()) {
            let key = if b { "on_true" } else { "on_false" };
            if let Some(next) = inputs.get(key) {
                trace_conditioning(
                    ctx,
                    next,
                    prefer_negative,
                    out,
                    visited_slots,
                    visited_encoders,
                    params,
                    depth + 1,
                );
                return;
            }
        }
        if let Some(idx) = inputs
            .get("select")
            .or_else(|| inputs.get("index"))
            .and_then(|v| v.as_u64())
        {
            for candidate in [
                format!("input{idx}"),
                format!("input_{idx}"),
                format!("conditioning_{idx}"),
                format!("any_{idx:02}"),
            ] {
                if let Some(next) = inputs.get(&candidate) {
                    trace_conditioning(
                        ctx,
                        next,
                        prefer_negative,
                        out,
                        visited_slots,
                        visited_encoders,
                        params,
                        depth + 1,
                    );
                    return;
                }
            }
        }
    }

    let mut followed = false;
    for key in [
        "conditioning",
        "conditioning_1",
        "conditioning_2",
        "conditioning_to",
        "conditioning_from",
        "cond",
        "cond_1",
        "cond_2",
        "base_conditioning",
        "input",
        "any",
        "any_01",
        "any_02",
        "input1",
        "input2",
        "on_true",
        "on_false",
    ] {
        if let Some(next) = inputs.get(key) {
            if link_target(next).is_some() {
                followed = true;
                trace_conditioning(
                    ctx,
                    next,
                    prefer_negative,
                    out,
                    visited_slots,
                    visited_encoders,
                    params,
                    depth + 1,
                );
            }
        }
    }

    if !followed {
        for key in ["positive", "negative"] {
            if let Some(next) = inputs.get(key) {
                if next.is_string() || link_target(next).is_some() {
                    followed = true;
                    trace_conditioning(
                        ctx,
                        next,
                        key == "negative",
                        out,
                        visited_slots,
                        visited_encoders,
                        params,
                        depth + 1,
                    );
                }
            }
        }
    }

    // If a loader/custom node wired a pure text node directly into positive/negative
    if !followed {
        if let Some(text) = resolve_text(obj, value, 0, &mut HashSet::new()) {
            push_unique(out, &text);
        }
    }
}

fn is_sampler_execution_node(class_type: &str, inputs: &Map<String, Value>) -> bool {
    let lower = class_type.to_ascii_lowercase();
    if matches!(
        class_type,
        "KSampler"
            | "KSamplerAdvanced"
            | "SamplerCustom"
            | "SamplerCustomAdvanced"
            | "ClownsharKSampler"
            | "ClownsharKSampler_Beta"
            | "SharkSampler"
            | "SharkSampler_Beta"
    ) {
        return true;
    }
    if lower == "ksamplerselect"
        || lower.contains("samplerselect")
        || lower.contains("ksampler config")
    {
        return false;
    }
    if inputs.contains_key("guider") {
        return true;
    }
    let has_cond_or_pipe = inputs.contains_key("positive")
        || inputs.contains_key("negative")
        || inputs.contains_key("pipe")
        || inputs.contains_key("basic_pipe");
    has_cond_or_pipe
        && (lower.contains("sampler")
            || lower.contains("detailer")
            || lower.contains("upscale"))
}

fn trace_sigmas_params(
    ctx: &ComfyContext,
    value: &Value,
    params: &mut Vec<String>,
    visited: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Some((target_id, _)) = link_target(value) else {
        return;
    };
    if !visited.insert(target_id.clone()) {
        return;
    }
    let Some(inputs) = ctx
        .prompt
        .get(&target_id)
        .and_then(|n| n.get("inputs"))
        .and_then(|i| i.as_object())
    else {
        return;
    };
    for (key, label) in [
        ("steps", "Steps"),
        ("scheduler", "Scheduler"),
        ("denoise", "Denoise"),
    ] {
        if let Some(v) = inputs.get(key) {
            if let Some(s) = resolve_scalar(ctx, v, Some(key), 0, &mut HashSet::new()) {
                push_param(params, label, &s);
            }
        }
    }
    if let Some(next) = inputs.get("sigmas") {
        trace_sigmas_params(ctx, next, params, visited, depth + 1);
    }
}

fn trace_guider(
    ctx: &ComfyContext,
    guider_val: &Value,
    positive_parts: &mut Vec<String>,
    negative_parts: &mut Vec<String>,
    pos_encoders: &mut HashSet<String>,
    neg_encoders: &mut HashSet<String>,
    params: &mut Vec<String>,
) {
    let Some((guider_id, _)) = link_target(guider_val) else {
        return;
    };
    let Some(node) = ctx.prompt.get(&guider_id) else {
        return;
    };
    process_guider_node(
        ctx,
        node,
        positive_parts,
        negative_parts,
        pos_encoders,
        neg_encoders,
        params,
    );
}

fn process_guider_node(
    ctx: &ComfyContext,
    node: &Value,
    positive_parts: &mut Vec<String>,
    negative_parts: &mut Vec<String>,
    pos_encoders: &mut HashSet<String>,
    neg_encoders: &mut HashSet<String>,
    params: &mut Vec<String>,
) {
    let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
        return;
    };
    for key in ["cfg", "cfg_conds"] {
        if let Some(v) = inputs.get(key) {
            if let Some(s) = resolve_scalar(ctx, v, Some("cfg"), 0, &mut HashSet::new()) {
                push_param(params, "CFG", &s);
            }
        }
    }
    for key in ["positive", "conditioning", "cond1", "cond2"] {
        if let Some(v) = inputs.get(key) {
            trace_conditioning(
                ctx,
                v,
                false,
                positive_parts,
                &mut HashSet::new(),
                pos_encoders,
                params,
                0,
            );
        }
    }
    if let Some(v) = inputs.get("negative") {
        trace_conditioning(
            ctx,
            v,
            true,
            negative_parts,
            &mut HashSet::new(),
            neg_encoders,
            params,
            0,
        );
    }
}

fn trace_pipe_conditioning(
    ctx: &ComfyContext,
    pipe_val: &Value,
    positive_parts: &mut Vec<String>,
    negative_parts: &mut Vec<String>,
    pos_encoders: &mut HashSet<String>,
    neg_encoders: &mut HashSet<String>,
    params: &mut Vec<String>,
    visited: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Some((target_id, _)) = link_target(pipe_val) else {
        return;
    };
    if !visited.insert(target_id.clone()) {
        return;
    }
    let Some(inputs) = ctx
        .prompt
        .get(&target_id)
        .and_then(|n| n.get("inputs"))
        .and_then(|i| i.as_object())
    else {
        return;
    };
    if let Some(pos) = inputs.get("positive") {
        trace_conditioning(
            ctx,
            pos,
            false,
            positive_parts,
            &mut HashSet::new(),
            pos_encoders,
            params,
            0,
        );
    }
    if let Some(neg) = inputs.get("negative") {
        trace_conditioning(
            ctx,
            neg,
            true,
            negative_parts,
            &mut HashSet::new(),
            neg_encoders,
            params,
            0,
        );
    }
    for key in ["pipe", "basic_pipe"] {
        if let Some(next) = inputs.get(key) {
            trace_pipe_conditioning(
                ctx,
                next,
                positive_parts,
                negative_parts,
                pos_encoders,
                neg_encoders,
                params,
                visited,
                depth + 1,
            );
        }
    }
}

fn trace_latent_size(
    ctx: &ComfyContext,
    value: &Value,
    params: &mut Vec<String>,
    visited: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Some((target_id, _)) = link_target(value) else {
        return;
    };
    if !visited.insert(target_id.clone()) {
        return;
    }
    let Some(inputs) = ctx
        .prompt
        .get(&target_id)
        .and_then(|n| n.get("inputs"))
        .and_then(|i| i.as_object())
    else {
        return;
    };
    let w = inputs
        .get("width")
        .or_else(|| inputs.get("empty_latent_width"))
        .and_then(|v| resolve_scalar(ctx, v, Some("width"), 0, &mut HashSet::new()));
    let h = inputs
        .get("height")
        .or_else(|| inputs.get("empty_latent_height"))
        .and_then(|v| resolve_scalar(ctx, v, Some("height"), 0, &mut HashSet::new()));
    if let (Some(w), Some(h)) = (w, h) {
        if w.parse::<u64>().is_ok_and(|n| n > 0) && h.parse::<u64>().is_ok_and(|n| n > 0) {
            push_param(params, "Size", &format!("{w}x{h}"));
            return;
        }
    }
    for key in ["latent", "latent_image", "samples", "ctx_01", "ctx_02", "base_ctx", "context"] {
        if let Some(next) = inputs.get(key) {
            if link_target(next).is_some() {
                trace_latent_size(ctx, next, params, visited, depth + 1);
            }
        }
    }
}

fn format_strength(val: &Value) -> Option<String> {
    let s = scalar_string(val)?;
    if let Ok(n) = s.parse::<f64>() {
        if (n - n.round()).abs() < 1e-6 {
            return Some(format!("{}", n.round() as i64));
        }
        let trimmed = format!("{n:.4}");
        return Some(trimmed.trim_end_matches('0').trim_end_matches('.').to_string());
    }
    Some(s)
}

fn extract_active_loras(ctx: &ComfyContext) -> Vec<String> {
    let mut loras = Vec::new();
    for (_node_id, node) in ctx.prompt {
        let Some(class_type) = node.get("class_type").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
            continue;
        };
        let lower = class_type.to_ascii_lowercase();
        if !lower.contains("lora") {
            continue;
        }

        // LoraManager: inputs.loras.__value__ = [{ name, strength, active }, ...]
        if let Some(arr) = inputs
            .get("loras")
            .and_then(|v| v.get("__value__"))
            .and_then(|v| v.as_array())
        {
            for item in arr {
                if item.get("active").and_then(|v| v.as_bool()) != Some(true) {
                    continue;
                }
                if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    let base = clean_model_basename(name);
                    if base.is_empty() {
                        continue;
                    }
                    let label = match item.get("strength").and_then(format_strength) {
                        Some(st) => format!("{base}:{st}"),
                        None => base.to_string(),
                    };
                    push_unique(&mut loras, &label);
                }
            }
            continue;
        }

        // Power Lora Loader (rgthree): lora_1: { on: true, lora: "...", strength: 0.8 }
        let mut found_rgthree = false;
        for (key, val) in inputs {
            if key.starts_with("lora_") {
                if let Some(obj) = val.as_object() {
                    found_rgthree = true;
                    if obj.get("on").and_then(|v| v.as_bool()) == Some(true) {
                        if let Some(name) = obj.get("lora").and_then(|v| v.as_str()) {
                            let base = clean_model_basename(name);
                            if !base.is_empty() && !base.eq_ignore_ascii_case("none") {
                                let label = match obj.get("strength").and_then(format_strength) {
                                    Some(st) => format!("{base}:{st}"),
                                    None => base.to_string(),
                                };
                                push_unique(&mut loras, &label);
                            }
                        }
                    }
                }
            }
        }
        if found_rgthree {
            continue;
        }

        // Standard LoraLoader / LoraLoaderModelOnly
        if let Some(name_val) = inputs.get("lora_name") {
            if let Some(name) = resolve_scalar(ctx, name_val, Some("lora_name"), 0, &mut HashSet::new()) {
                let base = clean_model_basename(&name);
                if !base.is_empty() && !base.eq_ignore_ascii_case("none") {
                    let strength = inputs
                        .get("strength_model")
                        .or_else(|| inputs.get("strength"))
                        .and_then(|v| resolve_scalar(ctx, v, None, 0, &mut HashSet::new()));
                    let is_zero = strength
                        .as_deref()
                        .and_then(|s| s.parse::<f64>().ok())
                        .is_some_and(|n| n.abs() < 1e-9);
                    if !is_zero {
                        let label = match strength {
                            Some(st) => format!("{base}:{st}"),
                            None => base.to_string(),
                        };
                        push_unique(&mut loras, &label);
                    }
                }
            }
        }
    }
    loras
}

fn clean_model_basename(raw: &str) -> &str {
    raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim()
}

#[cfg(test)]
fn parse_comfyui(prompt_json: &str) -> Option<RawMeta> {
    parse_comfyui_with_workflow(prompt_json, None)
}

fn parse_comfyui_with_workflow(prompt_json: &str, workflow_json: Option<&str>) -> Option<RawMeta> {
    // ComfyUI prompt JSON: { "node_id": { "class_type": "...", "inputs": { ... } } }
    let parsed = parse_json_lenient(prompt_json)?;
    let root = parsed.as_object()?;
    let obj = if is_comfyui_prompt_graph(root) {
        root
    } else if let Some(nested) = root
        .get("prompt")
        .and_then(|v| v.as_object())
        .filter(|m| is_comfyui_prompt_graph(m))
    {
        nested
    } else {
        return None;
    };

    let workflow_val = workflow_json.and_then(parse_json_lenient);
    let ctx = ComfyContext::new(obj, workflow_val.as_ref());

    let mut positive_parts: Vec<String> = Vec::new();
    let mut negative_parts: Vec<String> = Vec::new();
    let mut param_parts: Vec<String> = Vec::new();
    let mut pos_encoders: HashSet<String> = HashSet::new();
    let mut neg_encoders: HashSet<String> = HashSet::new();

    // Pass 1: Model / CLIP / VAE loaders (placed first in params summary)
    for (_node_id, node) in obj {
        let Some(class_type) = node.get("class_type").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
            continue;
        };
        let lower = class_type.to_ascii_lowercase();
        if lower.contains("loader")
            || lower.contains("checkpoint")
            || lower.contains("unet")
            || lower.contains("diffusionmodel")
        {
            for key in ["ckpt_name", "unet_name", "diffusion_model", "gguf_name"] {
                if let Some(v) = inputs.get(key) {
                    if let Some(s) = resolve_scalar(&ctx, v, Some(key), 0, &mut HashSet::new()) {
                        push_param(&mut param_parts, "Model", clean_model_basename(&s));
                    }
                }
            }
            if lower.contains("clip") {
                for key in ["clip_name", "clip_name1", "clip_name2", "clip_name3"] {
                    if let Some(v) = inputs.get(key) {
                        if let Some(s) = resolve_scalar(&ctx, v, Some(key), 0, &mut HashSet::new()) {
                            push_param(&mut param_parts, "CLIP", clean_model_basename(&s));
                        }
                    }
                }
            }
            if lower.contains("vae") {
                if let Some(v) = inputs.get("vae_name") {
                    if let Some(s) = resolve_scalar(&ctx, v, Some("vae_name"), 0, &mut HashSet::new()) {
                        push_param(&mut param_parts, "VAE", clean_model_basename(&s));
                    }
                }
            }
        }
    }

    let active_loras = extract_active_loras(&ctx);
    if !active_loras.is_empty() {
        push_param(&mut param_parts, "LoRA", &active_loras.join(", "));
    }

    // Pass 2: Samplers & Guiders — trace positive/negative conditioning & sampling parameters
    for (_node_id, node) in obj {
        let Some(class_type) = node.get("class_type").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
            continue;
        };
        let lower = class_type.to_ascii_lowercase();

        if is_sampler_execution_node(class_type, inputs) {
            for (key, label) in [
                ("steps", "Steps"),
                ("sampler_name", "Sampler"),
                ("scheduler", "Scheduler"),
                ("cfg", "CFG"),
                ("guidance", "Guidance"),
                ("seed", "Seed"),
                ("noise_seed", "Seed"),
                ("denoise", "Denoise"),
            ] {
                if let Some(v) = inputs.get(key) {
                    if let Some(s) = resolve_scalar(&ctx, v, Some(key), 0, &mut HashSet::new()) {
                        push_param(&mut param_parts, label, &s);
                    }
                }
            }
            if let Some(noise) = inputs.get("noise") {
                if let Some(seed) = resolve_scalar(&ctx, noise, Some("noise_seed"), 0, &mut HashSet::new()) {
                    push_param(&mut param_parts, "Seed", &seed);
                }
            }
            if let Some(sampler) = inputs.get("sampler") {
                if let Some(name) = resolve_scalar(&ctx, sampler, Some("sampler_name"), 0, &mut HashSet::new()) {
                    push_param(&mut param_parts, "Sampler", &name);
                }
            }
            if let Some(sigmas) = inputs.get("sigmas") {
                trace_sigmas_params(&ctx, sigmas, &mut param_parts, &mut HashSet::new(), 0);
            }
            if let Some(latent) = inputs.get("latent_image") {
                trace_latent_size(&ctx, latent, &mut param_parts, &mut HashSet::new(), 0);
            }
            if let Some(guider) = inputs.get("guider") {
                trace_guider(
                    &ctx,
                    guider,
                    &mut positive_parts,
                    &mut negative_parts,
                    &mut pos_encoders,
                    &mut neg_encoders,
                    &mut param_parts,
                );
            }
            if let Some(pos) = inputs.get("positive") {
                trace_conditioning(
                    &ctx,
                    pos,
                    false,
                    &mut positive_parts,
                    &mut HashSet::new(),
                    &mut pos_encoders,
                    &mut param_parts,
                    0,
                );
            }
            if let Some(neg) = inputs.get("negative") {
                trace_conditioning(
                    &ctx,
                    neg,
                    true,
                    &mut negative_parts,
                    &mut HashSet::new(),
                    &mut neg_encoders,
                    &mut param_parts,
                    0,
                );
            }
            if let Some(cond) = inputs.get("conditioning") {
                trace_conditioning(
                    &ctx,
                    cond,
                    false,
                    &mut positive_parts,
                    &mut HashSet::new(),
                    &mut pos_encoders,
                    &mut param_parts,
                    0,
                );
            }
            for pipe_key in ["pipe", "basic_pipe"] {
                if let Some(pipe) = inputs.get(pipe_key) {
                    trace_pipe_conditioning(
                        &ctx,
                        pipe,
                        &mut positive_parts,
                        &mut negative_parts,
                        &mut pos_encoders,
                        &mut neg_encoders,
                        &mut param_parts,
                        &mut HashSet::new(),
                        0,
                    );
                }
            }
        } else if lower.contains("guider") {
            process_guider_node(
                &ctx,
                node,
                &mut positive_parts,
                &mut negative_parts,
                &mut pos_encoders,
                &mut neg_encoders,
                &mut param_parts,
            );
        }
    }

    // Pass 3: Fallback latent / resolution nodes if sampler didn't trace Size
    if !param_parts.iter().any(|p| p.starts_with("Size: ")) {
        for (_node_id, node) in obj {
            let Some(class_type) = node.get("class_type").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
                continue;
            };
            let lower = class_type.to_ascii_lowercase();
            if lower.contains("latent")
                || lower.contains("resolution")
                || lower.contains("efficient loader")
                || lower.contains("eff. loader")
            {
                let w = inputs
                    .get("width")
                    .or_else(|| inputs.get("empty_latent_width"))
                    .and_then(|v| resolve_scalar(&ctx, v, Some("width"), 0, &mut HashSet::new()));
                let h = inputs
                    .get("height")
                    .or_else(|| inputs.get("empty_latent_height"))
                    .and_then(|v| resolve_scalar(&ctx, v, Some("height"), 0, &mut HashSet::new()));
                if let (Some(w), Some(h)) = (w, h) {
                    if w.parse::<u64>().is_ok_and(|n| n > 0) && h.parse::<u64>().is_ok_and(|n| n > 0) {
                        push_param(&mut param_parts, "Size", &format!("{w}x{h}"));
                    }
                }
            }
        }
    }

    // Pass 4: Fallback for unlinked / partially wired text encoders
    if positive_parts.is_empty() || negative_parts.is_empty() {
        let mut fallback_pos = Vec::new();
        let mut fallback_neg = Vec::new();
        for (node_id, node) in obj {
            if pos_encoders.contains(node_id) || neg_encoders.contains(node_id) {
                continue;
            }
            let target = if is_negative_title(node) {
                &mut fallback_neg
            } else {
                &mut fallback_pos
            };
            extract_encoder_texts(
                &ctx,
                node_id,
                node,
                target,
                &mut HashSet::new(),
                &mut param_parts,
            );
        }
        if positive_parts.is_empty() {
            positive_parts = fallback_pos;
        }
        if negative_parts.is_empty() {
            negative_parts = fallback_neg;
        }
    }

    // Pass 5: Fallback for all-in-one loader / styler nodes when no encoders matched
    if positive_parts.is_empty() && negative_parts.is_empty() {
        for (_node_id, node) in obj {
            let Some(inputs) = node.get("inputs").and_then(|v| v.as_object()) else {
                continue;
            };
            for key in ["positive", "text_positive", "positive_prompt"] {
                if let Some(v) = inputs.get(key) {
                    if let Some(s) = resolve_text(obj, v, 0, &mut HashSet::new()) {
                        push_unique(&mut positive_parts, &s);
                    }
                }
            }
            for key in ["negative", "text_negative", "negative_prompt"] {
                if let Some(v) = inputs.get(key) {
                    if let Some(s) = resolve_text(obj, v, 0, &mut HashSet::new()) {
                        push_unique(&mut negative_parts, &s);
                    }
                }
            }
        }
    }

    Some(RawMeta {
        positive: positive_parts.join(", "),
        negative: negative_parts.join(", "),
        params: param_parts.join(", "),
        source: "comfyui".to_string(),
    })
}

fn workflow_node_id(node: &Value) -> Option<String> {
    match node.get("id")? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn workflow_widget_strings(node: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(values) = node.get("widgets_values").and_then(|v| v.as_array()) {
        for v in values {
            if let Some(s) = v.as_str() {
                push_unique(&mut out, s);
            }
        }
    } else if let Some(values) = node.get("widgets_values").and_then(|v| v.as_object()) {
        for key in ["text", "prompt", "value", "string", "t5xxl", "clip_l"] {
            if let Some(s) = values.get(key).and_then(|v| v.as_str()) {
                push_unique(&mut out, s);
            }
        }
    }
    out
}

fn trace_workflow_link(
    nodes_by_id: &HashMap<String, &Value>,
    links_by_id: &HashMap<u64, (String, u64)>,
    link_id: u64,
    out: &mut Vec<String>,
    visited: &mut HashSet<String>,
    visited_encoders: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 24 {
        return;
    }
    let Some((from_id, from_slot)) = links_by_id.get(&link_id) else {
        return;
    };
    let visit_key = format!("{from_id}:{from_slot}");
    if !visited.insert(visit_key) {
        return;
    }
    let Some(node) = nodes_by_id.get(from_id) else {
        return;
    };
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let lower = node_type.to_ascii_lowercase();
    if lower == "conditioningzeroout" || lower.contains("zeroout") {
        return;
    }

    if is_text_encoder_class(&lower) {
        visited_encoders.insert(from_id.clone());
        // If text input is linked in workflow, trace it first
        let mut linked_text = false;
        if let Some(inputs) = node.get("inputs").and_then(|v| v.as_array()) {
            for inp in inputs {
                let name = inp.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(
                    name,
                    "text" | "prompt" | "t5xxl" | "clip_l" | "clip_g" | "text_g" | "text_l" | "user_prompt"
                ) {
                    if let Some(lid) = inp.get("link").and_then(|v| v.as_u64()) {
                        linked_text = true;
                        trace_workflow_link(
                            nodes_by_id,
                            links_by_id,
                            lid,
                            out,
                            visited,
                            visited_encoders,
                            depth + 1,
                        );
                    }
                }
            }
        }
        if !linked_text {
            for s in workflow_widget_strings(node) {
                push_unique(out, &s);
            }
        }
        return;
    }

    if lower.contains("primitive")
        || lower.contains("string")
        || lower.contains("text")
        || lower.contains("wildcard")
    {
        let mut followed_text = false;
        if let Some(inputs) = node.get("inputs").and_then(|v| v.as_array()) {
            for inp in inputs {
                if let Some(lid) = inp.get("link").and_then(|v| v.as_u64()) {
                    followed_text = true;
                    trace_workflow_link(
                        nodes_by_id,
                        links_by_id,
                        lid,
                        out,
                        visited,
                        visited_encoders,
                        depth + 1,
                    );
                }
            }
        }
        if !followed_text {
            for s in workflow_widget_strings(node) {
                push_unique(out, &s);
            }
        }
        return;
    }

    if let Some(inputs) = node.get("inputs").and_then(|v| v.as_array()) {
        for inp in inputs {
            let name = inp.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let follow = matches!(
                name,
                "conditioning"
                    | "conditioning_1"
                    | "conditioning_2"
                    | "conditioning_to"
                    | "conditioning_from"
                    | "cond"
                    | "input"
            ) || (name == "positive" && *from_slot == 0)
                || (name == "negative" && *from_slot == 1);
            if follow {
                if let Some(lid) = inp.get("link").and_then(|v| v.as_u64()) {
                    trace_workflow_link(
                        nodes_by_id,
                        links_by_id,
                        lid,
                        out,
                        visited,
                        visited_encoders,
                        depth + 1,
                    );
                }
            }
        }
    }
}

fn collect_workflow_graph<'a>(
    graph: &'a Map<String, Value>,
    positive_parts: &mut Vec<String>,
    negative_parts: &mut Vec<String>,
    param_parts: &mut Vec<String>,
) {
    let Some(nodes) = graph.get("nodes").and_then(|v| v.as_array()) else {
        return;
    };
    let mut nodes_by_id: HashMap<String, &'a Value> = HashMap::new();
    for node in nodes {
        // Skip muted (2) or bypassed (4) UI nodes
        if matches!(node.get("mode").and_then(|v| v.as_u64()), Some(2 | 4)) {
            continue;
        }
        if let Some(id) = workflow_node_id(node) {
            nodes_by_id.insert(id, node);
        }
    }

    let mut links_by_id: HashMap<u64, (String, u64)> = HashMap::new();
    if let Some(links) = graph.get("links").and_then(|v| v.as_array()) {
        for link in links {
            if let Some(arr) = link.as_array() {
                if arr.len() >= 3 {
                    if let (Some(lid), Some(slot)) = (arr[0].as_u64(), arr[2].as_u64()) {
                        let from_id = match &arr[1] {
                            Value::String(s) => s.clone(),
                            Value::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        links_by_id.insert(lid, (from_id, slot));
                    }
                }
            } else if let Some(obj) = link.as_object() {
                // Subgraph link format: { id, origin_id, origin_slot, ... }
                if let (Some(lid), Some(slot)) = (
                    obj.get("id").and_then(|v| v.as_u64()),
                    obj.get("origin_slot").and_then(|v| v.as_u64()),
                ) {
                    if let Some(origin) = obj.get("origin_id") {
                        let from_id = match origin {
                            Value::String(s) => s.clone(),
                            Value::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        links_by_id.insert(lid, (from_id, slot));
                    }
                }
            }
        }
    }

    let mut visited_encoders: HashSet<String> = HashSet::new();
    for node in nodes {
        if matches!(node.get("mode").and_then(|v| v.as_u64()), Some(2 | 4)) {
            continue;
        }
        let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let lower = node_type.to_ascii_lowercase();

        if node_type == "KSampler" {
            if let Some(w) = node.get("widgets_values").and_then(|v| v.as_array()) {
                // Standard KSampler widgets: [seed, control, steps, cfg, sampler_name, scheduler, denoise]
                if w.len() >= 7 {
                    if let Some(s) = scalar_string(&w[2]) {
                        push_param(param_parts, "Steps", &s);
                    }
                    if let Some(s) = scalar_string(&w[4]) {
                        push_param(param_parts, "Sampler", &s);
                    }
                    if let Some(s) = scalar_string(&w[5]) {
                        push_param(param_parts, "Scheduler", &s);
                    }
                    if let Some(s) = scalar_string(&w[3]) {
                        push_param(param_parts, "CFG", &s);
                    }
                    if let Some(s) = scalar_string(&w[0]) {
                        push_param(param_parts, "Seed", &s);
                    }
                    if let Some(s) = scalar_string(&w[6]) {
                        push_param(param_parts, "Denoise", &s);
                    }
                }
            }
        }

        if lower.contains("sampler") || lower.contains("guider") || lower.contains("detailer") {
            if let Some(inputs) = node.get("inputs").and_then(|v| v.as_array()) {
                for inp in inputs {
                    let name = inp.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let Some(lid) = inp.get("link").and_then(|v| v.as_u64()) else {
                        continue;
                    };
                    if matches!(name, "positive" | "conditioning" | "cond1" | "cond2") {
                        trace_workflow_link(
                            &nodes_by_id,
                            &links_by_id,
                            lid,
                            positive_parts,
                            &mut HashSet::new(),
                            &mut visited_encoders,
                            0,
                        );
                    } else if name == "negative" {
                        trace_workflow_link(
                            &nodes_by_id,
                            &links_by_id,
                            lid,
                            negative_parts,
                            &mut HashSet::new(),
                            &mut visited_encoders,
                            0,
                        );
                    }
                }
            }
        }
    }

    if positive_parts.is_empty() || negative_parts.is_empty() {
        let mut fallback_pos = Vec::new();
        let mut fallback_neg = Vec::new();
        for node in nodes {
            if matches!(node.get("mode").and_then(|v| v.as_u64()), Some(2 | 4)) {
                continue;
            }
            let Some(id) = workflow_node_id(node) else {
                continue;
            };
            if visited_encoders.contains(&id) {
                continue;
            }
            let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if !is_text_encoder_class(&node_type.to_ascii_lowercase()) {
                continue;
            }
            let target = if is_negative_title(node) {
                &mut fallback_neg
            } else {
                &mut fallback_pos
            };
            for s in workflow_widget_strings(node) {
                push_unique(target, &s);
            }
        }
        if positive_parts.is_empty() {
            *positive_parts = fallback_pos;
        }
        if negative_parts.is_empty() {
            *negative_parts = fallback_neg;
        }
    }
}

fn parse_comfyui_workflow(workflow_json: &str) -> Option<RawMeta> {
    let parsed = parse_json_lenient(workflow_json)?;
    let obj = parsed.as_object()?;
    if obj.get("nodes").and_then(|v| v.as_array()).is_none() {
        return None;
    }

    let mut positive_parts = Vec::new();
    let mut negative_parts = Vec::new();
    let mut param_parts = Vec::new();

    collect_workflow_graph(obj, &mut positive_parts, &mut negative_parts, &mut param_parts);

    // Also inspect ComfyUI 2025+ subgraph definitions if top-level had no prompts
    if positive_parts.is_empty() && negative_parts.is_empty() {
        if let Some(subgraphs) = obj
            .get("definitions")
            .and_then(|d| d.get("subgraphs"))
            .and_then(|s| s.as_array())
        {
            for sg in subgraphs {
                if let Some(sg_obj) = sg.as_object() {
                    collect_workflow_graph(
                        sg_obj,
                        &mut positive_parts,
                        &mut negative_parts,
                        &mut param_parts,
                    );
                }
            }
        }
    }

    Some(RawMeta {
        positive: positive_parts.join(", "),
        negative: negative_parts.join(", "),
        params: param_parts.join(", "),
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

    let looks_like_novelai = !source_ver.is_empty()
        || obj.contains_key("v4_prompt")
        || obj.contains_key("v4_negative_prompt")
        || obj.contains_key("prompt")
        || obj.contains_key("uc")
        || obj.contains_key("sampler")
        || obj.contains_key("scale")
        || obj.contains_key("noise_schedule");

    if !looks_like_novelai
        || (positive.is_empty()
            && artist_parts.is_empty()
            && negative.is_empty()
            && param_parts.is_empty())
    {
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
        source: "novelai".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::TempDir;

    /// 在 PNG 的 IHDR 之后插入一个 tEXt 块
    fn png_with_text(path: &Path, key: &str, value: &str) {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(2, 2)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        // 8 字节签名 + IHDR（4 长度 + 4 类型 + 13 数据 + 4 CRC）
        let (head, tail) = bytes.split_at(8 + 25);
        let data = [key.as_bytes(), &[0], value.as_bytes()].concat();
        let mut crc = flate2::Crc::new();
        crc.update(b"tEXt");
        crc.update(&data);
        let mut out = head.to_vec();
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(b"tEXt");
        out.extend(&data);
        out.extend(crc.sum().to_be_bytes());
        out.extend(tail);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn scan_and_export_each_run_in_one_tagged_round() {
        let root = TempDir::new("sd_metadata_scan");
        png_with_text(
            &root.join("a.png"),
            "parameters",
            "1girl, solo\nNegative prompt: lowres\nSteps: 20",
        );
        image::RgbImage::new(2, 2).save(root.join("b.png")).unwrap();
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);

        let scan = scan_sync(app.handle(), &root.to_string_lossy(), false).unwrap();
        assert_eq!((scan.has_meta_count, scan.no_meta_count), (1, 1));
        assert_eq!(scan.items[0].positive, "1girl, solo");
        {
            let events = log.lock().unwrap();
            let run_id = events[0]["run_id"].as_u64().unwrap();
            assert!(events.iter().all(|e| e["run_id"] == run_id));
            let done = events.last().unwrap();
            assert_eq!(done["message"], "扫描完成: 2 张图片, 1 有元数据");
            assert_eq!((&done["current"], &done["total"]), (&2.into(), &2.into()));
        }
        log.lock().unwrap().clear();

        let export = export_sync(
            app.handle(),
            &ExportTagsOptions {
                mode: "same".into(),
                dest_folder: None,
                input_root: None,
                items: vec![
                    ExportTagItem {
                        source_path: root.join("a.png").to_string_lossy().into_owned(),
                        positive: "1girl, solo".into(),
                    },
                    ExportTagItem {
                        source_path: root.join("b.png").to_string_lossy().into_owned(),
                        positive: " ".into(),
                    },
                ],
            },
        )
        .unwrap();
        assert_eq!(
            (export.success_count, export.fail_count, export.skip_count),
            (1, 0, 1)
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "1girl, solo"
        );
        let events = log.lock().unwrap();
        let done = events.last().unwrap();
        assert_eq!(done["message"], "导出完成: 成功 1, 失败 0, 跳过 1");
        assert!(events.iter().all(|e| e["run_id"] == done["run_id"]));
    }

    #[test]
    fn malformed_nodes_do_not_discard_valid_prompts() {
        let prompt = serde_json::json!({
            "0": {"inputs": {}},
            "1": {"class_type": "CLIPTextEncode", "inputs": {"text": "positive"}},
            "2": {"class_type": "CLIPTextEncode", "inputs": {"text": "negative"}},
            "3": {"class_type": "KSampler"},
            "4": {"class_type": "KSampler", "inputs": {"negative": ["2", 0]}},
            "5": {"class_type": false, "inputs": null},
            "6": {"class_type": "CLIPTextEncode"}
        });
        let meta = parse_comfyui(&prompt.to_string()).unwrap();
        assert_eq!(meta.positive, "positive");
        assert_eq!(meta.negative, "negative");
    }

    #[test]
    fn comfyui_flux_sampler_custom_advanced_with_linked_text_and_zero_out() {
        let prompt = serde_json::json!({
            "10": {"class_type": "UNETLoader", "inputs": {"unet_name": "flux\\flux1-dev.safetensors"}},
            "20": {"class_type": "PrimitiveStringMultiline", "inputs": {"value": "1girl, cinematic sunlight"}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": ["20", 0], "clip": ["11", 0]}},
            "26": {"class_type": "FluxGuidance", "inputs": {"conditioning": ["6", 0], "guidance": 3.5}},
            "28": {"class_type": "ConditioningZeroOut", "inputs": {"conditioning": ["6", 0]}},
            "22": {"class_type": "CFGGuider", "inputs": {"positive": ["26", 0], "negative": ["28", 0], "cfg": 1.0, "model": ["10", 0]}},
            "25": {"class_type": "RandomNoise", "inputs": {"noise_seed": 987654321}},
            "16": {"class_type": "KSamplerSelect", "inputs": {"sampler_name": "euler"}},
            "17": {"class_type": "BasicScheduler", "inputs": {"scheduler": "simple", "steps": 24, "denoise": 1.0}},
            "27": {"class_type": "EmptySD3LatentImage", "inputs": {"width": 1024, "height": 1280}},
            "13": {"class_type": "SamplerCustomAdvanced", "inputs": {
                "noise": ["25", 0],
                "guider": ["22", 0],
                "sampler": ["16", 0],
                "sigmas": ["17", 0],
                "latent_image": ["27", 0]
            }}
        });
        let meta = parse_comfyui(&prompt.to_string()).unwrap();
        assert_eq!(meta.source, "comfyui");
        assert_eq!(meta.positive, "1girl, cinematic sunlight");
        assert_eq!(meta.negative, "");
        assert!(meta.params.contains("Model: flux1-dev.safetensors"));
        assert!(meta.params.contains("Steps: 24"));
        assert!(meta.params.contains("Sampler: euler"));
        assert!(meta.params.contains("Scheduler: simple"));
        assert!(meta.params.contains("Guidance: 3.5"));
        assert!(meta.params.contains("Seed: 987654321"));
        assert!(meta.params.contains("Size: 1024x1280"));
    }

    #[test]
    fn comfyui_handles_nan_infinity_concat_wildcards_controlnet_and_qwen_nodes() {
        let raw_with_nan = r#"{
            "12:1": {"class_type": "PrimitiveString", "inputs": {"value": "masterpiece"}},
            "12:2": {"class_type": "ImpactWildcardProcessor", "inputs": {"wildcard_text": "{red|blue} dress", "populated_text": "blue dress"}},
            "12:3": {"class_type": "StringConcatenate", "inputs": {"string_a": ["12:1", 0], "string_b": ["12:2", 0], "delimiter": ", "}},
            "12:6": {"class_type": "TextEncodeQwenImageEdit", "inputs": {"prompt": ["12:3", 0]}},
            "12:7": {"class_type": "CLIPTextEncodeSDXL", "inputs": {"text_g": "lowres, NaN artifact", "text_l": "lowres, NaN artifact"}},
            "12:8": {"class_type": "ControlNetApplyAdvanced", "inputs": {"positive": ["12:6", 0], "negative": ["12:7", 0]}},
            "12:9": {"class_type": "ClownsharKSampler", "inputs": {"positive": ["12:8", 0], "negative": ["12:8", 1], "steps": 28, "cfg": 5.5, "bad_val": NaN, "inf_val": -Infinity}}
        }"#;
        let meta = parse_comfyui(raw_with_nan).unwrap();
        assert_eq!(meta.positive, "masterpiece, blue dress");
        assert_eq!(meta.negative, "lowres, NaN artifact");
        assert!(meta.params.contains("Steps: 28"));
        assert!(meta.params.contains("CFG: 5.5"));
    }

    #[test]
    fn comfyui_workflow_only_and_empty_prompt_graph_are_recognized() {
        let prompt_only_upscale = serde_json::json!({
            "1": {"class_type": "LoadImage", "inputs": {"image": "in.png"}},
            "2": {"class_type": "ImageUpscaleWithModel", "inputs": {"image": ["1", 0]}}
        });
        let meta = parse_comfyui(&prompt_only_upscale.to_string()).unwrap();
        assert_eq!(meta.source, "comfyui");
        assert_eq!(meta.positive, "");

        let workflow = serde_json::json!({
            "nodes": [
                {"id": 6, "type": "CLIPTextEncode", "widgets_values": ["wf positive prompt"]},
                {"id": 7, "type": "CLIPTextEncode", "widgets_values": ["wf negative prompt"]},
                {"id": 3, "type": "KSampler", "inputs": [
                    {"name": "positive", "link": 10},
                    {"name": "negative", "link": 11}
                ], "widgets_values": [42, "fixed", 20, 7.0, "euler", "normal", 1.0]}
            ],
            "links": [
                [10, 6, 0, 3, 1, "CONDITIONING"],
                [11, 7, 0, 3, 2, "CONDITIONING"]
            ]
        });
        let wf_meta = parse_text_metadata(&[("workflow".into(), workflow.to_string())]).unwrap();
        assert_eq!(wf_meta.source, "comfyui");
        assert_eq!(wf_meta.positive, "wf positive prompt");
        assert_eq!(wf_meta.negative, "wf negative prompt");
        assert!(wf_meta.params.contains("Steps: 20"));
        assert!(wf_meta.params.contains("Seed: 42"));
    }

    #[test]
    fn lenient_png_reader_recovers_comfyui_with_trailing_bytes_or_quirky_chunks() {
        let root = TempDir::new("sd_metadata_comfyui_lenient");
        let path = root.join("comfy_quirky.png");
        let prompt = serde_json::json!({
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "quirky comfy prompt"}},
            "3": {"class_type": "KSampler", "inputs": {"positive": ["6", 0], "steps": 25}}
        })
        .to_string();
        png_with_text(&path, "prompt", &prompt);
        // Append trailing bytes after IEND AND inject a NUL-containing extra tEXt chunk before IEND
        let bytes = std::fs::read(&path).unwrap();
        let iend_pos = bytes.len() - 12;
        let extra_data = b"Custom\0has\0null";
        let mut crc = flate2::Crc::new();
        crc.update(b"tEXt");
        crc.update(extra_data);
        let mut rebuilt = bytes[..iend_pos].to_vec();
        rebuilt.extend((extra_data.len() as u32).to_be_bytes());
        rebuilt.extend(b"tEXt");
        rebuilt.extend(extra_data);
        rebuilt.extend(crc.sum().to_be_bytes());
        rebuilt.extend(&bytes[iend_pos..]);
        rebuilt.extend(b"trailing_bytes");
        std::fs::write(&path, &rebuilt).unwrap();

        let meta = read_single_sd_metadata(path.to_string_lossy().into_owned())
            .unwrap()
            .unwrap();
        assert_eq!(meta.source, "comfyui");
        assert_eq!(meta.positive, "quirky comfy prompt");
        assert!(meta.params.contains("Steps: 25"));
    }

    fn build_exif_with_make_and_desc(make: &str, desc: &str) -> Vec<u8> {
        let mut make_bytes = make.as_bytes().to_vec();
        make_bytes.push(0);
        let mut desc_bytes = desc.as_bytes().to_vec();
        desc_bytes.push(0);

        let mut tiff = Vec::new();
        tiff.extend(b"II");
        tiff.extend(42u16.to_le_bytes());
        tiff.extend(8u32.to_le_bytes()); // IFD0 at offset 8
        tiff.extend(2u16.to_le_bytes()); // 2 entries
        let data_start = 8u32 + 2 + 2 * 12 + 4;
        let desc_off = data_start;
        let make_off = data_start + desc_bytes.len() as u32;
        // Entry 1: ImageDescription (0x010E)
        tiff.extend(0x010Eu16.to_le_bytes());
        tiff.extend(2u16.to_le_bytes());
        tiff.extend((desc_bytes.len() as u32).to_le_bytes());
        tiff.extend(desc_off.to_le_bytes());
        // Entry 2: Make (0x010F)
        tiff.extend(0x010Fu16.to_le_bytes());
        tiff.extend(2u16.to_le_bytes());
        tiff.extend((make_bytes.len() as u32).to_le_bytes());
        tiff.extend(make_off.to_le_bytes());
        // Next IFD offset = 0
        tiff.extend(0u32.to_le_bytes());
        tiff.extend(&desc_bytes);
        tiff.extend(&make_bytes);
        tiff
    }

    #[test]
    fn reads_comfyui_metadata_from_webp_and_jpeg_exif() {
        let root = TempDir::new("sd_metadata_webp_jpeg");
        let prompt = serde_json::json!({
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "webp comfy girl"}},
            "3": {"class_type": "KSampler", "inputs": {"positive": ["6", 0], "steps": 30}}
        })
        .to_string();
        let exif = build_exif_with_make_and_desc(&format!("Prompt:{prompt}"), "Workflow:{\"nodes\":[]}");

        // Minimal RIFF WEBP with EXIF chunk
        let webp_path = root.join("comfy.webp");
        let mut webp_body = b"WEBP".to_vec();
        webp_body.extend(b"EXIF");
        webp_body.extend((exif.len() as u32).to_le_bytes());
        webp_body.extend(&exif);
        if exif.len() % 2 == 1 {
            webp_body.push(0);
        }
        let mut webp = b"RIFF".to_vec();
        webp.extend((webp_body.len() as u32).to_le_bytes());
        webp.extend(&webp_body);
        std::fs::write(&webp_path, &webp).unwrap();

        let webp_meta = read_single_sd_metadata(webp_path.to_string_lossy().into_owned())
            .unwrap()
            .unwrap();
        assert_eq!(webp_meta.source, "comfyui");
        assert_eq!(webp_meta.positive, "webp comfy girl");
        assert!(webp_meta.params.contains("Steps: 30"));

        // Minimal JPEG with APP1 Exif
        let jpg_path = root.join("comfy.jpg");
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(&exif);
        let seg_len = (app1.len() + 2) as u16;
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpg.extend(seg_len.to_be_bytes());
        jpg.extend(&app1);
        jpg.extend([0xFF, 0xD9]);
        std::fs::write(&jpg_path, &jpg).unwrap();

        let jpg_meta = read_single_sd_metadata(jpg_path.to_string_lossy().into_owned())
            .unwrap()
            .unwrap();
        assert_eq!(jpg_meta.source, "comfyui");
        assert_eq!(jpg_meta.positive, "webp comfy girl");

        // Regular camera JPEG EXIF should NOT produce false metadata
        let cam_path = root.join("camera.jpg");
        let cam_exif = build_exif_with_make_and_desc("Canon", "Vacation Photo");
        let mut cam_app1 = b"Exif\0\0".to_vec();
        cam_app1.extend(&cam_exif);
        let mut cam_jpg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        cam_jpg.extend(((cam_app1.len() + 2) as u16).to_be_bytes());
        cam_jpg.extend(&cam_app1);
        cam_jpg.extend([0xFF, 0xD9]);
        std::fs::write(&cam_path, &cam_jpg).unwrap();

        assert!(read_single_sd_metadata(cam_path.to_string_lossy().into_owned())
            .unwrap()
            .is_none());
    }

    #[test]
    fn novelai_cleared_positive_stays_novelai_without_falling_back_to_description() {
        let comment = serde_json::json!({
            "prompt": "",
            "steps": 28,
            "sampler": "k_euler_ancestral",
            "scale": 5.5,
            "seed": 123456,
            "width": 832,
            "height": 1216,
            "v4_prompt": {"caption": {"base_caption": "", "char_captions": []}},
            "v4_negative_prompt": {"caption": {"base_caption": "lowres", "char_captions": []}},
            "uc": "lowres"
        })
        .to_string();
        let chunks = vec![
            ("Comment".to_string(), comment),
            ("Description".to_string(), "stale description".to_string()),
            ("Source".to_string(), "NovelAI Diffusion V5 DB276663".to_string()),
        ];
        let meta = parse_text_metadata(&chunks).unwrap();
        assert_eq!(meta.source, "novelai");
        assert_eq!(meta.positive, "");
        assert_eq!(meta.negative, "lowres");
        assert!(meta.params.contains("Model: NovelAI Diffusion V5"));
        assert!(meta.params.contains("Steps: 28"));
    }

    #[test]
    fn comfyui_rgthree_context_workflow_panel_loramanager_and_resolution_master() {
        let prompt = serde_json::json!({
            "21188": {"class_type": "ImageToolsResolutionMaster", "inputs": {"width": 1280, "height": 1920}},
            "21189": {"class_type": "IS_NoisyLatentImage", "inputs": {"seed": 226538413143682u64, "width": ["21188", 0], "height": ["21188", 1]}},
            "21191": {"class_type": "YP_ParameterControlPanel", "inputs": {"pcp_ui": ""}},
            "21196": {"class_type": "YP_ParameterBreak", "inputs": {"parameters": ["21191", 0]}},
            "21200": {"class_type": "KSampler Config (rgthree)", "inputs": {
                "steps_total": ["21196", 0], "cfg": ["21196", 1], "sampler_name": ["21196", 2], "scheduler": ["21196", 3]
            }},
            "21212": {"class_type": "YP_PromptSelector", "inputs": {"selected_prompts": "best quality, masterpiece,"}},
            "21287": {"class_type": "Lora Loader (LoraManager)", "inputs": {
                "loras": {"__value__": [
                    {"name": "disabled_lora", "strength": "0.80", "active": false},
                    {"name": "komachi_pochi-v1", "strength": "1.00", "active": true}
                ]}
            }},
            "21288": {"class_type": "TriggerWord Toggle (LoraManager)", "inputs": {
                "toggle_trigger_words": {"__value__": [
                    {"text": "@active_tag", "active": true},
                    {"text": "@inactive_tag", "active": false}
                ]}
            }},
            "21353": {"class_type": "YP_PromptEditor", "inputs": {"prompt": "1girl, purple hair"}},
            "21355": {"class_type": "YP_PromptEditor", "inputs": {"prompt": "worst quality, blurry"}},
            "21224:1": {"class_type": "easy promptConcat", "inputs": {"separator": ",", "prompt1": ["21212", 0], "prompt2": ["21288", 0]}},
            "21224:2": {"class_type": "easy promptConcat", "inputs": {"separator": ",", "prompt1": ["21224:1", 0], "prompt2": ["21353", 0]}},
            "21190:1": {"class_type": "UNETLoader", "inputs": {"unet_name": "anima_baseV10.safetensors"}},
            "21190:2": {"class_type": "VAELoader", "inputs": {"vae_name": "qwen_image_vae.safetensors"}},
            "21190:3": {"class_type": "CLIPLoader", "inputs": {"clip_name": "anima_baseV10_txt.safetensors"}},
            "21229:1": {"class_type": "CLIPTextEncode", "inputs": {"text": ["21224:2", 0]}},
            "21229:2": {"class_type": "CLIPTextEncode", "inputs": {"text": ["21355", 0]}},
            "21229:3": {"class_type": "Context (rgthree)", "inputs": {
                "positive": ["21229:1", 0], "negative": ["21229:2", 0], "latent": ["21189", 0], "seed": ["21189", 1]
            }},
            "21243:1": {"class_type": "Context Switch (rgthree)", "inputs": {"ctx_01": ["21229:3", 0]}},
            "21243:2": {"class_type": "KSampler", "inputs": {
                "seed": ["21243:1", 8],
                "steps": ["21200", 0],
                "cfg": ["21200", 2],
                "sampler_name": ["21200", 3],
                "scheduler": ["21200", 4],
                "denoise": ["21196", 4],
                "positive": ["21243:1", 4],
                "negative": ["21243:1", 5],
                "latent_image": ["21243:1", 6]
            }}
        });
        let workflow = serde_json::json!({
            "nodes": [{
                "id": 21191,
                "type": "YP_ParameterControlPanel",
                "properties": {
                    "parameters": [
                        {"name": "采样设置", "type": "separator", "value": null},
                        {"name": "步数", "type": "slider", "value": 45},
                        {"name": "CFG", "type": "slider", "value": 4},
                        {"name": "采样器", "type": "dropdown", "value": "euler_ancestral"},
                        {"name": "调度器", "type": "dropdown", "value": "simple"},
                        {"name": "降噪幅度", "type": "slider", "value": 1}
                    ]
                }
            }]
        });
        let meta = parse_text_metadata(&[
            ("prompt".into(), prompt.to_string()),
            ("workflow".into(), workflow.to_string()),
        ])
        .unwrap();
        assert_eq!(meta.source, "comfyui");
        assert_eq!(meta.positive, "best quality, masterpiece, @active_tag, 1girl, purple hair");
        assert_eq!(meta.negative, "worst quality, blurry");
        assert!(meta.params.contains("Model: anima_baseV10.safetensors"));
        assert!(meta.params.contains("CLIP: anima_baseV10_txt.safetensors"));
        assert!(meta.params.contains("VAE: qwen_image_vae.safetensors"));
        assert!(meta.params.contains("LoRA: komachi_pochi-v1:1"));
        assert!(!meta.params.contains("disabled_lora"));
        assert!(meta.params.contains("Steps: 45"));
        assert!(meta.params.contains("Sampler: euler_ancestral"));
        assert!(meta.params.contains("Scheduler: simple"));
        assert!(meta.params.contains("CFG: 4"));
        assert!(meta.params.contains("Seed: 226538413143682"));
        assert!(meta.params.contains("Denoise: 1"));
        assert!(meta.params.contains("Size: 1280x1920"));
    }
}
