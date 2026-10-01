use super::ProgressEvent;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);
static TRANSLATE_CANCEL: AtomicBool = AtomicBool::new(false);
static IS_DOWNLOADING: AtomicBool = AtomicBool::new(false);
static IS_TRANSLATING: AtomicBool = AtomicBool::new(false);

/// 标签库翻译每批送给 Google 的标签数
const TRANSLATE_BATCH: usize = 80;

fn tag_db_path() -> PathBuf {
    super::config_paths::default_tagcache_dir().join("danbooru_tags.db")
}

fn open_tag_db() -> Result<Connection, String> {
    let path = tag_db_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开标签数据库失败: {}", e))?;
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

fn http_client() -> Result<reqwest::Client, String> {
    super::proxy_config::build_http_client()
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))
}

fn emit_db<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    status: &str,
    message: &str,
    current: u32,
    total: u32,
) {
    ProgressEvent::new(status, message)
        .at(current, total)
        .emit(app, "tag-db-progress");
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
    fetch_latest_tag_filename(&http_client()?).await
}

/// 从 GitHub API 获取最新的 danbooru pt20 CSV 文件名
async fn fetch_latest_tag_filename(client: &reqwest::Client) -> Result<String, String> {
    let api_url = "https://api.github.com/repos/DraconicDragon/dbr-e621-lists-archive/contents/tag-lists/danbooru";
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

/// 下载并导入 Danbooru 标签数据
#[tauri::command]
pub async fn download_danbooru_tags(app: tauri::AppHandle) -> Result<u32, String> {
    let _busy = super::BusyGuard::acquire(&IS_DOWNLOADING, "标签库下载")?;
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    // 动态获取最新文件名
    emit_db(&app, "checking", "正在检查最新版本...", 0, 0);

    let client = http_client()?;
    let filename = fetch_latest_tag_filename(&client).await?;
    let url = format!("https://raw.githubusercontent.com/DraconicDragon/dbr-e621-lists-archive/main/tag-lists/danbooru/{}", filename);

    emit_db(&app, "downloading", "正在下载标签数据...", 0, 0);

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }

    let csv_text = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
        return Err("下载已取消".into());
    }

    emit_db(&app, "importing", "正在导入标签数据库...", 0, 0);

    let source_file = filename.clone();
    let (count, total) = tokio::task::spawn_blocking(move || -> Result<(u32, u32), String> {
        let conn = open_tag_db()?;
        conn.execute_batch("BEGIN TRANSACTION;")
            .map_err(|e| format!("开始事务失败: {}", e))?;
        // 清空旧数据必须在事务内执行，确保中途取消/失败时 ROLLBACK 能恢复旧库
        if let Err(e) = conn.execute("DELETE FROM danbooru_tags", []) {
            conn.execute_batch("ROLLBACK;").ok();
            return Err(format!("清空旧数据失败: {}", e));
        }

        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO danbooru_tags (name, category, post_count, aliases) VALUES (?1, ?2, ?3, ?4)"
        ).map_err(|e| format!("准备语句失败: {}", e))?;

        let (mut count, mut total) = (0u32, 0u32);
        for record in tag_csv_reader(&csv_text).records().flatten() {
            if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
                conn.execute_batch("ROLLBACK;").ok();
                return Err("导入已取消".into());
            }
            total += 1;
            let Some((name, category, post_count, aliases)) = parse_tag_record(&record) else {
                continue;
            };
            stmt.execute(rusqlite::params![name, category, post_count, aliases])
                .map_err(|e| format!("插入标签失败: {} - {}", name, e))?;
            count += 1;
        }

        conn.execute_batch("COMMIT;")
            .map_err(|e| format!("提交事务失败: {}", e))?;

        set_meta(&conn, "source_file", &source_file);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        set_meta(&conn, "import_date", &format!("{}", now));

        Ok((count, total))
    }).await.map_err(|e| format!("导入任务失败: {}", e))??;

    emit_db(
        &app,
        "done",
        &format!("导入完成，共 {} 个标签 ({})", count, filename),
        count,
        total,
    );

    Ok(count)
}

/// 标签表 CSV 没有表头，每行 `name,category,post_count,aliases`，别名列含逗号时带引号
fn tag_csv_reader(text: &str) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes())
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

    let tl = target_lang.clone();
    let untranslated: Vec<String> =
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
            // 从翻译缓存中查找已有翻译的标签
            let already = match super::translator::open_db() {
                Ok(cache_conn) => super::translator::tags_for_lang(&cache_conn, &tl)
                    .map_err(|e| format!("查询翻译缓存失败: {}", e))?,
                Err(_) => std::collections::HashSet::new(),
            };
            Ok(all_tags
                .into_iter()
                .filter(|t| !already.contains(t))
                .collect())
        })
        .await
        .map_err(|e| format!("获取未翻译标签失败: {}", e))??;

    let total = untranslated.len() as u32;
    if total == 0 {
        emit_db(&app, "done", "所有标签已翻译", 0, 0);
        return Ok(0);
    }

    emit_db(
        &app,
        "translating",
        &format!("开始翻译 {} 个标签...", total),
        0,
        total,
    );

    let client = http_client()?;
    let mut translated_count = 0u32;

    for chunk in untranslated.chunks(TRANSLATE_BATCH) {
        if TRANSLATE_CANCEL.load(Ordering::SeqCst) {
            return Err("翻译已取消".into());
        }

        match super::translator::translate_google(&client, chunk, &target_lang).await {
            // 返回行数与请求行数不一致说明结果已错位，整批放弃，避免错误翻译写入缓存
            Ok(parts) if parts.len() != chunk.len() => emit_db(
                &app,
                "warning",
                &format!(
                    "翻译返回行数 ({}) 与请求行数 ({}) 不一致，跳过当前批次",
                    parts.len(),
                    chunk.len()
                ),
                translated_count,
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

                let lang = target_lang.clone();
                tokio::task::spawn_blocking(move || -> Result<(), String> {
                    // 只写入翻译缓存数据库
                    let cache_conn = super::translator::open_db()
                        .map_err(|e| format!("打开翻译缓存失败: {}", e))?;
                    super::translator::upsert_translations(&cache_conn, &pairs, &lang)
                        .map_err(|e| format!("准备写入语句失败: {}", e))
                })
                .await
                .map_err(|e| format!("写入翻译失败: {}", e))??;

                translated_count += batch_count;
            }
            Err(e) => emit_db(
                &app,
                "warning",
                &format!("{}, 跳过当前批次", e),
                translated_count,
                total,
            ),
        }

        emit_db(
            &app,
            "translating",
            &format!("已翻译 {}/{}", translated_count, total),
            translated_count,
            total,
        );

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    emit_db(
        &app,
        "done",
        &format!("翻译完成，共翻译 {} 个标签", translated_count),
        translated_count,
        total,
    );

    Ok(translated_count)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn db_events_keep_status_message_and_counts() {
        let app = tauri::test::mock_app();
        let events = super::super::batch::capture_events(app.handle(), "tag-db-progress");
        emit_db(app.handle(), "translating", "翻译中", 2, 9);
        let events = events.lock().unwrap();
        assert_eq!(events[0]["status"], "translating");
        assert_eq!(events[0]["message"], "翻译中");
        assert_eq!(events[0]["current"], 2);
        assert_eq!(events[0]["total"], 9);
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
        let mut rdr = tag_csv_reader(text);
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
}
