use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// 标签数据库路径
static TAG_DB_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);
static IS_DOWNLOADING: AtomicBool = AtomicBool::new(false);
static IS_TRANSLATING: AtomicBool = AtomicBool::new(false);

/// 获取软件根目录
fn get_exe_root() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = exe.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf();
    if cfg!(target_os = "macos") {
        if let Some(contents) = exe_dir.parent() {
            if let Some(app_bundle) = contents.parent() {
                if app_bundle.extension().map(|e| e == "app").unwrap_or(false) {
                    return app_bundle.parent().unwrap_or(&exe_dir).to_path_buf();
                }
            }
        }
    }
    exe_dir
}

fn default_db_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        get_exe_root().join("data").join("tagcache")
    } else {
        get_exe_root().join("tagcache")
    }
}

fn get_tag_db_path() -> PathBuf {
    let guard = TAG_DB_PATH.lock().unwrap();
    guard.clone().unwrap_or_else(|| default_db_dir().join("danbooru_tags.db"))
}

fn open_tag_db() -> Result<Connection, String> {
    let path = get_tag_db_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path)
        .map_err(|e| format!("打开标签数据库失败: {}", e))?;
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
        CREATE INDEX IF NOT EXISTS idx_tags_name ON danbooru_tags(name);
        "
    ).map_err(|e| format!("创建标签表失败: {}", e))?;
    // 旧表有 translated 列则删除（通过重建表迁移）
    let has_translated: bool = conn
        .prepare("PRAGMA table_info(danbooru_tags)")
        .and_then(|mut stmt| {
            let cols: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(1))?
                .filter_map(|r| r.ok()).collect();
            Ok(cols.iter().any(|c| c == "translated"))
        }).unwrap_or(false);
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
            CREATE INDEX IF NOT EXISTS idx_tags_post_count ON danbooru_tags(post_count DESC);
            CREATE INDEX IF NOT EXISTS idx_tags_name ON danbooru_tags(name);"
        );
    }
    Ok(conn)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDbStats {
    pub total_tags: u32,
    pub translated_tags: u32,
    pub db_size_bytes: u64,
    pub has_data: bool,
    pub source_file: String,
    pub import_date: String,
}

fn get_meta(conn: &Connection, key: &str) -> String {
    conn.query_row("SELECT value FROM tag_db_meta WHERE key = ?1", [key], |r| r.get::<_, String>(0)).unwrap_or_default()
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute("INSERT OR REPLACE INTO tag_db_meta (key, value) VALUES (?1, ?2)", rusqlite::params![key, value]).ok();
}

/// 获取标签数据库状态
#[tauri::command]
pub fn get_tag_db_stats(target_lang: Option<String>) -> Result<TagDbStats, String> {
    let path = get_tag_db_path();
    if !path.exists() {
        return Ok(TagDbStats {
            total_tags: 0, translated_tags: 0, db_size_bytes: 0, has_data: false,
            source_file: String::new(), import_date: String::new(),
        });
    }
    let conn = open_tag_db()?;
    let total: u32 = conn.query_row(
        "SELECT COUNT(*) FROM danbooru_tags", [], |r| r.get(0)
    ).unwrap_or(0);
    // 从翻译缓存统计已翻译数量（按目标语言过滤）
    let lang = target_lang.unwrap_or_else(|| "zh-CN".to_string());
    let translated: u32 = if let Ok(cache_conn) = super::translator::open_db() {
        cache_conn.query_row(
            "SELECT COUNT(*) FROM translations WHERE lang = ?1",
            rusqlite::params![&lang], |r| r.get(0)
        ).unwrap_or(0)
    } else { 0 };
    let db_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let source_file = get_meta(&conn, "source_file");
    let import_date = get_meta(&conn, "import_date");
    Ok(TagDbStats {
        total_tags: total, translated_tags: translated,
        db_size_bytes: db_size, has_data: total > 0,
        source_file, import_date,
    })
}

/// 查询标签数据库是否正在忙（下载或翻译中）
#[tauri::command]
pub fn is_tag_db_busy() -> (bool, bool) {
    (IS_DOWNLOADING.load(Ordering::SeqCst), IS_TRANSLATING.load(Ordering::SeqCst))
}

/// 下载并导入 Danbooru 标签数据
#[tauri::command]
pub async fn download_danbooru_tags(app: tauri::AppHandle) -> Result<u32, String> {
    if IS_DOWNLOADING.load(Ordering::SeqCst) {
        return Err("下载已在进行中".into());
    }
    IS_DOWNLOADING.store(true, Ordering::SeqCst);
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    let result = download_danbooru_tags_inner(app).await;
    IS_DOWNLOADING.store(false, Ordering::SeqCst);
    result
}

