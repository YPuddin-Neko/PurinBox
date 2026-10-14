use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

use super::sd_metadata::{summarize_text_metadata, SdImageMeta};

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_TEXT_BYTES: usize = 64 * 1024 * 1024;
static SAVE_LOCK: Mutex<()> = Mutex::new(());
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub struct SdMetadataEntry {
    /// PNG 块序号，重复关键词各占一个条目。
    pub index: usize,
    pub keyword: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SdMetadataDocument {
    pub path: String,
    pub revision: String,
    pub entries: Vec<SdMetadataEntry>,
    pub metadata: Option<SdImageMeta>,
}

#[derive(Debug, Deserialize)]
pub struct SdMetadataEdit {
    pub index: usize,
    pub text: String,
}

#[derive(Debug)]
enum TextFormat {
    Latin { compressed: bool },
    International { prefix: Vec<u8>, compressed: bool },
}

#[derive(Debug)]
struct TextChunk {
    keyword: String,
    keyword_bytes: Vec<u8>,
    text: String,
    format: TextFormat,
}

#[derive(Debug)]
struct Chunk {
    raw: Range<usize>,
    text: Option<TextChunk>,
}

#[tauri::command]
pub async fn read_sd_metadata_document(file_path: String) -> Result<SdMetadataDocument, String> {
    tokio::task::spawn_blocking(move || read_document_sync(file_path))
        .await
        .map_err(|e| format!("读取元数据失败: {e}"))?
}

fn read_document_sync(file_path: String) -> Result<SdMetadataDocument, String> {
    let path = png_path(&file_path)?;
    let bytes = read_bytes(&path)?;
    document(Path::new(&file_path), &bytes, &parse_png(&bytes)?)
}

pub(super) fn read_text_chunks(path: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = read_bytes(path)?;
    Ok(parse_png(&bytes)?
        .into_iter()
        .filter_map(|chunk| chunk.text.map(|text| (text.keyword, text.text)))
        .collect())
}

#[tauri::command]
pub async fn save_sd_metadata_document(
    file_path: String,
    revision: String,
    edits: Vec<SdMetadataEdit>,
) -> Result<SdMetadataDocument, String> {
    tokio::task::spawn_blocking(move || save_document_sync(file_path, revision, edits))
        .await
        .map_err(|e| format!("保存元数据失败: {e}"))?
}

fn save_document_sync(
    file_path: String,
    revision: String,
    edits: Vec<SdMetadataEdit>,
) -> Result<SdMetadataDocument, String> {
    // 同一应用内两次保存不能同时通过旧修订检查。
    let _guard = SAVE_LOCK
        .lock()
        .map_err(|_| "元数据保存状态不可用".to_string())?;
    let path = png_path(&file_path)?;
    let bytes = read_bytes(&path)?;
    check_revision(&bytes, &revision)?;
    let chunks = parse_png(&bytes)?;
    let updated = apply_edits(&bytes, &chunks, edits)?;
    let result = document(Path::new(&file_path), &updated, &parse_png(&updated)?)?;
    if updated == bytes {
        return Ok(result);
    }
    let permissions = fs::metadata(&path)
        .map_err(|e| format!("读取文件属性失败: {e}"))?
        .permissions();
    if permissions.readonly() {
        return Err("图片为只读文件，无法保存".into());
    }
    // 目录可写不代表图片可写，原子替换前也检查文件自身的访问权限。
    OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|e| format!("图片不可写: {e}"))?;
    replace_checked(&path, &updated, &revision, permissions, || Ok(()))?;
    Ok(result)
}

fn png_path(file_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(file_path);
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        return Err("仅支持编辑 PNG 图片的文本元数据".into());
    }
    let path = fs::canonicalize(path).map_err(|e| format!("无法打开图片: {e}"))?;
    if !path.is_file() {
        return Err("请选择 PNG 图片文件".into());
    }
    Ok(path)
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("读取图片失败: {e}"))?;
    if file
        .metadata()
        .map_err(|e| format!("读取文件属性失败: {e}"))?
        .len()
        > MAX_FILE_BYTES as u64
    {
        return Err("图片超过 512 MiB，无法编辑元数据".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取图片失败: {e}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("图片超过 512 MiB，无法编辑元数据".into());
    }
    Ok(bytes)
}

