use super::http_download::{self, DownloadError};
use super::ProgressEvent;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Runtime};

static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);
static TRANSLATE_CANCEL: AtomicBool = AtomicBool::new(false);
static IS_DOWNLOADING: AtomicBool = AtomicBool::new(false);
static IS_TRANSLATING: AtomicBool = AtomicBool::new(false);

/// 标签库下载、翻译共用的进度事件
const EVENT: &str = "tag-db-progress";

/// 标签库翻译每批送给 Google 的标签数
const TRANSLATE_BATCH: usize = 80;

/// 两批翻译之间的间隔，避免触发 Google 的限流
const TRANSLATE_INTERVAL: Duration = Duration::from_millis(500);

/// GitHub API、Google 翻译单次请求的时限
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 等网络请求期间检查取消的间隔
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// danbooru 标签表所在的 GitHub 目录（contents API）和文件直链的前缀
const TAG_LIST_API: &str =
    "https://api.github.com/repos/DraconicDragon/dbr-e621-lists-archive/contents/tag-lists/danbooru";
const TAG_LIST_FILES: &str =
    "https://raw.githubusercontent.com/DraconicDragon/dbr-e621-lists-archive/main/tag-lists/danbooru";

fn tag_db_path() -> PathBuf {
    super::config_paths::default_tagcache_dir().join("danbooru_tags.db")
}

fn open_tag_db() -> Result<Connection, String> {
    open_tag_db_at(&tag_db_path())
}

fn open_tag_db_at(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(path).map_err(|e| format!("打开标签数据库失败: {}", e))?;
    // name 是主键，自带索引；旧版本另建的同列索引 idx_tags_name 是重复的，打开时删掉
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS danbooru_tags (
            name TEXT PRIMARY KEY,
            category INTEGER NOT NULL DEFAULT 0,
            post_count INTEGER NOT NULL DEFAULT 0,
            aliases TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS tag_db_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tags_post_count ON danbooru_tags(post_count DESC);
        DROP INDEX IF EXISTS idx_tags_name;
        ",
    )
    .map_err(|e| format!("创建标签表失败: {}", e))?;
    // 旧表有 translated 列则删除（通过重建表迁移）
    let has_translated: bool = conn
        .prepare("PRAGMA table_info(danbooru_tags)")
        .and_then(|mut stmt| {
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(cols.iter().any(|c| c == "translated"))
        })
        .unwrap_or(false);
    if has_translated {
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS danbooru_tags_new (
                name TEXT PRIMARY KEY,
                category INTEGER NOT NULL DEFAULT 0,
                post_count INTEGER NOT NULL DEFAULT 0,
                aliases TEXT NOT NULL DEFAULT ''
            );
            INSERT OR IGNORE INTO danbooru_tags_new (name, category, post_count, aliases)
                SELECT name, category, post_count, aliases FROM danbooru_tags;
            DROP TABLE danbooru_tags;
            ALTER TABLE danbooru_tags_new RENAME TO danbooru_tags;
            CREATE INDEX IF NOT EXISTS idx_tags_post_count ON danbooru_tags(post_count DESC);",
        );
    }
    Ok(conn)
}

/// 标签库任务的进度事件。每条都显式带本轮的运行 ID：下载和翻译共用一个通道，
/// 按通道取「当前一轮」会把一个任务的事件记到另一个任务名下
struct DbEvents<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    run_id: u64,
}

impl<R: Runtime> DbEvents<'_, R> {
    fn emit(&self, status: &str, message: impl Into<String>, current: u32, total: u32) {
        ProgressEvent::new(status, message)
            .at(current, total)
            .for_run(self.run_id)
            .emit(self.app, EVENT);
    }

    /// 被取消那一轮的终态：done + cancelled，文案以「已取消」开头
    fn cancelled(&self, message: impl Into<String>, current: u32, total: u32) {
        ProgressEvent::new("done", message)
            .at(current, total)
            .cancelled()
            .for_run(self.run_id)
            .emit(self.app, EVENT);
    }
}