/// 检查远端最新标签文件版本
#[tauri::command]
pub async fn check_tag_db_update() -> Result<String, String> {
    let latest = fetch_latest_tag_filename().await?;
    Ok(latest)
}

/// 从 GitHub API 获取最新的 danbooru pt20 CSV 文件名
async fn fetch_latest_tag_filename() -> Result<String, String> {
    let api_url = "https://api.github.com/repos/DraconicDragon/dbr-e621-lists-archive/contents/tag-lists/danbooru";
    let client = reqwest::Client::builder()
        .user_agent("PurinBox")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    let resp = client.get(api_url).send().await
        .map_err(|e| format!("请求 GitHub API 失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 {}", resp.status()));
    }

    let body = resp.text().await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    let items: Vec<serde_json::Value> = serde_json::from_str(&body)
        .map_err(|e| format!("解析 JSON 失败: {}", e))?;

    // 查找最新的 pt20 文件（按文件名排序，日期越新越靠后）
    let mut candidates: Vec<String> = items.iter()
        .filter_map(|item| item["name"].as_str())
        .filter(|name| name.starts_with("danbooru_") && name.contains("_pt20") && name.ends_with(".csv"))
        .map(|s| s.to_string())
        .collect();

    candidates.sort();
    candidates.last().cloned().ok_or_else(|| "未找到 pt20 标签文件".to_string())
}

async fn download_danbooru_tags_inner(app: tauri::AppHandle) -> Result<u32, String> {
    use tauri::Emitter;

    // 动态获取最新文件名
    let _ = app.emit("tag-db-progress", serde_json::json!({
        "status": "checking", "message": "正在检查最新版本...", "current": 0, "total": 0
    }));

    let filename = fetch_latest_tag_filename().await?;
    let url = format!("https://raw.githubusercontent.com/DraconicDragon/dbr-e621-lists-archive/main/tag-lists/danbooru/{}", filename);

    let _ = app.emit("tag-db-progress", serde_json::json!({
        "status": "downloading", "message": "正在下载标签数据...", "current": 0, "total": 0
    }));

    let client = reqwest::Client::builder()
        .user_agent("PurinBox")
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    let resp = client.get(url).send().await
        .map_err(|e| format!("下载失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }

    let csv_text = resp.text().await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
        return Err("下载已取消".into());
    }

    let _ = app.emit("tag-db-progress", serde_json::json!({
        "status": "importing", "message": "正在导入标签数据库...", "current": 0, "total": 0
    }));

    let lines: Vec<String> = csv_text.lines().map(|s| s.to_string()).collect();
    let total = lines.len() as u32;
    let filename_clone = filename.clone();

    let count = tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let conn = open_tag_db()?;
        conn.execute("DELETE FROM danbooru_tags", [])
            .map_err(|e| format!("清空旧数据失败: {}", e))?;
        conn.execute_batch("BEGIN TRANSACTION;")
            .map_err(|e| format!("开始事务失败: {}", e))?;

        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO danbooru_tags (name, category, post_count, aliases) VALUES (?1, ?2, ?3, ?4)"
        ).map_err(|e| format!("准备语句失败: {}", e))?;

        let mut count = 0u32;
        for line in &lines {
            if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
                conn.execute_batch("ROLLBACK;").ok();
                return Err("导入已取消".into());
            }
            let line = line.trim();
            if line.is_empty() { continue; }
            let (name, category, post_count, aliases) = parse_csv_line(line);
            if name.is_empty() { continue; }
            stmt.execute(rusqlite::params![name, category, post_count, aliases])
                .map_err(|e| format!("插入标签失败: {} - {}", name, e))?;
            count += 1;
        }

        conn.execute_batch("COMMIT;")
            .map_err(|e| format!("提交事务失败: {}", e))?;

        // 保存版本元数据
        set_meta(&conn, "source_file", &filename_clone);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        set_meta(&conn, "import_date", &format!("{}", now));

        Ok(count)
    }).await.map_err(|e| format!("导入任务失败: {}", e))??;

    let _ = app.emit("tag-db-progress", serde_json::json!({
        "status": "done", "message": format!("导入完成，共 {} 个标签 ({})", count, filename),
        "current": count, "total": total
    }));

    Ok(count)
}

/// 解析 CSV 行（处理引号内逗号）
fn parse_csv_line(line: &str) -> (String, i32, i64, String) {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(current.clone());
                current.clear();
            },
            _ => current.push(ch),
        }
    }
    fields.push(current);

    let name = fields.get(0).cloned().unwrap_or_default();
    let category: i32 = fields.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let post_count: i64 = fields.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let aliases = fields.get(3).cloned().unwrap_or_default();

    (name, category, post_count, aliases)
}

/// 取消下载
#[tauri::command]
pub fn cancel_tag_db_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
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
    pub aliases: String,
    pub translated: Option<String>,
}