fn revision(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn check_revision(bytes: &[u8], expected: &str) -> Result<(), String> {
    if revision(bytes) != expected {
        return Err("图片已被修改，请重新读取后再保存".into());
    }
    Ok(())
}

fn document(path: &Path, bytes: &[u8], chunks: &[Chunk]) -> Result<SdMetadataDocument, String> {
    let entries: Vec<_> = chunks
        .iter()
        .enumerate()
        .filter_map(|(index, chunk)| {
            chunk.text.as_ref().map(|text| SdMetadataEntry {
                index,
                keyword: text.keyword.clone(),
                text: text.text.clone(),
            })
        })
        .collect();
    let texts: Vec<_> = entries
        .iter()
        .map(|entry| (entry.keyword.clone(), entry.text.clone()))
        .collect();
    Ok(SdMetadataDocument {
        path: path.to_string_lossy().into_owned(),
        revision: revision(bytes),
        metadata: summarize_text_metadata(path, &texts),
        entries,
    })
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

fn invalid(reason: &str) -> String {
    format!("PNG 文件损坏或格式不支持: {reason}")
}

fn parse_png(bytes: &[u8]) -> Result<Vec<Chunk>, String> {
    if !bytes.starts_with(SIGNATURE) {
        return Err(invalid("文件签名无效"));
    }
    let mut chunks = Vec::new();
    let mut pos = SIGNATURE.len();
    let mut seen_idat = false;
    let mut ended_idat = false;
    let mut total_text = 0usize;
    while pos < bytes.len() {
        if bytes.len() - pos < 12 || chunks.len() >= 100_000 {
            return Err(invalid("数据块不完整或数量过多"));
        }
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("数据块长度无效"))?;
        let kind = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..end - 4];
        if !kind.iter().all(u8::is_ascii_alphabetic) || !kind[2].is_ascii_uppercase() {
            return Err(invalid("数据块类型无效"));
        }
        if crc32(&bytes[pos + 4..end - 4])
            != u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap())
        {
            return Err(invalid("数据块校验失败"));
        }
        if chunks.is_empty() && (kind != b"IHDR" || len != 13) {
            return Err(invalid("缺少图片头"));
        }
        if kind == b"IHDR" {
            if !chunks.is_empty()
                || len != 13
                || data[..4] == [0; 4]
                || data[4..8] == [0; 4]
                || data[10] != 0
                || data[11] != 0
                || data[12] > 1
            {
                return Err(invalid("图片头无效"));
            }
            let depth_ok = match data[9] {
                0 => matches!(data[8], 1 | 2 | 4 | 8 | 16),
                2 | 4 | 6 => matches!(data[8], 8 | 16),
                3 => matches!(data[8], 1 | 2 | 4 | 8),
                _ => false,
            };
            if !depth_ok {
                return Err(invalid("颜色类型或位深无效"));
            }
        }
        if kind == b"IDAT" {
            if ended_idat {
                return Err(invalid("图片数据块不连续"));
            }
            seen_idat = true;
        } else if seen_idat {
            ended_idat = true;
        }
        let text = if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt") {
            Some(parse_text(kind, data)?)
        } else {
            None
        };
        if let Some(text) = &text {
            total_text += text.text.len();
            if total_text > MAX_TOTAL_TEXT_BYTES {
                return Err("文本元数据总量超过 64 MiB".into());
            }
        }
        chunks.push(Chunk {
            raw: pos..end,
            text,
        });
        if kind == b"IEND" {
            if len != 0 || !seen_idat || end != bytes.len() {
                return Err(invalid("结束块无效"));
            }
            return Ok(chunks);
        }
        pos = end;
    }
    Err(invalid("缺少结束块"))
}