/// 等 `future` 完成；期间 `cancel` 置位就不再等，返回 None
async fn until_cancelled<T>(future: impl Future<Output = T>, cancel: &AtomicBool) -> Option<T> {
    tokio::pin!(future);
    let mut tick = tokio::time::interval(CANCEL_POLL);
    loop {
        tokio::select! {
            output = &mut future => return Some(output),
            _ = tick.tick() => {
                if cancel.load(Ordering::SeqCst) {
                    return None;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagDbStats {
    pub total_tags: u32,
    pub translated_tags: u32,
    pub db_size_bytes: u64,
    pub has_data: bool,
    pub source_file: String,
    pub import_date: String,
}

fn get_meta(conn: &Connection, key: &str) -> String {
    conn.query_row("SELECT value FROM tag_db_meta WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .unwrap_or_default()
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO tag_db_meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )
    .ok();
}

/// 获取标签数据库状态
#[tauri::command]
pub fn get_tag_db_stats(target_lang: Option<String>) -> Result<TagDbStats, String> {
    let path = tag_db_path();
    if !path.exists() {
        return Ok(TagDbStats::default());
    }
    let conn = open_tag_db()?;
    let total: u32 = conn
        .query_row("SELECT COUNT(*) FROM danbooru_tags", [], |r| r.get(0))
        .unwrap_or(0);
    // 从翻译缓存统计已翻译数量（按目标语言过滤）
    let lang = target_lang.unwrap_or_else(|| "zh-CN".to_string());
    let translated = super::translator::open_db()
        .map(|cache_conn| super::translator::count_for_lang(&cache_conn, &lang) as u32)
        .unwrap_or(0);
    let db_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let source_file = get_meta(&conn, "source_file");
    let import_date = get_meta(&conn, "import_date");
    Ok(TagDbStats {
        total_tags: total,
        translated_tags: translated,
        db_size_bytes: db_size,
        has_data: total > 0,
        source_file,
        import_date,
    })
}

/// 查询标签数据库是否正在忙（下载或翻译中）
#[tauri::command]
pub fn is_tag_db_busy() -> (bool, bool) {
    (
        IS_DOWNLOADING.load(Ordering::SeqCst),
        IS_TRANSLATING.load(Ordering::SeqCst),
    )
}

/// 检查远端最新标签文件版本
#[tauri::command]
pub async fn check_tag_db_update() -> Result<String, String> {
    fetch_latest_tag_filename(&http_download::api_client(REQUEST_TIMEOUT)?, TAG_LIST_API).await
}

/// 从 GitHub API 获取最新的 danbooru pt20 CSV 文件名
async fn fetch_latest_tag_filename(
    client: &reqwest::Client,
    api_url: &str,
) -> Result<String, String> {
    let resp = client
        .get(api_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await;

    if let Ok(resp) = resp {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                    let mut candidates: Vec<String> = items
                        .iter()
                        .filter_map(|item| item["name"].as_str())
                        .filter(|name| {
                            name.starts_with("danbooru_")
                                && name.contains("_pt20")
                                && name.ends_with(".csv")
                        })
                        .map(|s| s.to_string())
                        .collect();
                    candidates.sort();
                    if let Some(latest) = candidates.last().cloned() {
                        return Ok(latest);
                    }
                }
            }
        }
    }

    Err("GitHub API 请求失败（可能是限流或网络问题），请检查网络连接或代理设置后重试".to_string())
}

/// 标签表从哪里取（测试换成本地服务器）
struct TagListSource {
    /// 查最新文件名用，有总时限
    api: reqwest::Client,
    /// 下载标签表用，只有停滞超时
    download: reqwest::Client,
    list_url: String,
    /// 标签表文件的下载目录，文件名拼在后面
    files_url: String,
}

/// 下载并导入 Danbooru 标签数据
#[tauri::command]
pub async fn download_danbooru_tags(app: tauri::AppHandle) -> Result<u32, String> {
    let _busy = super::BusyGuard::acquire(&IS_DOWNLOADING, "标签库下载")?;
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);
    let events = DbEvents {
        app: &app,
        run_id: super::begin_run(EVENT),
    };
    let source = TagListSource {
        api: http_download::api_client(REQUEST_TIMEOUT)?,
        download: http_download::download_client()?,
        list_url: TAG_LIST_API.to_string(),
        files_url: TAG_LIST_FILES.to_string(),
    };
    download_and_import(&events, &source, &tag_db_path(), &DOWNLOAD_CANCEL).await
}

/// 查最新的标签表 → 下载到标签库旁边 → 导入。取消时发 done + cancelled 并返回 Ok(0)
async fn download_and_import<R: Runtime>(
    events: &DbEvents<'_, R>,
    source: &TagListSource,
    db_path: &Path,
    cancel: &'static AtomicBool,
) -> Result<u32, String> {
    events.emit("checking", "正在检查最新版本...", 0, 0);
    let latest = until_cancelled(
        fetch_latest_tag_filename(&source.api, &source.list_url),
        cancel,
    )
    .await;
    let Some(latest) = latest else {
        events.cancelled("已取消下载", 0, 0);
        return Ok(0);
    };
    let filename = latest?;

    events.emit("downloading", "正在下载标签数据...", 0, 0);
    let csv_path = db_path.with_file_name("danbooru_tags.csv");
    let downloaded = http_download::download_to_file(
        source
            .download
            .get(format!("{}/{}", source.files_url, filename)),
        &csv_path,
        &filename,
        cancel,
        |p| events.emit("downloading", p.message, p.percent.round() as u32, 100),
    )
    .await;
    if let Err(e) = downloaded {
        if matches!(e, DownloadError::Cancelled) || cancel.load(Ordering::SeqCst) {
            events.cancelled("已取消下载", 0, 0);
            return Ok(0);
        }
        return Err(format!("下载失败: {}", e));
    }

    events.emit("importing", "正在导入标签数据库...", 0, 0);
    let db = db_path.to_path_buf();
    let source_file = filename.clone();
    let imported = tokio::task::spawn_blocking(move || {
        let imported = import_tag_csv(&db, &csv_path, &source_file, cancel);
        let _ = std::fs::remove_file(&csv_path);
        imported
    })
    .await
    .map_err(|e| format!("导入任务失败: {}", e))??;
    let Some((count, total)) = imported else {
        events.cancelled("已取消导入", 0, 0);
        return Ok(0);
    };
    events.emit(
        "done",
        format!("导入完成，共 {} 个标签 ({})", count, filename),
        count,
        total,
    );
    Ok(count)
}

/// 把下载的 CSV 导入标签库。清空旧数据和写入新数据在同一个事务里：取消或失败时回滚，旧库原样保留。
/// 取消返回 Ok(None)，否则返回 (导入条数, CSV 记录数)
fn import_tag_csv(
    db_path: &Path,
    csv_path: &Path,
    source_file: &str,
    cancel: &AtomicBool,
) -> Result<Option<(u32, u32)>, String> {
    let csv = std::fs::File::open(csv_path).map_err(|e| format!("读取标签数据失败: {}", e))?;
    let mut conn = open_tag_db_at(db_path)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("开始事务失败: {}", e))?;
    tx.execute("DELETE FROM danbooru_tags", [])
        .map_err(|e| format!("清空旧数据失败: {}", e))?;
    let (mut count, mut total) = (0u32, 0u32);
    {
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO danbooru_tags (name, category, post_count, aliases) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| format!("准备语句失败: {}", e))?;
        for record in tag_csv_reader(csv).records().flatten() {
            if cancel.load(Ordering::SeqCst) {
                return Ok(None);
            }
            total += 1;
            let Some((name, category, post_count, aliases)) = parse_tag_record(&record) else {
                continue;
            };
            stmt.execute(rusqlite::params![name, category, post_count, aliases])
                .map_err(|e| format!("插入标签失败: {} - {}", name, e))?;
            count += 1;
        }
    }
    tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;

    set_meta(&conn, "source_file", source_file);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    set_meta(&conn, "import_date", &now.to_string());
    Ok(Some((count, total)))
}

