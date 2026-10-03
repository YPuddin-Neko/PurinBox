use std::io::Write;
use std::path::{Path, PathBuf};

use crate::commands::collect_image_files_with_recursive;

pub(crate) fn has_labels(image: &Path) -> bool {
    ["txt", "json"]
        .iter()
        .any(|ext| image.with_extension(ext).exists())
}

pub(crate) fn draft_path(image: &Path, format: &str) -> PathBuf {
    // 保留图片扩展名，避免同目录中 a.png 和 a.jpg 共用中间标签。
    let mut name = image.as_os_str().to_os_string();
    name.push(if format == "json" {
        ".purin-local-json"
    } else {
        ".purin-local-txt"
    });
    PathBuf::from(name)
}

pub(crate) fn source_path(image: &Path, format: &str) -> PathBuf {
    let draft = draft_path(image, format);
    if format == "txt" && !draft.exists() {
        return draft_path(image, "json");
    }
    draft
}

pub(crate) struct Preparation {
    pub total: u32,
    pub copied: u32,
    pub needs_json_conversion: bool,
}

pub(crate) fn prepare_sources(
    input: &Path,
    recursive: bool,
    format: &str,
) -> Result<Preparation, String> {
    if !matches!(format, "txt" | "json") {
        return Err("不支持的标签格式".into());
    }
    let files = collect_image_files_with_recursive(input, recursive)?;
    if files.is_empty() {
        return Err("输入目录中没有找到图片文件".into());
    }
    let mut result = Preparation {
        total: files.len() as u32,
        copied: 0,
        needs_json_conversion: false,
    };
    for image in files {
        if super::inference::is_tagging_cancelled() {
            return Err("已取消".into());
        }
        clear_drafts(&image)?;
        let source_format = if image.with_extension(format).exists() {
            format
        } else if format == "txt" && image.with_extension("json").exists() {
            "json"
        } else if format == "json" && image.with_extension("txt").exists() {
            result.needs_json_conversion = true;
            continue;
        } else {
            continue;
        };
        // 只复制到中间文件，保留原标签直到 VLM 成功写回。
        std::fs::copy(
            image.with_extension(source_format),
            draft_path(&image, source_format),
        )
        .map_err(|e| format!("准备已有标签失败 [{}]: {}", image.display(), e))?;
        result.copied += 1;
    }
    Ok(result)
}

pub(crate) fn clear_drafts(image: &Path) -> Result<(), String> {
    clear_draft(image, "txt")?;
    clear_draft(image, "json")
}

pub(crate) fn clear_draft(image: &Path, format: &str) -> Result<(), String> {
    match std::fs::remove_file(draft_path(image, format)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清理中间标签失败: {}", e)),
    }
}

pub(crate) fn write_final(image: &Path, output: &Path, content: &str) -> Result<bool, String> {
    if has_labels(image) {
        return Ok(false);
    }
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(format!("写入失败: {}", e)),
    };
    if let Err(e) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(output);
        return Err(format!("写入失败: {}", e));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::llm_client::test_support::TempDir;

    #[test]
    fn intermediate_extensions_do_not_count_as_labels() {
        let root = TempDir::new("hybrid_drafts");
        let image = root.join("图片.png");
        for format in ["txt", "json"] {
            let draft = draft_path(&image, format);
            std::fs::write(&draft, "local tags").unwrap();
            assert!(!has_labels(&image));
            assert_ne!(draft.extension(), image.with_extension(format).extension());
            assert_ne!(draft, draft_path(&image.with_extension("jpg"), format));
            assert_eq!(source_path(&image, format), draft);
            clear_draft(&image, format).unwrap();
            clear_draft(&image, format).unwrap();
            assert!(!draft.exists());
        }
        for extension in ["txt", "json"] {
            let label = image.with_extension(extension);
            std::fs::write(&label, "").unwrap();
            assert!(has_labels(&image));
            std::fs::remove_file(label).unwrap();
        }
    }

    #[test]
    fn existing_sources_are_copied_without_changing_originals() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        super::super::inference::reset_tagging_cancel();
        let root = TempDir::new("hybrid_prefer");
        let image = root.join("a.png");
        std::fs::write(&image, "image").unwrap();
        for format in ["txt", "json"] {
            std::fs::write(image.with_extension(format), "existing").unwrap();
            let prepared = prepare_sources(&root, false, format).unwrap();
            assert_eq!(prepared.copied, 1);
            assert!(!prepared.needs_json_conversion);
            assert_eq!(
                std::fs::read_to_string(image.with_extension(format)).unwrap(),
                "existing"
            );
            assert_eq!(source_path(&image, format), draft_path(&image, format));
            assert_eq!(
                std::fs::read_to_string(draft_path(&image, format)).unwrap(),
                "existing"
            );
        }
        std::fs::remove_file(image.with_extension("txt")).unwrap();
        let prepared = prepare_sources(&image, false, "txt").unwrap();
        assert_eq!(prepared.copied, 1);
        assert_eq!(source_path(&image, "txt"), draft_path(&image, "json"));
        std::fs::remove_file(image.with_extension("json")).unwrap();
        std::fs::write(image.with_extension("txt"), "solo").unwrap();
        let prepared = prepare_sources(&image, false, "json").unwrap();
        assert!(prepared.needs_json_conversion);
        assert_eq!(prepared.copied, 0);
        assert!(!draft_path(&image, "json").exists());
    }

    #[test]
    fn final_labels_are_not_overwritten() {
        let root = TempDir::new("hybrid_final");
        for format in ["txt", "json"] {
            let image = root.join(format!("{}.png", format));
            let output = image.with_extension(format);
            let other = image.with_extension(if format == "txt" { "json" } else { "txt" });
            std::fs::write(&other, "external").unwrap();
            assert!(!write_final(&image, &output, "new").unwrap());
            assert!(!output.exists());
            std::fs::remove_file(other).unwrap();
            assert!(write_final(&image, &output, "refined").unwrap());
            assert!(!write_final(&image, &output, "overwrite").unwrap());
            assert_eq!(std::fs::read_to_string(output).unwrap(), "refined");
        }
    }
}