fn inflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = Decompress::new(true);
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &bytes[before_in as usize..],
                &mut buffer,
                FlushDecompress::None,
            )
            .map_err(|e| invalid(&format!("文本解压失败: {e}")))?;
        let produced = (decoder.total_out() - before_out) as usize;
        if output.len() + produced > MAX_TEXT_BYTES {
            return Err("单个文本元数据超过 16 MiB".into());
        }
        output.extend_from_slice(&buffer[..produced]);
        if status == Status::StreamEnd {
            if decoder.total_in() as usize != bytes.len() {
                return Err(invalid("压缩文本包含多余数据"));
            }
            return Ok(output);
        }
        if decoder.total_in() == before_in && produced == 0 {
            return Err(invalid("压缩文本不完整"));
        }
    }
}

fn parse_text(kind: &[u8], data: &[u8]) -> Result<TextChunk, String> {
    let split = data
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| invalid("文本关键词不完整"))?;
    let keyword = &data[..split];
    if keyword.is_empty()
        || keyword.len() > 79
        || keyword.first() == Some(&b' ')
        || keyword.last() == Some(&b' ')
        || keyword.windows(2).any(|v| v == b"  ")
        || !keyword.iter().all(|b| matches!(b, 32..=126 | 161..=255))
    {
        return Err(invalid("文本关键词无效"));
    }
    let rest = &data[split + 1..];
    let (decoded, format) = match kind {
        b"tEXt" => {
            if rest.len() > MAX_TEXT_BYTES {
                return Err("单个文本元数据超过 16 MiB".into());
            }
            (rest.to_vec(), TextFormat::Latin { compressed: false })
        }
        b"zTXt" => {
            if rest.first() != Some(&0) {
                return Err(invalid("文本压缩方式无效"));
            }
            (inflate(&rest[1..])?, TextFormat::Latin { compressed: true })
        }
        _ => {
            if rest.len() < 4 || rest[0] > 1 || rest[1] != 0 {
                return Err(invalid("国际文本压缩标记无效"));
            }
            let language_end = rest[2..]
                .iter()
                .position(|b| *b == 0)
                .map(|i| i + 2)
                .ok_or_else(|| invalid("国际文本语言字段不完整"))?;
            if !rest[2..language_end].is_ascii() {
                return Err(invalid("国际文本语言字段无效"));
            }
            let translated_end = rest[language_end + 1..]
                .iter()
                .position(|b| *b == 0)
                .map(|i| i + language_end + 1)
                .ok_or_else(|| invalid("国际文本关键词不完整"))?;
            std::str::from_utf8(&rest[language_end + 1..translated_end])
                .map_err(|_| invalid("翻译关键词不是 UTF-8"))?;
            let compressed = rest[0] == 1;
            let text = &rest[translated_end + 1..];
            if !compressed && text.len() > MAX_TEXT_BYTES {
                return Err("单个文本元数据超过 16 MiB".into());
            }
            let decoded = if compressed {
                inflate(text)?
            } else {
                text.to_vec()
            };
            (
                decoded,
                TextFormat::International {
                    prefix: data[..split + 2 + translated_end].to_vec(),
                    compressed,
                },
            )
        }
    };
    if decoded.len() > MAX_TEXT_BYTES {
        return Err("单个文本元数据超过 16 MiB".into());
    }
    if decoded.contains(&0) {
        return Err(invalid("文本包含空字符"));
    }
    let text = match &format {
        TextFormat::International { .. } => {
            String::from_utf8(decoded).map_err(|_| invalid("国际文本不是 UTF-8"))?
        }
        // 部分生成工具在 tEXt 中写 UTF-8；其它字节仍按 PNG 的 Latin-1 读取。
        TextFormat::Latin { .. } => String::from_utf8(decoded.clone())
            .unwrap_or_else(|_| decoded.iter().map(|b| char::from(*b)).collect()),
    };
    if text.len() > MAX_TEXT_BYTES {
        return Err("单个文本元数据超过 16 MiB".into());
    }
    Ok(TextChunk {
        keyword: keyword.iter().map(|b| char::from(*b)).collect(),
        keyword_bytes: keyword.to_vec(),
        text,
        format,
    })
}

fn compress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .map_err(|e| format!("压缩文本失败: {e}"))?;
    encoder.finish().map_err(|e| format!("压缩文本失败: {e}"))
}