/// 标签表 CSV 没有表头，每行 `name,category,post_count,aliases`，别名列含逗号时带引号
fn tag_csv_reader<R: std::io::Read>(reader: R) -> csv::Reader<R> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(reader)
}

/// 取出一行的 (name, category, post_count, aliases)；数字解析不了按 0，name 为空返回 None
fn parse_tag_record(record: &csv::StringRecord) -> Option<(&str, i32, i64, &str)> {
    let name = record.get(0).filter(|n| !n.trim().is_empty())?;
    let category = record.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let post_count = record.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((name, category, post_count, record.get(3).unwrap_or("")))
}

/// 取消下载
#[tauri::command]
pub fn cancel_tag_db_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn cancel_tag_db_translation() {
    TRANSLATE_CANCEL.store(true, Ordering::SeqCst);
}

/// 清空标签数据库
#[tauri::command]
pub fn clear_tag_db() -> Result<(), String> {
    let conn = open_tag_db()?;
    conn.execute("DELETE FROM danbooru_tags", [])
        .map_err(|e| format!("清空失败: {}", e))?;
    conn.execute("VACUUM", [])
        .map_err(|e| format!("压缩失败: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSuggestion {
    pub name: String,
    pub category: i32,
    pub post_count: i64,
    pub translated: Option<String>,
}

/// 按 `column LIKE pattern` 取热度最高的前 limit 条（column 只会是 name 或 aliases）
fn query_suggestions(
    conn: &Connection,
    column: &str,
    pattern: &str,
    limit: usize,
) -> Result<Vec<TagSuggestion>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name, category, post_count FROM danbooru_tags
         WHERE {} LIKE ?1 ORDER BY post_count DESC LIMIT ?2",
            column
        ))
        .map_err(|e| format!("查询失败: {}", e))?;

    let rows = stmt
        .query_map(rusqlite::params![pattern, limit], |row| {
            Ok(TagSuggestion {
                name: row.get(0)?,
                category: row.get(1)?,
                post_count: row.get(2)?,
                translated: None,
            })
        })
        .map_err(|e| format!("查询失败: {}", e))?;
    Ok(rows.flatten().collect())
}

