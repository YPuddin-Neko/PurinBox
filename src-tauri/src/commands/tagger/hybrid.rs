use std::io::Write;
use std::path::{Path, PathBuf};

use crate::commands::{collect_files_matching, collect_image_files_with_recursive};

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

/// `path` 是 `draft_path` 产生的中间标签文件（`*.purin-local-txt` / `*.purin-local-json`）时返回它的格式。
/// 后缀取自 `draft_path` 本身，两处不会各写一份。
pub(crate) fn draft_label_format(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?;
    ["txt", "json"]
        .into_iter()
        .find(|format| draft_path(Path::new("x"), format).extension() == Some(extension))
}

/// 是不是辅助打标的中间标签文件（见 `draft_label_format`）
pub(crate) fn is_draft_path(path: &Path) -> bool {
    draft_label_format(path).is_some()
}

pub(crate) fn source_path(image: &Path, format: &str) -> PathBuf {
    let draft = draft_path(image, format);
    if format == "txt" && !draft.exists() {
        return draft_path(image, "json");
    }
    draft
}

/// 准备阶段扫描的结果
#[derive(Debug, Default)]
pub(crate) struct Preparation {
    pub total: u32,
    /// 已有标签复制成草稿的图片数
    pub copied: u32,
    /// 没有可用标签的图片数
    pub unlabeled: u32,
    /// JSON 输出时只有 txt 标签、要按词表转换成 JSON 草稿的图片
    pub to_convert: Vec<PathBuf>,
    /// 准备失败的图片和原因
    pub failed: Vec<(PathBuf, String)>,
}

/// 辅助打标的准备阶段：先清掉输入目录下残留的全部草稿（包括图片已改名或删除后留下的），
/// 再按输出格式 `format` 给每张图准备草稿——已有同格式标签（txt 输出时也可以是 JSON 标签）
/// 就复制一份；JSON 输出且只有 txt 标签的放进 `to_convert`。原标签保持不动，直到 VLM 成功写回。
///
/// 取消时停在当前这张，返回已扫描部分的计数。
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
    clear_residual_drafts(input, recursive, false)?;
    let mut result = Preparation {
        total: files.len() as u32,
        ..Default::default()
    };
    for image in files {
        if super::inference::is_tagging_cancelled() {
            break;
        }
        clear_drafts(&image)?;
        let reuse = if super::has_label(&image, format) {
            Some(format)
        } else if format == "txt" && super::has_label(&image, "json") {
            Some("json")
        } else {
            None
        };
        if let Some(source) = reuse {
            match std::fs::copy(image.with_extension(source), draft_path(&image, source)) {
                Ok(_) => result.copied += 1,
                Err(e) => result
                    .failed
                    .push((image, format!("准备已有标签失败: {}", e))),
            }
        } else if format == "json" && super::has_label(&image, "txt") {
            result.to_convert.push(image);
        } else {
            result.unlabeled += 1;
        }
    }
    Ok(result)
}

/// 删除 `input` 目录下（按 `recursive`，剪掉产物目录）的草稿文件；`orphans_only` 时只删图片已不存在的。
/// `input` 是单张图片时什么都不做，它自己的草稿用 `clear_drafts`
pub(crate) fn clear_residual_drafts(
    input: &Path,
    recursive: bool,
    orphans_only: bool,
) -> Result<(), String> {
    if !input.is_dir() {
        return Ok(());
    }
    for draft in collect_files_matching(input, recursive, None, is_draft_path)? {
        // 去掉草稿后缀就是图片路径
        if orphans_only && draft.with_extension("").is_file() {
            continue;
        }
        remove_draft(&draft)?;
    }
    Ok(())
}

pub(crate) fn clear_drafts(image: &Path) -> Result<(), String> {
    remove_draft(&draft_path(image, "txt"))?;
    remove_draft(&draft_path(image, "json"))
}

fn remove_draft(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清理中间标签失败: {}", e)),
    }
}

/// 写正式标签，返回是否写入。图片已有非空标签（`super::has_labels`）时不写，其余同 `write_unless_labeled`
pub(crate) fn write_final(image: &Path, output: &Path, content: &str) -> Result<bool, String> {
    if super::has_labels(image) {
        return Ok(false);
    }
    write_unless_labeled(output, content)
}