fn encode_text(source: &TextChunk, text: &str) -> Result<Vec<u8>, String> {
    let mut data = source.keyword_bytes.clone();
    data.push(0);
    let (kind, payload) = match &source.format {
        TextFormat::Latin { compressed } if text.chars().all(|c| u32::from(c) <= 255) => {
            let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
            if *compressed {
                data.push(0);
                (b"zTXt", compress(&bytes)?)
            } else {
                (b"tEXt", bytes)
            }
        }
        format => {
            let compressed = match format {
                TextFormat::Latin { compressed } => {
                    data.extend_from_slice(&[u8::from(*compressed), 0, 0, 0]);
                    *compressed
                }
                TextFormat::International { prefix, compressed } => {
                    data = prefix.clone();
                    *compressed
                }
            };
            (
                b"iTXt",
                if compressed {
                    compress(text.as_bytes())?
                } else {
                    text.as_bytes().to_vec()
                },
            )
        }
    };
    data.extend_from_slice(&payload);
    Ok(make_chunk(kind, &data))
}

fn make_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
    chunk
}

fn apply_edits(
    bytes: &[u8],
    chunks: &[Chunk],
    edits: Vec<SdMetadataEdit>,
) -> Result<Vec<u8>, String> {
    let mut replacements = BTreeMap::new();
    for edit in edits {
        let source = chunks
            .get(edit.index)
            .and_then(|chunk| chunk.text.as_ref())
            .ok_or("文本条目不存在，请重新读取图片")?;
        if edit.text.len() > MAX_TEXT_BYTES {
            return Err("单个文本元数据超过 16 MiB".into());
        }
        if edit.text.contains('\0') {
            return Err("文本不能包含空字符".into());
        }
        if replacements.contains_key(&edit.index) {
            return Err("同一文本条目不能重复修改".into());
        }
        if matches!(source.keyword.as_str(), "prompt" | "workflow" | "Comment")
            && serde_json::from_str::<serde_json::Value>(&source.text)
                .is_ok_and(|v| v.is_object() || v.is_array())
            && !serde_json::from_str::<serde_json::Value>(&edit.text)
                .is_ok_and(|v| v.is_object() || v.is_array())
        {
            return Err(format!("{} 必须是有效的 JSON 对象或数组", source.keyword));
        }
        let encoded = if edit.text == source.text {
            bytes[chunks[edit.index].raw.clone()].to_vec()
        } else {
            encode_text(source, &edit.text)?
        };
        replacements.insert(edit.index, encoded);
    }
    let mut output = SIGNATURE.to_vec();
    for (index, chunk) in chunks.iter().enumerate() {
        output.extend_from_slice(
            replacements
                .get(&index)
                .map(Vec::as_slice)
                .unwrap_or(&bytes[chunk.raw.clone()]),
        );
        if output.len() > MAX_FILE_BYTES {
            return Err("保存后的图片超过 512 MiB".into());
        }
    }
    Ok(output)
}