/// 标签自动补全搜索
#[tauri::command]
pub fn search_tags(
    query: String,
    limit: Option<u32>,
    target_lang: Option<String>,
) -> Result<Vec<TagSuggestion>, String> {
    let path = tag_db_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let conn = open_tag_db()?;
    let limit = limit.unwrap_or(10).min(50) as usize;
    let query_lower = query.to_lowercase().replace(' ', "_");

    if query_lower.is_empty() {
        return Ok(vec![]);
    }

    // 依次用 name 前缀匹配、name 包含匹配、别名包含匹配补足 limit 条，合并去重；
    // 第三项是在已有结果之外多取的条数，给去重留余量
    let mut results: Vec<TagSuggestion> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let steps = [
        ("name", format!("{}%", query_lower), 0),
        ("name", format!("%{}%", query_lower), 20),
        ("aliases", format!("%{}%", query_lower), 20),
    ];
    for (column, pattern, extra) in &steps {
        if results.len() >= limit {
            break;
        }
        let fetch = limit - results.len() + extra;
        for tag in query_suggestions(&conn, column, pattern, fetch)? {
            if results.len() >= limit {
                break;
            }
            if seen.insert(tag.name.clone()) {
                results.push(tag);
            }
        }
    }

    // 从翻译缓存查找翻译
    let lang = target_lang.unwrap_or_else(|| "zh-CN".to_string());
    if !lang.is_empty() {
        if let Ok(trans_conn) = super::translator::open_db() {
            let names = results.iter().map(|t| t.name.as_str());
            if let Ok(found) = super::translator::lookup_cached(&trans_conn, names, &lang) {
                for tag in results.iter_mut() {
                    tag.translated = found.get(&tag.name).cloned();
                }
            }
        }
    }

    Ok(results)
}

/// 批量翻译标签数据库中的标签（使用 Google 翻译）
#[tauri::command]
pub async fn translate_tag_db(app: tauri::AppHandle, target_lang: String) -> Result<u32, String> {
    let _busy = super::BusyGuard::acquire(&IS_TRANSLATING, "标签库翻译")?;
    TRANSLATE_CANCEL.store(false, Ordering::SeqCst);
    let events = DbEvents {
        app: &app,
        run_id: super::begin_run(EVENT),
    };
    let untranslated = untranslated_tags(target_lang.clone()).await?;
    let client = http_download::api_client(REQUEST_TIMEOUT)?;
    translate_in_batches(
        &events,
        untranslated,
        &TRANSLATE_CANCEL,
        |chunk| {
            let client = client.clone();
            let lang = target_lang.clone();
            async move { super::translator::translate_google(&client, &chunk, &lang).await }
        },
        |pairs| save_translations(pairs, target_lang.clone()),
    )
    .await
}

/// 标签库里在翻译缓存中还没有 `lang` 译文的标签，按热度从高到低
async fn untranslated_tags(lang: String) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
        let conn = open_tag_db()?;
        let all_tags: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM danbooru_tags ORDER BY post_count DESC")
                .map_err(|e| format!("查询失败: {}", e))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("查询失败: {}", e))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let already = match super::translator::open_db() {
            Ok(cache_conn) => super::translator::tags_for_lang(&cache_conn, &lang)
                .map_err(|e| format!("查询翻译缓存失败: {}", e))?,
            Err(_) => std::collections::HashSet::new(),
        };
        Ok(all_tags
            .into_iter()
            .filter(|t| !already.contains(t))
            .collect())
    })
    .await
    .map_err(|e| format!("获取未翻译标签失败: {}", e))?
}