/// 标签自动补全搜索
#[tauri::command]
pub fn search_tags(query: String, limit: Option<u32>, target_lang: Option<String>) -> Result<Vec<TagSuggestion>, String> {
    let path = get_tag_db_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let conn = open_tag_db()?;
    let limit = limit.unwrap_or(10).min(50);
    let query_lower = query.to_lowercase().replace(' ', "_");

    if query_lower.is_empty() {
        return Ok(vec![]);
    }

    // 先精确前缀匹配，再模糊包含匹配，合并去重
    let mut results: Vec<TagSuggestion> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. 前缀匹配（优先）
    let prefix_pattern = format!("{}%", query_lower);
    let mut stmt = conn.prepare(
        "SELECT name, category, post_count, aliases FROM danbooru_tags
         WHERE name LIKE ?1 ORDER BY post_count DESC LIMIT ?2"
    ).map_err(|e| format!("查询失败: {}", e))?;

    let rows = stmt.query_map(rusqlite::params![prefix_pattern, limit], |row| {
        Ok(TagSuggestion {
            name: row.get(0)?,
            category: row.get(1)?,
            post_count: row.get(2)?,
            aliases: row.get(3)?,
            translated: None,
        })
    }).map_err(|e| format!("查询失败: {}", e))?;

    for row in rows {
        if let Ok(tag) = row {
            seen.insert(tag.name.clone());
            results.push(tag);
        }
    }

    // 2. 如果前缀匹配不够，用包含匹配补充
    if results.len() < limit as usize {
        let remaining = limit as usize - results.len();
        let contain_pattern = format!("%{}%", query_lower);
        let mut stmt2 = conn.prepare(
            "SELECT name, category, post_count, aliases FROM danbooru_tags
             WHERE name LIKE ?1 ORDER BY post_count DESC LIMIT ?2"
        ).map_err(|e| format!("查询失败: {}", e))?;

        let rows2 = stmt2.query_map(rusqlite::params![contain_pattern, remaining + 20], |row| {
            Ok(TagSuggestion {
                name: row.get(0)?,
                category: row.get(1)?,
                post_count: row.get(2)?,
                aliases: row.get(3)?,
                translated: None,
            })
        }).map_err(|e| format!("查询失败: {}", e))?;

        for row in rows2 {
            if results.len() >= limit as usize { break; }
            if let Ok(tag) = row {
                if !seen.contains(&tag.name) {
                    seen.insert(tag.name.clone());
                    results.push(tag);
                }
            }
        }
    }

    // 3. 还不够的话，搜索别名
    if results.len() < limit as usize {
        let remaining = limit as usize - results.len();
        let alias_pattern = format!("%{}%", query_lower);
        let mut stmt3 = conn.prepare(
            "SELECT name, category, post_count, aliases FROM danbooru_tags
             WHERE aliases LIKE ?1 ORDER BY post_count DESC LIMIT ?2"
        ).map_err(|e| format!("查询失败: {}", e))?;

        let rows3 = stmt3.query_map(rusqlite::params![alias_pattern, remaining + 20], |row| {
            Ok(TagSuggestion {
                name: row.get(0)?,
                category: row.get(1)?,
                post_count: row.get(2)?,
                aliases: row.get(3)?,
                translated: None,
            })
        }).map_err(|e| format!("查询失败: {}", e))?;

        for row in rows3 {
            if results.len() >= limit as usize { break; }
            if let Ok(tag) = row {
                if !seen.contains(&tag.name) {
                    seen.insert(tag.name.clone());
                    results.push(tag);
                }
            }
        }
    }

    // 4. 从翻译缓存查找翻译
    let lang = target_lang.unwrap_or_else(|| "zh-CN".to_string());
    if !lang.is_empty() {
        if let Ok(trans_conn) = super::translator::open_db() {
            for tag in results.iter_mut() {
                if let Ok(tr) = trans_conn.query_row(
                    "SELECT translated FROM translations WHERE tag = ?1 AND lang = ?2",
                    rusqlite::params![&tag.name, &lang],
                    |row| row.get::<_, String>(0),
                ) {
                    tag.translated = Some(tr);
                }
            }
        }
    }

    Ok(results)
}

/// 批量翻译标签数据库中的标签（使用 Google 翻译）
#[tauri::command]
pub async fn translate_tag_db(
    app: tauri::AppHandle,
    target_lang: String,
    batch_size: Option<u32>,
) -> Result<u32, String> {
    if IS_TRANSLATING.load(Ordering::SeqCst) {
        return Err("翻译已在进行中".into());
    }
    IS_TRANSLATING.store(true, Ordering::SeqCst);
    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    let result = translate_tag_db_inner(app, target_lang, batch_size).await;
    IS_TRANSLATING.store(false, Ordering::SeqCst);
    result
}