fn replace_checked(
    path: &Path,
    bytes: &[u8],
    expected: &str,
    permissions: Permissions,
    before_commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let stage = path.with_file_name(format!(
        ".purin-metadata-{}-{}.png",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage)
        .map_err(|e| format!("无法创建临时文件: {e}"))?;
    let result = (|| {
        super::config_paths::write_file_atomic(&stage, bytes)
            .map_err(|e| format!("保存元数据失败: {e}"))?;
        fs::set_permissions(&stage, permissions).map_err(|e| format!("保留文件权限失败: {e}"))?;
        before_commit()?;
        check_revision(&read_bytes(path)?, expected)?;
        fs::rename(&stage, path).map_err(|e| format!("替换图片失败: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{
        read_document_sync as read_sd_metadata_document,
        save_document_sync as save_sd_metadata_document,
    };
    use crate::commands::test_support::TempDir;
    use std::io::Cursor;

    fn png(extra: &[Vec<u8>]) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([x as u8 * 40, y as u8 * 90, 123, 177])
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        let mut result = bytes[..33].to_vec();
        for chunk in extra {
            result.extend_from_slice(chunk);
        }
        result.extend_from_slice(&bytes[33..]);
        result
    }

    fn text(key: &str, value: &[u8], compressed: bool) -> Vec<u8> {
        let mut data = key.as_bytes().to_vec();
        data.push(0);
        if compressed {
            data.push(0);
            data.extend_from_slice(&compress(value).unwrap());
        } else {
            data.extend_from_slice(value);
        }
        make_chunk(if compressed { b"zTXt" } else { b"tEXt" }, &data)
    }

    fn international(key: &str, value: &str, compressed: bool) -> Vec<u8> {
        let mut data = key.as_bytes().to_vec();
        data.extend_from_slice(&[0, u8::from(compressed), 0]);
        data.extend_from_slice("zh-CN\0工作流\0".as_bytes());
        data.extend_from_slice(&if compressed {
            compress(value.as_bytes()).unwrap()
        } else {
            value.as_bytes().to_vec()
        });
        make_chunk(b"iTXt", &data)
    }

    fn fixture(extra: &[Vec<u8>]) -> (TempDir, String, Vec<u8>) {
        let root = TempDir::new("sd_metadata_edit");
        let path = root.join("图 片.PNG");
        let bytes = png(extra);
        fs::write(&path, &bytes).unwrap();
        (root, path.to_string_lossy().into_owned(), bytes)
    }

    fn edit(index: usize, value: &str) -> SdMetadataEdit {
        SdMetadataEdit {
            index,
            text: value.into(),
        }
    }

    #[test]
    fn sd_metadata_roundtrip_preserves_pixels_and_every_untouched_chunk() {
        let mut profile = b"Fixture\0\0".to_vec();
        profile.extend_from_slice(&compress(b"fixture profile").unwrap());
        let (_root, path, original) = fixture(&[
            make_chunk(b"iCCP", &profile),
            make_chunk(b"vpAg", b"unknown\0metadata"),
            text(
                "parameters",
                b"old prompt\nNegative prompt: bad\nSteps: 20",
                false,
            ),
            text("Description", b"first duplicate", false),
            text("Description", b"second duplicate", false),
        ]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        assert_eq!(before.metadata.unwrap().positive, "old prompt");
        let after = save_sd_metadata_document(
            path.clone(),
            before.revision,
            vec![
                edit(3, "新提示词\nNegative prompt: 模糊\nSteps: 35"),
                edit(5, "only second changed"),
            ],
        )
        .unwrap();
        assert_eq!(after.metadata.as_ref().unwrap().positive, "新提示词");
        assert_eq!(after.metadata.as_ref().unwrap().params, "Steps: 35");
        assert_eq!(after.entries[1].text, "first duplicate");
        assert_eq!(after.entries[2].text, "only second changed");
        let saved = fs::read(&path).unwrap();
        let old_chunks = parse_png(&original).unwrap();
        let new_chunks = parse_png(&saved).unwrap();
        assert_eq!(old_chunks.len(), new_chunks.len());
        for index in 0..old_chunks.len() {
            if index != 3 && index != 5 {
                assert_eq!(
                    original[old_chunks[index].raw.clone()],
                    saved[new_chunks[index].raw.clone()]
                );
            }
        }
        let pixels = |bytes: &[u8]| {
            image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .unwrap()
                .to_rgba8()
        };
        assert_eq!(pixels(&original), pixels(&saved));
        assert_ne!(revision(&original), after.revision);
        assert_eq!(revision(&saved), after.revision);
    }

    #[test]
    fn sd_metadata_unicode_compression_and_international_headers_roundtrip() {
        let (_root, path, _) = fixture(&[
            text("Latin", b"caf\xe9", false),
            text("Compressed", b"original", true),
            international("workflow", r#"{"nodes":[]}"#, true),
            international("International", "旧文本", false),
        ]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        assert_eq!(before.entries[0].text, "café");
        let after = save_sd_metadata_document(
            path.clone(),
            before.revision,
            vec![
                edit(1, "français"),
                edit(2, "中文压缩 😀"),
                edit(3, r#"{"nodes":["中文😀"]}"#),
                edit(4, "新文本 🌸"),
            ],
        )
        .unwrap();
        assert_eq!(
            after
                .entries
                .iter()
                .map(|v| v.text.as_str())
                .collect::<Vec<_>>(),
            [
                "français",
                "中文压缩 😀",
                r#"{"nodes":["中文😀"]}"#,
                "新文本 🌸"
            ]
        );
        let bytes = fs::read(path).unwrap();
        let chunks = parse_png(&bytes).unwrap();
        assert_eq!(
            &bytes[chunks[1].raw.start + 4..chunks[1].raw.start + 8],
            b"tEXt"
        );
        assert!(matches!(
            chunks[2].text.as_ref().unwrap().format,
            TextFormat::International {
                compressed: true,
                ..
            }
        ));
        for index in [3, 4] {
            let TextFormat::International { prefix, .. } =
                &chunks[index].text.as_ref().unwrap().format
            else {
                panic!("iTXt expected")
            };
            assert!(prefix.ends_with("zh-CN\0工作流\0".as_bytes()));
        }
    }

    #[tokio::test]
    async fn sd_metadata_saved_text_matches_single_read_and_directory_scan() {
        for compressed in [false, true] {
            for (initial, edited) in [("old", "café français"), ("中文旧提示", "中文新提示 😀")]
            {
                let (root, path, _) =
                    fixture(&[text("parameters", initial.as_bytes(), compressed)]);
                let before = read_sd_metadata_document(path.clone()).unwrap();
                assert_eq!(before.entries[0].text, initial);
                let saved =
                    save_sd_metadata_document(path.clone(), before.revision, vec![edit(1, edited)])
                        .unwrap();
                let single = crate::commands::sd_metadata::read_single_sd_metadata(path)
                    .unwrap()
                    .unwrap();
                assert_eq!(single.positive, edited);
                assert_eq!(single.positive, saved.metadata.unwrap().positive);
                let app = tauri::test::mock_app();
                let scan = crate::commands::sd_metadata::scan_sd_metadata(
                    app.handle().clone(),
                    root.to_string_lossy().into_owned(),
                    Some(false),
                )
                .await
                .unwrap();
                assert_eq!(scan.has_meta_count, 1);
                assert_eq!(scan.items[0].positive, edited);
            }
        }
    }

    #[test]
    fn sd_metadata_json_validation_preserves_exact_text_and_large_seeds() {
        let prompt = r#"{"1":{"class_type":"CLIPTextEncode","inputs":{"text":"中文"}},"2":{"class_type":"KSampler","inputs":{"seed":18446744073709551615,"steps":30,"cfg":7.5}}}"#;
        let (_root, path, _) = fixture(&[
            text("prompt", prompt.as_bytes(), true),
            text("Description", b"old", false),
        ]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        let edited = prompt.replace("\"steps\":30", "\"steps\":35");
        let after =
            save_sd_metadata_document(path, before.revision, vec![edit(1, &edited)]).unwrap();
        assert_eq!(after.entries[0].text, edited);
        let summary = after.metadata.unwrap();
        assert!(summary.params.contains("Seed: 18446744073709551615"));
        assert!(summary.params.contains("Steps: 35"));
        assert_eq!(summary.positive, "中文");
    }

    #[test]
    fn sd_metadata_invalid_json_or_indices_never_change_the_file() {
        let (_root, path, original) = fixture(&[
            text("prompt", br#"{"1":{}}"#, false),
            text("workflow", b"[]", true),
            text("Comment", b"{}", false),
        ]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        for index in [1, 2, 3] {
            for value in ["{broken", "null", "42", "\"a string\""] {
                let result = save_sd_metadata_document(
                    path.clone(),
                    before.revision.clone(),
                    vec![edit(index, value)],
                );
                assert!(result.unwrap_err().contains("JSON"));
                assert_eq!(fs::read(&path).unwrap(), original);
            }
        }
        for edits in [
            vec![edit(0, "not text")],
            vec![edit(999, "missing")],
            vec![edit(1, "{}"), edit(1, "[]")],
            vec![edit(1, "{}\0")],
        ] {
            assert!(
                save_sd_metadata_document(path.clone(), before.revision.clone(), edits).is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn sd_metadata_stale_and_concurrent_revisions_are_rejected() {
        let (_root, path, _) = fixture(&[text("Description", b"original", false)]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|value| {
                let (path, revision, barrier) =
                    (path.clone(), before.revision.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    save_sd_metadata_document(path, revision, vec![edit(1, value)])
                })
            })
            .collect();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(outcomes
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap()
            .contains("已被修改"));
        let saved = fs::read(&path).unwrap();
        assert!(
            save_sd_metadata_document(path.clone(), before.revision, vec![edit(1, "stale")])
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), saved);
    }

    #[test]
    fn sd_metadata_staging_failures_and_external_changes_preserve_the_current_file() {
        let (root, path, original) = fixture(&[text("Description", b"original", false)]);
        let path = Path::new(&path);
        let updated = png(&[text("Description", b"new", false)]);
        let permissions = fs::metadata(path).unwrap().permissions();
        assert!(replace_checked(
            path,
            &updated,
            &revision(&original),
            permissions.clone(),
            || Err("fixture failure".into())
        )
        .is_err());
        assert_eq!(fs::read(path).unwrap(), original);
        let external = png(&[text("Description", b"external edit", false)]);
        let result = replace_checked(path, &updated, &revision(&original), permissions, || {
            fs::write(path, &external).unwrap();
            Ok(())
        });
        assert!(result.unwrap_err().contains("已被修改"));
        assert_eq!(fs::read(path).unwrap(), external);
        assert_eq!(fs::read_dir(root.to_path_buf()).unwrap().count(), 1);
    }

    #[test]
    fn sd_metadata_readonly_files_are_not_replaced_and_permissions_are_preserved() {
        let (_root, path, original) = fixture(&[text("Description", b"old", false)]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        assert!(save_sd_metadata_document(
            path.clone(),
            before.revision.clone(),
            vec![edit(1, "new")]
        )
        .unwrap_err()
        .contains("只读"));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::set_permissions(&path, original_permissions.clone()).unwrap();
        save_sd_metadata_document(path.clone(), before.revision, vec![edit(1, "new")]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode(),
                original_permissions.mode()
            );
        }
    }

    #[test]
    fn sd_metadata_corrupt_chunks_and_unsupported_files_are_rejected() {
        let (_root, path, original) = fixture(&[text("Description", b"valid", false)]);
        let mut bad_crc = original.clone();
        bad_crc[33 + 8] ^= 1;
        let mut bad_length = original.clone();
        bad_length[33..37].copy_from_slice(&u32::MAX.to_be_bytes());
        let truncated = original[..original.len() - 3].to_vec();
        let mut zipped = compress(b"text").unwrap();
        zipped.truncate(zipped.len() - 2);
        let bad_zlib = png(&[make_chunk(
            b"zTXt",
            &[b"key\0\0".as_slice(), &zipped].concat(),
        )]);
        for bytes in [
            b"not PNG".to_vec(),
            bad_crc,
            bad_length,
            truncated,
            bad_zlib,
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(read_sd_metadata_document(path.clone()).is_err());
            assert!(save_sd_metadata_document(
                path.clone(),
                revision(&bytes),
                vec![edit(1, "change")]
            )
            .is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        assert!(read_sd_metadata_document("photo.jpg".into())
            .unwrap_err()
            .contains("PNG"));
    }

    #[test]
    fn sd_metadata_noop_preserves_bytes_and_size_limits_apply_before_writing() {
        let (_root, path, original) = fixture(&[text("Description", b"old", true)]);
        let before = read_sd_metadata_document(path.clone()).unwrap();
        let same =
            save_sd_metadata_document(path.clone(), before.revision.clone(), vec![edit(1, "old")])
                .unwrap();
        assert_eq!(same.revision, before.revision);
        assert_eq!(fs::read(&path).unwrap(), original);
        let too_large = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(save_sd_metadata_document(
            path.clone(),
            before.revision,
            vec![edit(1, &too_large)]
        )
        .unwrap_err()
        .contains("16 MiB"));
        assert_eq!(fs::read(&path).unwrap(), original);
        let bomb = png(&[text("Description", too_large.as_bytes(), true)]);
        fs::write(&path, bomb).unwrap();
        assert!(read_sd_metadata_document(path.clone())
            .unwrap_err()
            .contains("16 MiB"));
        File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(read_sd_metadata_document(path)
            .unwrap_err()
            .contains("512 MiB"));
    }
}