/// 译文只写入翻译缓存数据库
async fn save_translations(pairs: Vec<(String, String)>, lang: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let cache_conn =
            super::translator::open_db().map_err(|e| format!("打开翻译缓存失败: {}", e))?;
        super::translator::upsert_translations(&cache_conn, &pairs, &lang)
            .map_err(|e| format!("准备写入语句失败: {}", e))
    })
    .await
    .map_err(|e| format!("写入翻译失败: {}", e))?
}

/// 按批翻译、写入并发进度，返回写入的译文数。某一批请求失败或返回行数对不上时只跳过那一批（发 warning）；
/// 取消时发 done + cancelled，返回 Ok
async fn translate_in_batches<R, T, TF, S, SF>(
    events: &DbEvents<'_, R>,
    untranslated: Vec<String>,
    cancel: &AtomicBool,
    mut translate: T,
    mut save: S,
) -> Result<u32, String>
where
    R: Runtime,
    T: FnMut(Vec<String>) -> TF,
    TF: Future<Output = Result<Vec<String>, String>>,
    S: FnMut(Vec<(String, String)>) -> SF,
    SF: Future<Output = Result<(), String>>,
{
    let total = untranslated.len() as u32;
    if total == 0 {
        events.emit("done", "所有标签已翻译", 0, 0);
        return Ok(0);
    }
    events.emit(
        "translating",
        format!("开始翻译 {} 个标签...", total),
        0,
        total,
    );

    let mut processed = 0u32;
    let mut translated = 0u32;
    let finish_cancelled = |processed: u32, translated: u32| {
        events.cancelled(
            format!(
                "已取消翻译: 已处理 {}/{}, 成功 {}",
                processed, total, translated
            ),
            translated,
            total,
        );
        Ok(translated)
    };
    for chunk in untranslated.chunks(TRANSLATE_BATCH) {
        if cancel.load(Ordering::SeqCst) {
            return finish_cancelled(processed, translated);
        }
        let Some(reply) = until_cancelled(translate(chunk.to_vec()), cancel).await else {
            return finish_cancelled(processed, translated);
        };
        match reply {
            // 返回行数与请求行数不一致说明结果已错位，整批放弃，避免错误翻译写入缓存
            Ok(parts) if parts.len() != chunk.len() => events.emit(
                "warning",
                format!(
                    "翻译返回行数 ({}) 与请求行数 ({}) 不一致，跳过当前批次",
                    parts.len(),
                    chunk.len()
                ),
                translated,
                total,
            ),
            Ok(parts) => {
                let pairs: Vec<(String, String)> = chunk
                    .iter()
                    .cloned()
                    .zip(parts)
                    .filter(|(_, tr)| !tr.is_empty())
                    .collect();
                let batch_count = pairs.len() as u32;
                save(pairs).await?;
                translated += batch_count;
            }
            Err(e) => events.emit("warning", format!("{}, 跳过当前批次", e), translated, total),
        }
        processed += chunk.len() as u32;

        events.emit(
            "translating",
            format!("已翻译 {}/{}", translated, total),
            translated,
            total,
        );
        tokio::time::sleep(TRANSLATE_INTERVAL).await;
    }

    events.emit(
        "done",
        format!("翻译完成，共翻译 {} 个标签", translated),
        translated,
        total,
    );
    Ok(translated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::batch::capture_raw_events;
    use crate::commands::test_support::{serve_once, write_response, TempDir};
    use std::io::Write;

    const LISTING: &str = r#"[
        {"name": "danbooru_2024-01-01_pt20-ia-dd.csv"},
        {"name": "danbooru_2025-03-01_pt20-ia-dd.csv"},
        {"name": "danbooru_2025-04-01_pt10-ia-dd.csv"},
        {"name": "e621_2025-05-01_pt20-ia-dd.csv"},
        {"name": "README.md"}
    ]"#;
    const LATEST: &str = "danbooru_2025-03-01_pt20-ia-dd.csv";

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn source(list_url: String, files_url: String) -> TagListSource {
        TagListSource {
            api: client(),
            download: client(),
            list_url,
            files_url,
        }
    }

    fn listing_server() -> crate::commands::test_support::TestServer {
        serve_once(|stream, _, _| {
            write_response(stream, "200 OK", "application/json", LISTING.as_bytes())
        })
    }

    fn rows(db: &Path) -> Vec<(String, i64)> {
        let conn = open_tag_db_at(db).unwrap();
        let mut stmt = conn
            .prepare("SELECT name, post_count FROM danbooru_tags ORDER BY name")
            .unwrap();
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    fn seed(db: &Path) {
        let conn = open_tag_db_at(db).unwrap();
        conn.execute(
            "INSERT INTO danbooru_tags (name, category, post_count, aliases) VALUES ('old_tag', 0, 7, '')",
            [],
        )
        .unwrap();
    }

    fn old_rows() -> Vec<(String, i64)> {
        vec![("old_tag".to_string(), 7)]
    }

    #[test]
    fn cancellation_flags_are_independent() {
        DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);
        TRANSLATE_CANCEL.store(false, Ordering::SeqCst);
        cancel_tag_db_download();
        assert!(DOWNLOAD_CANCEL.load(Ordering::SeqCst));
        assert!(!TRANSLATE_CANCEL.load(Ordering::SeqCst));
        DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);
        cancel_tag_db_translation();
        assert!(!DOWNLOAD_CANCEL.load(Ordering::SeqCst));
        assert!(TRANSLATE_CANCEL.load(Ordering::SeqCst));
        TRANSLATE_CANCEL.store(false, Ordering::SeqCst);
    }

    #[test]
    fn db_events_keep_status_message_counts_and_their_own_run() {
        let app = tauri::test::mock_app();
        let events = capture_raw_events(app.handle(), EVENT);
        let db_events = DbEvents {
            app: app.handle(),
            run_id: 3,
        };
        // 另一个任务在同一通道上开始了新一轮，这一轮的事件仍带自己的运行 ID
        crate::commands::begin_run(EVENT);
        db_events.emit("translating", "翻译中", 2, 9);
        db_events.cancelled("已取消下载", 0, 0);
        let events = events.lock().unwrap();
        assert_eq!(events[0]["status"], "translating");
        assert_eq!(events[0]["message"], "翻译中");
        assert_eq!(events[0]["current"], 2);
        assert_eq!(events[0]["total"], 9);
        assert_eq!(events[0]["run_id"], 3);
        assert!(events[0].get("cancelled").is_none());
        assert_eq!(events[1]["status"], "done");
        assert_eq!(events[1]["cancelled"], true);
        assert_eq!(events[1]["run_id"], 3);
    }

    /// 别名列的引号、引号内逗号与转义引号按 CSV 规则解析；空行、空白行和空 name 行不入库
    #[test]
    fn tag_csv_rows_parse_quoted_aliases() {
        let text = "1girl,0,6000000,\"1girls,sole_female\"\n\
                    \n\
                    solo,0,5\n   \n\
                    ,1,2,orphan\n\
                    \"quote\"\"d\",4,abc,\"say \"\"hi\"\"\"\r\n\
                    bad_count,x,y\n";
        let mut rdr = tag_csv_reader(text.as_bytes());
        let records: Vec<csv::StringRecord> = rdr.records().flatten().collect();
        let parsed: Vec<_> = records.iter().filter_map(parse_tag_record).collect();
        assert_eq!(
            parsed,
            vec![
                ("1girl", 0, 6000000, "1girls,sole_female"),
                ("solo", 0, 5, ""),
                ("quote\"d", 4, 0, "say \"hi\""),
                ("bad_count", 0, 0, ""),
            ]
        );
    }

    #[tokio::test]
    async fn download_imports_the_latest_list_and_reports_progress() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let api = listing_server();
        let csv = b"1girl,0,6000000,\"1girls,sole_female\"\nsolo,0,5000000,\n,1,2,orphan\nbad_count,x,y\n";
        let files =
            serve_once(move |stream, _, _| write_response(stream, "200 OK", "text/csv", csv));
        let dir = TempDir::new("tag_db_download");
        let db = dir.join("danbooru_tags.db");
        seed(&db);
        let app = tauri::test::mock_app();
        let raw = capture_raw_events(app.handle(), EVENT);
        let events = DbEvents {
            app: app.handle(),
            run_id: 41,
        };
        let source = source(format!("{}/contents", api.url), files.url.clone());
        let count = download_and_import(&events, &source, &db, &CANCEL)
            .await
            .unwrap();

        assert_eq!(count, 3);
        assert_eq!(files.requests.recv().unwrap().path, format!("/{LATEST}"));
        assert_eq!(
            rows(&db),
            [
                ("1girl".to_string(), 6000000),
                ("bad_count".to_string(), 0),
                ("solo".to_string(), 5000000),
            ]
        );
        let conn = open_tag_db_at(&db).unwrap();
        assert_eq!(get_meta(&conn, "source_file"), LATEST);
        assert!(!dir.join("danbooru_tags.csv").exists());

        let raw = raw.lock().unwrap();
        assert!(raw.iter().all(|e| e["run_id"] == 41));
        let statuses: Vec<&str> = raw.iter().map(|e| e["status"].as_str().unwrap()).collect();
        assert_eq!(statuses.first(), Some(&"checking"));
        assert!(statuses.contains(&"downloading"));
        assert!(statuses.contains(&"importing"));
        let done = raw.last().unwrap();
        assert_eq!(done["status"], "done");
        assert!(done.get("cancelled").is_none());
        assert_eq!(done["current"], 3);
        assert_eq!(done["total"], 4);
        assert_eq!(done["message"], format!("导入完成，共 3 个标签 ({LATEST})"));
    }

    #[tokio::test]
    async fn cancelled_download_keeps_the_existing_db_and_returns_ok() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let api = listing_server();
        // 发出一部分数据后停住，同时点取消
        let files = serve_once(|stream, _, stop| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\nsolo,0,1\n",
            );
            let _ = stream.flush();
            CANCEL.store(true, Ordering::SeqCst);
            stop.wait(Duration::from_secs(10));
        });
        let dir = TempDir::new("tag_db_download_cancel");
        let db = dir.join("danbooru_tags.db");
        seed(&db);
        let app = tauri::test::mock_app();
        let raw = capture_raw_events(app.handle(), EVENT);
        let events = DbEvents {
            app: app.handle(),
            run_id: 7,
        };
        let source = source(format!("{}/contents", api.url), files.url.clone());
        let started = std::time::Instant::now();
        let count = download_and_import(&events, &source, &db, &CANCEL)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(files.requests.try_recv().is_ok());
        assert_eq!(rows(&db), old_rows());
        assert!(!dir.join("danbooru_tags.csv").exists());
        assert!(!dir.join("danbooru_tags.csv.part").exists());
        let last = raw.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last["status"], "done");
        assert_eq!(last["cancelled"], true);
        assert_eq!(last["message"], "已取消下载");
        assert_eq!(last["run_id"], 7);
    }

    #[tokio::test]
    async fn cancel_while_checking_for_the_latest_list_stops_waiting() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let api = serve_once(|_, _, stop| {
            CANCEL.store(true, Ordering::SeqCst);
            stop.wait(Duration::from_secs(10));
        });
        let dir = TempDir::new("tag_db_check_cancel");
        let db = dir.join("danbooru_tags.db");
        let app = tauri::test::mock_app();
        let raw = capture_raw_events(app.handle(), EVENT);
        let events = DbEvents {
            app: app.handle(),
            run_id: 8,
        };
        let source = source(format!("{}/contents", api.url), "http://127.0.0.1:9".into());
        let started = std::time::Instant::now();
        assert_eq!(
            download_and_import(&events, &source, &db, &CANCEL).await,
            Ok(0)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!db.exists());
        let last = raw.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last["cancelled"], true);
        assert_eq!(last["message"], "已取消下载");
    }

    #[tokio::test]
    async fn missing_list_file_is_an_error_and_writes_nothing() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let api = listing_server();
        let files = serve_once(|stream, _, _| {
            write_response(stream, "404 Not Found", "text/plain", b"nope")
        });
        let dir = TempDir::new("tag_db_download_404");
        let db = dir.join("danbooru_tags.db");
        let app = tauri::test::mock_app();
        let events = DbEvents {
            app: app.handle(),
            run_id: 5,
        };
        let source = source(format!("{}/contents", api.url), files.url.clone());
        let error = download_and_import(&events, &source, &db, &CANCEL)
            .await
            .unwrap_err();
        assert!(error.starts_with("下载失败: HTTP 404"), "{error}");
        assert!(!db.exists());
        assert!(!dir.join("danbooru_tags.csv").exists());
    }

    #[test]
    fn cancelled_import_rolls_back_to_the_old_rows() {
        let dir = TempDir::new("tag_db_import_cancel");
        let db = dir.join("danbooru_tags.db");
        seed(&db);
        let csv = dir.join("danbooru_tags.csv");
        std::fs::write(&csv, "solo,0,5\n1girl,0,6\n").unwrap();

        assert_eq!(
            import_tag_csv(&db, &csv, "new.csv", &AtomicBool::new(true)),
            Ok(None)
        );
        assert_eq!(rows(&db), old_rows());
        assert_eq!(get_meta(&open_tag_db_at(&db).unwrap(), "source_file"), "");

        assert_eq!(
            import_tag_csv(&db, &csv, "new.csv", &AtomicBool::new(false)),
            Ok(Some((2, 2)))
        );
        assert_eq!(
            rows(&db),
            [("1girl".to_string(), 6), ("solo".to_string(), 5)]
        );
        assert_eq!(
            get_meta(&open_tag_db_at(&db).unwrap(), "source_file"),
            "new.csv"
        );
    }

    #[tokio::test]
    async fn cancelled_translation_reports_done_and_returns_ok() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let raw = capture_raw_events(app.handle(), EVENT);
        let events = DbEvents {
            app: app.handle(),
            run_id: 9,
        };
        let tags: Vec<String> = (0..100).map(|i| format!("tag_{i}")).collect();
        let saved = std::sync::Mutex::new(Vec::new());
        let result = translate_in_batches(
            &events,
            tags,
            &CANCEL,
            |chunk| async move {
                Ok::<_, String>(chunk.iter().map(|t| t.to_uppercase()).collect())
            },
            |pairs| {
                saved.lock().unwrap().extend(pairs);
                // 第一批写完后点取消
                CANCEL.store(true, Ordering::SeqCst);
                async { Ok::<(), String>(()) }
            },
        )
        .await;
        assert_eq!(result, Ok(80));
        assert_eq!(saved.lock().unwrap().len(), 80);
        let last = raw.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last["status"], "done");
        assert_eq!(last["cancelled"], true);
        assert_eq!(last["message"], "已取消翻译: 已处理 80/100, 成功 80");
        assert_eq!(last["current"], 80);
        assert_eq!(last["total"], 100);
        assert_eq!(last["run_id"], 9);
    }

    /// 返回行数对不上的批次整批放弃；请求失败的批次跳过；都不影响后面的批次
    #[tokio::test]
    async fn misaligned_or_failed_batches_are_skipped() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let app = tauri::test::mock_app();
        let raw = capture_raw_events(app.handle(), EVENT);
        let events = DbEvents {
            app: app.handle(),
            run_id: 10,
        };
        let tags: Vec<String> = (0..200).map(|i| format!("tag_{i}")).collect();
        let mut batch = 0;
        let saved = std::sync::Mutex::new(Vec::new());
        let result = translate_in_batches(
            &events,
            tags,
            &CANCEL,
            |chunk| {
                batch += 1;
                let reply = match batch {
                    1 => Ok(vec!["只有一行".to_string()]),
                    2 => Err("Google 翻译请求失败: timeout".to_string()),
                    _ => Ok(chunk.iter().map(|t| format!("译{t}")).collect()),
                };
                async move { reply }
            },
            |pairs| {
                saved.lock().unwrap().extend(pairs);
                async { Ok::<(), String>(()) }
            },
        )
        .await;
        assert_eq!(result, Ok(40));
        assert_eq!(saved.lock().unwrap().len(), 40);
        let raw = raw.lock().unwrap();
        let warnings: Vec<&str> = raw
            .iter()
            .filter(|e| e["status"] == "warning")
            .map(|e| e["message"].as_str().unwrap())
            .collect();
        assert_eq!(
            warnings,
            [
                "翻译返回行数 (1) 与请求行数 (80) 不一致，跳过当前批次",
                "Google 翻译请求失败: timeout, 跳过当前批次",
            ]
        );
        let last = raw.last().unwrap();
        assert_eq!(last["status"], "done");
        assert!(last.get("cancelled").is_none());
        assert_eq!(last["message"], "翻译完成，共翻译 40 个标签");
    }
}
