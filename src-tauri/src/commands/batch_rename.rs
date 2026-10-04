use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::batch::{FileBatch, FileOutcome, RunEvents};
use super::{
    collect_image_files_with_recursive, file_name_lossy, path_key_ci, tag_sidecars, ProcessResult,
    ProgressEvent,
};

const EVENT: &str = "rename-progress";

/// 冲突报错里最多列出的文件名数
const MAX_LISTED_CONFLICTS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameOptions {
    pub input_path: String,
    /// 文件名前缀
    pub prefix: String,
    /// 起始编号
    pub start_number: u32,
    /// 编号位数（例如 4 → 0001）
    pub digit_count: u32,
    /// 是否打乱顺序
    pub shuffle: bool,
    /// 打乱用的随机种子。预览与执行传同一种子时顺序一致；
    /// 不传（如工作流直接执行）则随机
    #[serde(default)]
    pub shuffle_seed: Option<u64>,
    /// 是否同步重命名标签文件（.txt, .json, .caption）
    #[serde(default)]
    pub rename_tags: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreviewItem {
    pub original: String,
    pub renamed: String,
    /// 共用这个标签文件的图片（多张图同名不同扩展名，如 `x.png` 与 `x.jpg` 共用 `x.txt`）。
    /// 非空时执行不重命名它，`renamed` 与原名相同
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_by: Vec<String>,
    /// 新名已被不在本次改名计划里的文件占着；有这样的项时执行会整批拒绝
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
}

/// 命名计划的一项
struct PlannedRename {
    path: PathBuf,
    new_name: String,
    /// 见 `RenamePreviewItem::shared_by`
    shared_by: Vec<String>,
}

/// 生成重命名预览（不实际执行）
#[tauri::command]
pub async fn preview_rename(options: RenameOptions) -> Result<Vec<RenamePreviewItem>, String> {
    tokio::task::spawn_blocking(move || preview_rename_sync(&options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

fn preview_rename_sync(options: &RenameOptions) -> Result<Vec<RenamePreviewItem>, String> {
    let input = Path::new(&options.input_path);
    let mut files = collect_image_files_with_recursive(input, false)?;

    if options.shuffle {
        apply_shuffle(&mut files, options.shuffle_seed);
    }

    let plan = plan_renames(&files, options);
    let blocked = blocked_flags(&plan);
    Ok(plan
        .into_iter()
        .zip(blocked)
        .map(|(item, blocked)| RenamePreviewItem {
            original: file_name_lossy(&item.path),
            renamed: item.new_name,
            shared_by: item.shared_by,
            blocked,
        })
        .collect())
}

/// 预览与执行共用的命名计划，标签文件紧跟在所属图片之后。
/// 被多张图片共用的标签文件只列一次（跟在第一张之后），保持原名
fn plan_renames(files: &[PathBuf], options: &RenameOptions) -> Vec<PlannedRename> {
    let new_stem = |i: usize| {
        format!(
            "{}{:0>width$}",
            options.prefix,
            options.start_number + i as u32,
            width = options.digit_count as usize
        )
    };
    let sidecars_of = |image: &Path| -> Vec<(&'static str, PathBuf)> {
        if options.rename_tags {
            tag_sidecars(image).filter(|(_, p)| p.is_file()).collect()
        } else {
            Vec::new()
        }
    };
    // 标签文件 → 认领它的图片；大小写不敏感的文件系统上 X.txt 与 x.txt 是同一个文件
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    for image in files {
        for (_, sidecar) in sidecars_of(image) {
            owners
                .entry(path_key_ci(&sidecar))
                .or_default()
                .push(file_name_lossy(image));
        }
    }

    let mut listed = HashSet::new();
    let mut plan = Vec::new();
    for (i, file_path) in files.iter().enumerate() {
        let ext = file_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let stem = new_stem(i);
        plan.push(PlannedRename {
            path: file_path.clone(),
            new_name: format!("{}.{}", stem, ext),
            shared_by: Vec::new(),
        });

        for (tag_ext, sidecar) in sidecars_of(file_path) {
            let key = path_key_ci(&sidecar);
            let sharing = &owners[&key];
            if sharing.len() == 1 {
                plan.push(PlannedRename {
                    path: sidecar,
                    new_name: format!("{}.{}", stem, tag_ext),
                    shared_by: Vec::new(),
                });
            } else if listed.insert(key) {
                plan.push(PlannedRename {
                    new_name: file_name_lossy(&sidecar),
                    path: sidecar,
                    shared_by: sharing.clone(),
                });
            }
        }
    }
    plan
}

/// 执行批量重命名
#[tauri::command]
pub async fn execute_rename<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    options: RenameOptions,
) -> Result<ProcessResult, String> {
    super::spawn_blocking_with_progress(move || execute_rename_sync(&app, &options))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

fn execute_rename_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    options: &RenameOptions,
) -> Result<ProcessResult, String> {
    let input = Path::new(&options.input_path);
    let mut files = collect_image_files_with_recursive(input, false)?;

    if options.shuffle {
        apply_shuffle(&mut files, options.shuffle_seed);
    }

    let (plan, shared): (Vec<_>, Vec<_>) = plan_renames(&files, options)
        .into_iter()
        .partition(|item| item.shared_by.is_empty());

    // 新名被批外文件占着时整批不动：等到第 3 步才发现，前面的文件已经改了名，
    // 被挡住的那一个只能留在旧名，图片和标签就对不上了
    let blocked = blocked_targets(&plan);
    if !blocked.is_empty() {
        return Err(conflict_message(&blocked));
    }

    // Step 1: 先整批改成临时名再改成最终名，避免新名撞上批内尚未改名的文件；
    // 临时名里的下标保证批内不重名，批次 ID 避免与目录里已有的文件同名
    let batch_id = uuid_simple();
    let moves: Vec<Move> = plan
        .into_iter()
        .enumerate()
        .map(|(idx, item)| Move {
            temp: item
                .path
                .with_file_name(format!("__rename_temp_{}_{}", idx, batch_id)),
            target: item.path.with_file_name(&item.new_name),
            original: item.path,
        })
        .collect();

    // Step 2: Rename to temp names
    for (idx, mv) in moves.iter().enumerate() {
        if let Err(e) = std::fs::rename(&mv.original, &mv.temp) {
            // 失败时尽力回滚：把已改为临时名的文件恢复为原名
            let rollback_fails: Vec<String> = moves[..idx]
                .iter()
                .filter_map(|done| {
                    restore(done)
                        .err()
                        .map(|kept| format!("{}: {}", file_name_lossy(&done.original), kept))
                })
                .collect();
            let mut msg = format!("临时重命名失败 {}: {}", mv.original.display(), e);
            if rollback_fails.is_empty() {
                if idx > 0 {
                    msg.push_str(&format!("（已回滚 {} 个文件）", idx));
                }
            } else {
                msg.push_str(&format!(
                    "（已回滚 {} 个文件，{} 个回滚失败: {}）",
                    idx - rollback_fails.len(),
                    rollback_fails.len(),
                    rollback_fails.join("; ")
                ));
            }
            return Err(msg);
        }
    }

    let run = RunEvents::begin(app, EVENT);
    for item in &shared {
        run.emit(ProgressEvent::new(
            "warning",
            format!(
                "[跳过] {} 被 {} 共用，未重命名",
                file_name_lossy(&item.path),
                item.shared_by.join("、")
            ),
        ));
    }

    // Step 3: Rename to final names
    let originals: Vec<PathBuf> = moves.iter().map(|mv| mv.original.clone()).collect();
    let mut pending = moves.iter();
    // 不提供取消：这一步中途停下，剩下的文件会滞留在临时名
    let never_cancelled = AtomicBool::new(false);
    Ok(FileBatch::new(app, EVENT, &never_cancelled)
        .in_run(run.run_id())
        .error_prefix("[错误] ")
        .run(
            &originals,
            |item| {
                let mv = pending.next().expect("FileBatch 按顺序逐个处理文件");
                let new_name = file_name_lossy(&mv.target);
                // 预检之后才出现的文件同样不覆盖（批内文件此时都已移到临时名）
                let renamed = if occupied(&mv.target) {
                    Err("目标文件已存在，跳过以免覆盖".to_string())
                } else {
                    std::fs::rename(&mv.temp, &mv.target).map_err(|e| e.to_string())
                };
                match renamed {
                    Ok(()) => Ok(FileOutcome::done(format!(
                        "[重命名] {} → {}",
                        item.name, new_name
                    ))),
                    Err(e) => {
                        let mut message = format!("重命名为 {} 失败: {}", new_name, e);
                        if let Err(kept) = restore(mv) {
                            message.push_str(&format!("；{}", kept));
                        }
                        Err(message)
                    }
                }
            },
            |c| {
                format!(
                    "重命名完成: 图片 {} 张, 共处理 {} 个文件, 失败 {}",
                    files.len(),
                    c.total,
                    c.failed
                )
            },
        ))
}

/// 执行时的一项：第 2 步从 `original` 移到 `temp`，第 3 步再移到 `target`
struct Move {
    original: PathBuf,
    temp: PathBuf,
    target: PathBuf,
}

/// 路径上已有目录项。悬空的符号链接也算：`exists()` 对它返回 false，rename 却会覆盖它
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// 计划里已被批外文件（保留原名的共用标签、孤儿标签等）占着的新名，按计划顺序
fn blocked_targets(plan: &[PlannedRename]) -> Vec<String> {
    plan.iter()
        .zip(blocked_flags(plan))
        .filter(|(_, blocked)| *blocked)
        .map(|(item, _)| item.new_name.clone())
        .collect()
}

/// 计划里每一项的新名是否被批外文件占着。保留原名的共用标签不改名，不算。
/// 本批要改名的文件第 2 步都会先移到临时名，占着新名不算冲突
fn blocked_flags(plan: &[PlannedRename]) -> Vec<bool> {
    let mut moving: HashMap<String, Vec<&Path>> = HashMap::new();
    for item in plan.iter().filter(|item| item.shared_by.is_empty()) {
        moving
            .entry(path_key_ci(&item.path))
            .or_default()
            .push(&item.path);
    }
    let mut listings: HashMap<PathBuf, HashSet<OsString>> = HashMap::new();
    let mut listed = |path: &Path| {
        let dir = path.parent().unwrap_or(Path::new("."));
        let names = listings.entry(dir.to_path_buf()).or_insert_with(|| {
            std::fs::read_dir(dir)
                .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
                .unwrap_or_default()
        });
        path.file_name().is_some_and(|name| names.contains(name))
    };

    let mut blocked = Vec::with_capacity(plan.len());
    for item in plan {
        let target = item.path.with_file_name(&item.new_name);
        if !item.shared_by.is_empty() || !occupied(&target) {
            blocked.push(false);
            continue;
        }
        // 新名与某个要改名的文件只差大小写时，大小写不敏感的文件系统上它们是同一个文件；
        // 两种写法同时出现在目录里，才是大小写敏感的文件系统上的另一个文件
        let vacated = moving.get(&path_key_ci(&target)).is_some_and(|sources| {
            sources
                .iter()
                .any(|&source| source == target.as_path() || !(listed(source) && listed(&target)))
        });
        blocked.push(!vacated);
    }
    blocked
}

/// 冲突时整批拒绝的报错，最多列出 `MAX_LISTED_CONFLICTS` 个文件名
fn conflict_message(blocked: &[String]) -> String {
    let names = blocked[..blocked.len().min(MAX_LISTED_CONFLICTS)].join("、");
    let existing = if blocked.len() > MAX_LISTED_CONFLICTS {
        format!("{} 等 {} 个文件已存在", names, blocked.len())
    } else {
        format!("{} 已存在", names)
    };
    format!("新文件名与现有文件冲突，未重命名任何文件：{}", existing)
}

/// 把临时名改回原名，免得文件滞留在无扩展名的临时名。原名已被占用（前面的文件刚改成这个名字，
/// 或别的程序新建了它）时不覆盖，文件留在临时名；Err 是写进报错的说明，带上临时名供用户找回
fn restore(mv: &Move) -> Result<(), String> {
    let kept = |reason: String| format!("{}，文件保留为临时名 {}", reason, mv.temp.display());
    if occupied(&mv.original) {
        return Err(kept("原名已被占用".to_string()));
    }
    std::fs::rename(&mv.temp, &mv.original).map_err(|e| kept(format!("改回原名失败（{}）", e)))
}

/// 打乱文件列表。种子相同时顺序确定，用于让预览与执行的映射一致；
/// 无种子（工作流等直接执行场景）时完全随机
fn apply_shuffle(files: &mut [PathBuf], seed: Option<u64>) {
    match seed {
        Some(s) => {
            use rand::SeedableRng;
            let mut rng = rand::rngs::StdRng::seed_from_u64(s);
            files.shuffle(&mut rng);
        }
        None => files.shuffle(&mut rand::rng()),
    }
}

/// 生成简易唯一 ID（避免引入 uuid 库）
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::{capture_events, capture_raw_events};
    use crate::commands::test_support::TempDir;

    fn options(root: &Path, prefix: &str, digits: u32) -> RenameOptions {
        RenameOptions {
            input_path: root.to_string_lossy().into_owned(),
            prefix: prefix.into(),
            start_number: 1,
            digit_count: digits,
            shuffle: false,
            shuffle_seed: None,
            rename_tags: true,
        }
    }

    /// 按 (文件名, 内容) 建文件。执行重命名不解码图片，图片也写成文本，方便核对内容
    fn write_files(root: &Path, files: &[(&str, &str)]) {
        for (name, content) in files {
            std::fs::write(root.join(name), content).unwrap();
        }
    }

    /// 目录里所有文件的 (文件名, 内容)，按文件名排序
    fn snapshot(root: &Path) -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    file_name_lossy(&path),
                    std::fs::read_to_string(&path).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    }

    fn owned(files: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = files
            .iter()
            .map(|(name, content)| (name.to_string(), content.to_string()))
            .collect();
        files.sort();
        files
    }

    /// 第 3 步处理到 `trigger`（它的 processing 事件）时在 `path` 新建文件，模拟预检之后才出现的文件。
    /// 监听器在 emit 里同步执行，文件一定先于这一项的改名出现
    fn create_when_processing<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
        trigger: &'static str,
        path: PathBuf,
    ) {
        use tauri::Listener;
        app.listen_any(EVENT, move |event| {
            let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            if payload["status"] == "processing" && payload["filename"] == trigger {
                std::fs::write(&path, "外部文件").unwrap();
            }
        });
    }

    /// .caption 标签跟着图片走：预览列出它；新名被占用时整批拒绝，挪开占用的文件后执行总数与预览一致
    #[test]
    fn caption_sidecar_is_planned_checked_and_renamed() {
        let root = TempDir::new("rename_caption");
        write_files(
            &root,
            &[
                ("a.png", "image"),
                ("a.caption", "original caption"),
                ("new1.caption", "existing caption"),
            ],
        );
        let options = options(&root, "new", 1);
        let preview = preview_rename_sync(&options).unwrap();
        assert_eq!(preview.len(), 2);
        assert_eq!(preview[1].renamed, "new1.caption");

        let app = tauri::test::mock_app();
        let events = capture_events(app.handle(), EVENT);
        let before = snapshot(&root);
        let err = execute_rename_sync(app.handle(), &options).unwrap_err();
        assert_eq!(
            err,
            "新文件名与现有文件冲突，未重命名任何文件：new1.caption 已存在"
        );
        assert_eq!(snapshot(&root), before);
        assert!(events.lock().unwrap().is_empty());

        std::fs::remove_file(root.join("new1.caption")).unwrap();
        let result = execute_rename_sync(app.handle(), &options).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (2, 0, 2)
        );
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .all(|event| event["total"] == 2));
        assert_eq!(
            snapshot(&root),
            owned(&[("new1.png", "image"), ("new1.caption", "original caption")])
        );
    }

    /// 场景 C：已编号的数据集里 img_03.png、img_03.jpg 共用 img_03.txt，前面新加 a.png 后重新编号。
    /// img_02.txt 的新名 img_03.txt 被保留原名的共用标签占着：整批拒绝，一个文件都不动
    #[test]
    fn name_taken_by_a_shared_sidecar_rejects_the_whole_batch() {
        let root = TempDir::new("rename_shared_blocks");
        let files = [
            ("a.png", "新图A"),
            ("a.txt", "A的标签"),
            ("img_01.png", "图1"),
            ("img_01.txt", "标签1"),
            ("img_02.png", "图2"),
            ("img_02.txt", "标签2"),
            ("img_03.jpg", "图3jpg"),
            ("img_03.png", "图3png"),
            ("img_03.txt", "标签3（共用）"),
        ];
        write_files(&root, &files);
        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);

        let err = execute_rename_sync(app.handle(), &options(&root, "img_", 2)).unwrap_err();

        assert_eq!(
            err,
            "新文件名与现有文件冲突，未重命名任何文件：img_03.txt 已存在"
        );
        assert_eq!(snapshot(&root), owned(&files));
        assert!(log.lock().unwrap().is_empty());
    }

    /// 场景 D：查重只删了图片，留下孤儿标签。孤儿占着某张图标签的新名时整批拒绝——
    /// 前面新加了图（回滚会覆盖丢失），或只是去掉中间空号（空号之后会整体错配），都一个文件不动
    #[test]
    fn name_taken_by_an_orphan_sidecar_rejects_the_whole_batch() {
        let shifted = vec![
            ("a.png", "新图A"),
            ("a.txt", "A的标签"),
            ("img_01.png", "图1"),
            ("img_01.txt", "标签1"),
            ("img_02.png", "图2"),
            ("img_02.txt", "标签2"),
            ("img_03.png", "图3"),
            ("img_03.txt", "标签3"),
            ("img_04.txt", "已删图4的标签"),
        ];
        let gap = vec![
            ("img_01.png", "图1"),
            ("img_01.txt", "标签1"),
            ("img_02.png", "图2"),
            ("img_02.txt", "标签2"),
            ("img_03.txt", "已删图3的标签"),
            ("img_04.png", "图4"),
            ("img_04.txt", "标签4"),
            ("img_05.png", "图5"),
            ("img_05.txt", "标签5"),
        ];
        for (tag, files, blocked) in [
            ("rename_orphan_shift", shifted, "img_04.txt"),
            ("rename_orphan_gap", gap, "img_03.txt"),
        ] {
            let root = TempDir::new(tag);
            write_files(&root, &files);
            let err =
                execute_rename_sync(tauri::test::mock_app().handle(), &options(&root, "img_", 2))
                    .unwrap_err();
            assert_eq!(
                err,
                format!(
                    "新文件名与现有文件冲突，未重命名任何文件：{} 已存在",
                    blocked
                ),
                "{tag}"
            );
            assert_eq!(snapshot(&root), owned(&files), "{tag}");
        }
    }

    /// 预览按行标出新名被批外文件占着的项，与执行时整批拒绝的判断一致；保留原名的共用标签不算
    #[test]
    fn preview_marks_rows_whose_new_name_is_taken() {
        let root = TempDir::new("rename_preview_blocked");
        write_files(
            &root,
            &[
                ("a.png", "新图A"),
                ("a.txt", "A的标签"),
                ("img_01.png", "图1"),
                ("img_01.txt", "标签1"),
                ("img_02.png", "图2"),
                ("img_02.jpg", "图2另存"),
                ("img_02.txt", "共用标签"),
                ("img_04.txt", "孤儿标签"),
            ],
        );
        let preview = preview_rename_sync(&options(&root, "img_", 2)).unwrap();
        let blocked: Vec<(&str, &str)> = preview
            .iter()
            .filter(|item| item.blocked)
            .map(|item| (item.original.as_str(), item.renamed.as_str()))
            .collect();
        // 共用的 img_02.txt 保留原名，挡住 img_01.txt 改成 img_02.txt；
        // 孤儿 img_04.txt 不在任何一项的新名上（新名 img_04 的是图片，它的标签是共用的、不改名），不算冲突
        assert_eq!(blocked, [("img_01.txt", "img_02.txt")]);
        assert!(preview.iter().any(|item| item.original == "img_02.txt"
            && !item.shared_by.is_empty()
            && !item.blocked));
        let json = serde_json::to_value(&preview).unwrap();
        assert!(json[0].get("blocked").is_none());
        assert_eq!(
            execute_rename_sync(tauri::test::mock_app().handle(), &options(&root, "img_", 2))
                .unwrap_err(),
            "新文件名与现有文件冲突，未重命名任何文件：img_02.txt 已存在"
        );
    }

    #[test]
    fn conflict_message_lists_at_most_five_names() {
        let names: Vec<String> = (1..=7).map(|i| format!("img_{:02}.txt", i)).collect();
        assert_eq!(
            conflict_message(&names[..2]),
            "新文件名与现有文件冲突，未重命名任何文件：img_01.txt、img_02.txt 已存在"
        );
        assert_eq!(
            conflict_message(&names),
            "新文件名与现有文件冲突，未重命名任何文件：\
             img_01.txt、img_02.txt、img_03.txt、img_04.txt、img_05.txt 等 7 个文件已存在"
        );
    }

    /// 新名与原名只差大小写不算冲突（大小写不敏感的文件系统上新名"已存在"的正是这个文件自己）
    #[test]
    fn case_only_renames_are_not_conflicts() {
        let root = TempDir::new("rename_case_only");
        write_files(
            &root,
            &[
                ("IMG_01.PNG", "图1"),
                ("IMG_01.txt", "标签1"),
                ("IMG_02.PNG", "图2"),
                ("IMG_02.txt", "标签2"),
            ],
        );
        let result =
            execute_rename_sync(tauri::test::mock_app().handle(), &options(&root, "img_", 2))
                .unwrap();
        assert_eq!(
            (result.success_count, result.fail_count),
            (4, 0),
            "{:?}",
            result.errors
        );
        assert_eq!(
            snapshot(&root),
            owned(&[
                ("img_01.png", "图1"),
                ("img_01.txt", "标签1"),
                ("img_02.png", "图2"),
                ("img_02.txt", "标签2"),
            ])
        );
    }

    /// 竞态：预检之后才出现的文件挡住 img_01.txt 的新名 img_02.txt。这一项失败；它的原名 img_01.txt
    /// 已被 a.txt 改过去的文件占用，回滚不能覆盖，留在临时名并在报错里写明
    #[test]
    fn race_failure_keeps_the_temp_name_when_the_original_is_taken() {
        let root = TempDir::new("rename_race_taken");
        write_files(
            &root,
            &[
                ("a.png", "A图"),
                ("a.txt", "A的标签"),
                ("img_01.png", "图1"),
                ("img_01.txt", "标签1"),
            ],
        );
        let app = tauri::test::mock_app();
        create_when_processing(app.handle(), "img_01.txt", root.join("img_02.txt"));

        let result = execute_rename_sync(app.handle(), &options(&root, "img_", 2)).unwrap();

        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (3, 1, 4)
        );
        let (temps, rest): (Vec<_>, Vec<_>) = snapshot(&root)
            .into_iter()
            .partition(|(name, _)| name.starts_with("__rename_temp_"));
        assert_eq!(
            rest,
            owned(&[
                ("img_01.png", "A图"),
                ("img_01.txt", "A的标签"),
                ("img_02.png", "图1"),
                ("img_02.txt", "外部文件"),
            ])
        );
        assert_eq!(temps.len(), 1, "{temps:?}");
        assert_eq!(temps[0].1, "标签1");
        assert_eq!(
            result.errors,
            vec![format!(
                "img_01.txt: 重命名为 img_02.txt 失败: 目标文件已存在，跳过以免覆盖；\
                 原名已被占用，文件保留为临时名 {}",
                root.join(&temps[0].0).display()
            )]
        );
    }

    /// 竞态时原名还空着：改回原名，不留临时名
    #[test]
    fn race_failure_restores_a_free_original_name() {
        let root = TempDir::new("rename_race_free");
        write_files(
            &root,
            &[
                ("a.png", "A图"),
                ("a.txt", "A的标签"),
                ("img_01.png", "图1"),
                ("img_01.txt", "标签1"),
            ],
        );
        let app = tauri::test::mock_app();
        create_when_processing(app.handle(), "a.png", root.join("img_01.png"));

        let result = execute_rename_sync(app.handle(), &options(&root, "img_", 2)).unwrap();

        assert_eq!(
            result.errors,
            vec!["a.png: 重命名为 img_01.png 失败: 目标文件已存在，跳过以免覆盖".to_string()]
        );
        assert_eq!(
            snapshot(&root),
            owned(&[
                ("a.png", "A图"),
                ("img_01.png", "外部文件"),
                ("img_01.txt", "A的标签"),
                ("img_02.png", "图1"),
                ("img_02.txt", "标签1"),
            ])
        );
    }

    /// `x.png` 与 `x.jpg` 共用 `x.txt`：预览标出冲突、只列一次；执行时不动它，其余照常
    #[test]
    fn shared_sidecar_is_flagged_and_left_alone() {
        let root = TempDir::new("rename_shared");
        image::RgbImage::new(2, 2).save(root.join("x.png")).unwrap();
        image::RgbImage::new(2, 2).save(root.join("x.jpg")).unwrap();
        std::fs::write(root.join("x.txt"), b"shared tags").unwrap();
        image::RgbImage::new(2, 2).save(root.join("y.png")).unwrap();
        std::fs::write(root.join("y.txt"), b"y tags").unwrap();
        let options = options(&root, "img_", 2);

        let preview = preview_rename_sync(&options).unwrap();
        let rows: Vec<(&str, &str, usize)> = preview
            .iter()
            .map(|p| (p.original.as_str(), p.renamed.as_str(), p.shared_by.len()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("x.jpg", "img_01.jpg", 0),
                ("x.txt", "x.txt", 2),
                ("x.png", "img_02.png", 0),
                ("y.png", "img_03.png", 0),
                ("y.txt", "img_03.txt", 0),
            ]
        );
        assert_eq!(preview[1].shared_by, vec!["x.jpg", "x.png"]);
        let json = serde_json::to_value(&preview).unwrap();
        assert!(json[0].get("shared_by").is_none());

        let app = tauri::test::mock_app();
        let log = capture_raw_events(app.handle(), EVENT);
        let result = execute_rename_sync(app.handle(), &options).unwrap();
        assert_eq!(
            (result.success_count, result.fail_count, result.total),
            (4, 0, 4)
        );
        for name in ["img_01.jpg", "img_02.png", "img_03.png"] {
            assert!(root.join(name).is_file(), "{}", name);
        }
        assert_eq!(std::fs::read(root.join("x.txt")).unwrap(), b"shared tags");
        assert_eq!(std::fs::read(root.join("img_03.txt")).unwrap(), b"y tags");
        let events = log.lock().unwrap();
        assert_eq!(events[0]["status"], "warning");
        assert_eq!(
            events[0]["message"],
            "[跳过] x.txt 被 x.jpg、x.png 共用，未重命名"
        );
        let run_id = events[0]["run_id"].as_u64().unwrap();
        assert!(events.iter().all(|e| e["run_id"] == run_id));
        assert_eq!(events.last().unwrap()["status"], "done");
    }

    #[test]
    fn shuffle_with_same_seed_is_deterministic() {
        let files: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("img_{}.png", i)))
            .collect();
        let mut a = files.clone();
        let mut b = files.clone();
        apply_shuffle(&mut a, Some(42));
        apply_shuffle(&mut b, Some(42));
        assert_eq!(a, b);

        // 只是重排，不增删文件
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(sorted, files);
    }

    #[test]
    fn shuffle_without_seed_keeps_all_files() {
        let files: Vec<PathBuf> = (0..5)
            .map(|i| PathBuf::from(format!("img_{}.png", i)))
            .collect();
        let mut a = files.clone();
        apply_shuffle(&mut a, None);
        a.sort();
        assert_eq!(a, files);
    }
}