/// 写标签文件，返回是否写入：目标不存在时独占创建；目标已存在但是空文件（没有标签可保护）时
/// 先写临时文件再改名替换。非空的目标文件绝不覆盖
fn write_unless_labeled(output: &Path, content: &str) -> Result<bool, String> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
    {
        Ok(file) => write_new(file, output, content).map(|()| true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => replace_empty(output, content),
        Err(e) => Err(format!("写入失败: {}", e)),
    }
}

fn write_new(mut file: std::fs::File, path: &Path, content: &str) -> Result<(), String> {
    if let Err(e) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("写入失败: {}", e));
    }
    Ok(())
}

fn is_empty_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == 0)
}

/// `output` 是空文件时整体替换成 `content`，返回是否替换
fn replace_empty(output: &Path, content: &str) -> Result<bool, String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    if !is_empty_file(output) {
        return Ok(false);
    }
    let name = output.file_name().unwrap_or_default().to_string_lossy();
    let temp = output.with_file_name(format!(
        ".{}.tmp-{}-{}",
        name,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("写入失败: {}", e))?;
    write_new(file, &temp, content)?;
    // 写临时文件期间别处给它写进了内容，就放弃替换
    if !is_empty_file(output) {
        let _ = std::fs::remove_file(&temp);
        return Ok(false);
    }
    if let Err(e) = std::fs::rename(&temp, output) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("写入失败: {}", e));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tagger::has_labels;
    use crate::commands::test_support::TempDir;

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
            clear_drafts(&image).unwrap();
            clear_drafts(&image).unwrap();
            assert!(!draft.exists());
        }
        for extension in ["txt", "json"] {
            let label = image.with_extension(extension);
            std::fs::write(&label, "").unwrap();
            assert!(!has_labels(&image), "空文件不算已有标签");
            std::fs::write(&label, " ").unwrap();
            assert!(has_labels(&image));
            std::fs::remove_file(label).unwrap();
        }
    }

    #[test]
    fn draft_paths_are_recognized_by_format() {
        for image in ["a.png", "/data/子目录/图 片.JPG", "noext"] {
            let image = Path::new(image);
            for format in ["txt", "json"] {
                let draft = draft_path(image, format);
                assert_eq!(
                    draft_label_format(&draft),
                    Some(format),
                    "{}",
                    draft.display()
                );
                assert!(is_draft_path(&draft));
                assert_eq!(draft.with_extension(""), image);
            }
        }
        for other in [
            "a.png",
            "a.txt",
            "a.json",
            "a.png.purin-local-png",
            "a.png.PURIN-LOCAL-TXT",
            "a.purin-local-txt.png",
            ".purin-local-txt",
            "",
        ] {
            assert_eq!(draft_label_format(Path::new(other)), None, "{other}");
            assert!(!is_draft_path(Path::new(other)));
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
            assert!(prepared.to_convert.is_empty());
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
        assert_eq!(prepared.to_convert, std::slice::from_ref(&image));
        assert_eq!(prepared.copied, 0);
        assert!(!draft_path(&image, "json").exists());
    }

    /// 准备阶段清掉全部残留草稿（含图片已不在的），空标签文件不复用，计数覆盖每一张图
    #[test]
    fn preparation_clears_residual_drafts_and_counts_every_image() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        super::super::inference::reset_tagging_cancel();
        let root = TempDir::new("hybrid_residual");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::create_dir_all(root.join("Fail")).unwrap();
        for name in [
            "reuse.png",
            "convert.png",
            "empty.png",
            "none.png",
            "nested/deep.png",
        ] {
            std::fs::write(root.join(name), "image").unwrap();
        }
        std::fs::write(root.join("reuse.json"), "{}").unwrap();
        std::fs::write(root.join("convert.txt"), "solo").unwrap();
        std::fs::write(root.join("empty.json"), "").unwrap();
        std::fs::write(root.join("empty.txt"), "").unwrap();
        let orphans = [
            root.join("renamed.png.purin-local-txt"),
            root.join("deleted.jpg.purin-local-json"),
            root.join("nested/gone.png.purin-local-txt"),
        ];
        let stale = draft_path(&root.join("none.png"), "json");
        let in_fail = root.join("Fail/x.png.purin-local-txt");
        for draft in orphans.iter().chain([&stale, &in_fail]) {
            std::fs::write(draft, "stale").unwrap();
        }

        let prepared = prepare_sources(&root, false, "json").unwrap();
        assert_eq!(
            (prepared.total, prepared.copied, prepared.unlabeled),
            (4, 1, 2)
        );
        assert_eq!(prepared.to_convert, [root.join("convert.png")]);
        assert!(prepared.failed.is_empty());
        assert!(!orphans[0].exists() && !orphans[1].exists() && !stale.exists());
        assert!(orphans[2].exists(), "不递归时不动子目录");
        assert!(draft_path(&root.join("reuse.png"), "json").exists());
        assert!(!draft_path(&root.join("empty.png"), "json").exists());

        let prepared = prepare_sources(&root, true, "txt").unwrap();
        assert_eq!(
            (prepared.total, prepared.copied, prepared.unlabeled),
            (5, 2, 3)
        );
        assert!(prepared.to_convert.is_empty());
        assert!(!orphans[2].exists());
        assert!(in_fail.exists(), "产物目录不扫描");
        assert!(draft_path(&root.join("convert.png"), "txt").exists());
        assert!(draft_path(&root.join("reuse.png"), "json").exists());
    }

    #[test]
    fn residual_clearing_can_keep_drafts_of_existing_images() {
        let root = TempDir::new("hybrid_orphans");
        let image = root.join("a.png");
        std::fs::write(&image, "image").unwrap();
        let kept = draft_path(&image, "txt");
        let orphan = draft_path(&root.join("b.png"), "txt");
        for draft in [&kept, &orphan] {
            std::fs::write(draft, "local").unwrap();
        }
        clear_residual_drafts(&root, false, true).unwrap();
        assert!(kept.exists());
        assert!(!orphan.exists());
        clear_residual_drafts(&image, false, false).unwrap();
        assert!(kept.exists(), "单张图片输入时不扫描目录");
        clear_residual_drafts(&root, false, false).unwrap();
        assert!(!kept.exists());
    }

    #[test]
    fn cancelled_preparation_stops_scanning() {
        let _lock = super::super::TAGGER_TEST_LOCK.lock().unwrap();
        let root = TempDir::new("hybrid_prepare_cancel");
        for name in ["a.png", "b.png"] {
            std::fs::write(root.join(name), "image").unwrap();
            std::fs::write(root.join(name).with_extension("txt"), "solo").unwrap();
        }
        super::super::inference::cancel_tagging();
        let prepared = prepare_sources(&root, false, "txt").unwrap();
        super::super::inference::reset_tagging_cancel();
        assert_eq!(
            (prepared.total, prepared.copied, prepared.unlabeled),
            (2, 0, 0)
        );
        assert!(!draft_path(&root.join("a.png"), "txt").exists());
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

    /// 空的正式标签不算已有标签：VLM 结果整体替换它，不留临时文件；非空的照样不动
    #[test]
    fn empty_final_labels_are_replaced() {
        let root = TempDir::new("hybrid_final_empty");
        for format in ["txt", "json"] {
            let image = root.join(format!("{}.png", format));
            let output = image.with_extension(format);
            std::fs::write(&output, "").unwrap();
            assert!(write_final(&image, &output, "refined").unwrap());
            assert_eq!(std::fs::read_to_string(&output).unwrap(), "refined");
            assert!(!write_final(&image, &output, "again").unwrap());
            assert_eq!(std::fs::read_to_string(&output).unwrap(), "refined");
        }
        let image = root.join("both.png");
        std::fs::write(image.with_extension("json"), "").unwrap();
        std::fs::write(image.with_extension("txt"), "").unwrap();
        assert!(write_final(&image, &image.with_extension("txt"), "").unwrap());
        assert!(write_final(&image, &image.with_extension("txt"), "solo").unwrap());
        assert_eq!(
            std::fs::read_to_string(image.with_extension("txt")).unwrap(),
            "solo"
        );
        assert_eq!(
            std::fs::read_to_string(image.with_extension("json")).unwrap(),
            ""
        );
        let leftovers: Vec<_> = std::fs::read_dir(&*root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