async fn translate_tag_db_inner(
    app: tauri::AppHandle,
    target_lang: String,
    batch_size: Option<u32>,
) -> Result<u32, String> {
    use tauri::Emitter;

    let batch = batch_size.unwrap_or(80).min(200) as usize;

    let tl = target_lang.clone();
    let untranslated: Vec<String> = tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
        let conn = open_tag_db()?;
        // 获取所有标签名
        let all_tags: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT name FROM danbooru_tags ORDER BY post_count DESC"
            ).map_err(|e| format!("查询失败: {}", e))?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("查询失败: {}", e))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        // 从翻译缓存中查找已有翻译的标签
        let already: std::collections::HashSet<String> = if let Ok(cache_conn) = super::translator::open_db() {
            let result: Vec<String> = {
                let mut stmt = cache_conn.prepare(
                    "SELECT tag FROM translations WHERE lang = ?1"
                ).map_err(|e| format!("查询翻译缓存失败: {}", e))?;
                let rows = stmt.query_map(rusqlite::params![&tl], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("查询翻译缓存失败: {}", e))?;
                rows.filter_map(|r| r.ok()).collect()
            };
            result.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        };
        Ok(all_tags.into_iter().filter(|t| !already.contains(t)).collect())
    }).await.map_err(|e| format!("获取未翻译标签失败: {}", e))??;

    let total = untranslated.len() as u32;
    if total == 0 {
        let _ = app.emit("tag-db-progress", serde_json::json!({
            "status": "done", "message": "所有标签已翻译", "current": 0, "total": 0
        }));
        return Ok(0);
    }

    let _ = app.emit("tag-db-progress", serde_json::json!({
        "status": "translating", "message": format!("开始翻译 {} 个标签...", total),
        "current": 0, "total": total
    }));

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    let mut translated_count = 0u32;
    let target = target_lang.clone();

    for chunk in untranslated.chunks(batch) {
        if DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
            return Err("翻译已取消".into());
        }

        let tags: Vec<String> = chunk.to_vec();
        let text = tags.join("\n");

        let url = format!(
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl=en&tl={}&dt=t&q={}",
            target,
            urlencoding::encode(&text)
        );

        let resp = client.get(&url).send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    let mut translated_text = String::new();
                    if let Some(sentences) = json.get(0).and_then(|v| v.as_array()) {
                        for sentence in sentences {
                            if let Some(t) = sentence.get(0).and_then(|v| v.as_str()) {
                                translated_text.push_str(t);
                            }
                        }
                    }
                    let translated_parts: Vec<&str> = translated_text.split('\n').collect();

                    let pairs: Vec<(String, String)> = tags.iter()
                        .zip(translated_parts.iter())
                        .map(|(src, tr)| (src.clone(), tr.trim().to_string()))
                        .filter(|(_, tr)| !tr.is_empty())
                        .collect();

                    let batch_count = pairs.len() as u32;

                    let target_lang_clone = target.clone();
                    tokio::task::spawn_blocking(move || -> Result<(), String> {
                        // 只写入翻译缓存数据库
                        let cache_conn = super::translator::open_db()
                            .map_err(|e| format!("打开翻译缓存失败: {}", e))?;
                        cache_conn.execute_batch("BEGIN").ok();
                        let mut stmt = cache_conn.prepare(
                            "INSERT OR REPLACE INTO translations (tag, translated, lang) VALUES (?1, ?2, ?3)"
                        ).map_err(|e| format!("准备写入语句失败: {}", e))?;
                        for (source, translated) in &pairs {
                            stmt.execute(rusqlite::params![source, translated, &target_lang_clone]).ok();
                        }
                        drop(stmt);
                        cache_conn.execute_batch("COMMIT").ok();
                        Ok(())
                    }).await.map_err(|e| format!("写入翻译失败: {}", e))??;

                    translated_count += batch_count;
                }
            },
            Ok(r) => {
                let _ = app.emit("tag-db-progress", serde_json::json!({
                    "status": "warning", "message": format!("翻译请求返回 {}, 跳过当前批次", r.status()),
                    "current": translated_count, "total": total
                }));
            },
            Err(e) => {
                let _ = app.emit("tag-db-progress", serde_json::json!({
                    "status": "warning", "message": format!("翻译请求失败: {}, 跳过当前批次", e),
                    "current": translated_count, "total": total
                }));
            }
        }

        let _ = app.emit("tag-db-progress", serde_json::json!({
            "status": "translating",
            "message": format!("已翻译 {}/{}", translated_count, total),
            "current": translated_count, "total": total
        }));

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let _ = app.emit("tag-db-progress", serde_json::json!({
        "status": "done", "message": format!("翻译完成，共翻译 {} 个标签", translated_count),
        "current": translated_count, "total": total
    }));

    Ok(translated_count)
}

