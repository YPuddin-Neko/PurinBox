use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

/// 翻译缓存数据库路径
static DB_PATH: OnceLock<PathBuf> = OnceLock::new();

fn get_db_path() -> PathBuf {
    DB_PATH.get().cloned().unwrap_or_else(|| PathBuf::from("tag_translations.db"))
}

fn open_db() -> Result<Connection, String> {
    let path = get_db_path();
    let conn = Connection::open(&path)
        .map_err(|e| format!("打开翻译缓存数据库失败: {}", e))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS translations (
            tag TEXT PRIMARY KEY,
            translated TEXT NOT NULL,
            lang TEXT NOT NULL DEFAULT 'zh-CN',
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );"
    ).map_err(|e| format!("创建翻译缓存表失败: {}", e))?;
    Ok(conn)
}

/// 初始化翻译缓存数据库路径（在 app 启动时调用）
pub fn init_db_path(app_data_dir: PathBuf) {
    let db_path = app_data_dir.join("tag_translations.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = DB_PATH.set(db_path);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateResult {
    pub translations: Vec<TranslatedItem>,
    pub cached_count: usize,
    pub translated_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslatedItem {
    pub source: String,
    pub translated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub total: usize,
    pub db_size_bytes: u64,
}

// ═══════════════════════════════════════
//  百度翻译
// ═══════════════════════════════════════

#[derive(Debug, Deserialize)]
struct BaiduResponse {
    trans_result: Option<Vec<BaiduTransItem>>,
    error_code: Option<String>,
    error_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BaiduTransItem {
    src: String,
    dst: String,
}

fn md5_hex(input: &str) -> String {
    use std::fmt::Write;
    let digest = md5::compute(input.as_bytes());
    let mut s = String::with_capacity(32);
    for byte in digest.iter() {
        write!(s, "{:02x}", byte).unwrap();
    }
    s
}

async fn translate_baidu(
    client: &reqwest::Client,
    texts: &[String],
    appid: &str,
    secret_key: &str,
) -> Result<Vec<String>, String> {
    let text = texts.join("\n");
    let salt = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    let sign_str = format!("{}{}{}{}", appid, &text, &salt, secret_key);
    let sign = md5_hex(&sign_str);

    let params = [
        ("q", text.as_str()),
        ("from", "en"),
        ("to", "zh"),
        ("appid", appid),
        ("salt", &salt),
        ("sign", &sign),
    ];

    let resp = client
        .post("https://fanyi-api.baidu.com/api/trans/vip/translate")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("百度翻译请求失败: {}\n请检查网络连接", e))?;

    let body = resp.text().await
        .map_err(|e| format!("读取百度翻译响应失败: {}", e))?;

    let baidu_resp: BaiduResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析百度翻译响应失败: {}", e))?;

    if let Some(code) = &baidu_resp.error_code {
        let msg = baidu_resp.error_msg.as_deref().unwrap_or("未知错误");
        let hint = match code.as_str() {
            "52001" => "请求超时，请稍后重试",
            "52002" => "系统错误，请稍后重试",
            "52003" => "APP ID 无效，请检查设置中的百度翻译 APP ID",
            "54000" => "缺少必要参数，请检查配置",
            "54001" => "签名错误，请检查设置中的百度翻译密钥是否正确",
            "54003" => "访问频率受限，标准版 QPS=1，请稍后重试",
            "54004" => "账户余额不足，请前往百度翻译开放平台充值",
            "54005" => "请求内容过长，请减少标签数量后重试",
            "58000" => "客户端 IP 非法，请在百度翻译平台添加 IP 白名单",
            "58001" => "不支持该语言，请检查翻译语言设置",
            "58002" => "服务已关闭，请在百度翻译平台开启翻译服务",
            "90107" => "认证未通过，请完成百度翻译平台的身份认证",
            _ => "请参考百度翻译错误码文档",
        };
        return Err(format!("百度翻译错误 [{}]: {}\n{}", code, msg, hint));
    }

    match baidu_resp.trans_result {
        Some(items) => Ok(items.iter().map(|item| item.dst.trim().to_string()).collect()),
        None => Err("百度翻译返回空结果".to_string()),
    }
}

// ═══════════════════════════════════════
//  Google 翻译
// ═══════════════════════════════════════

async fn translate_google(
    client: &reqwest::Client,
    texts: &[String],
    target_lang: &str,
) -> Result<Vec<String>, String> {
    let text = texts.join("\n");
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl=en&tl={}&dt=t&q={}",
        target_lang,
        urlencoding::encode(&text)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("Google 翻译请求失败: {}\n请检查网络是否能访问 Google 服务（可能需要代理/VPN）", e))?;

    let status = resp.status();
    let body = resp.text().await
        .map_err(|e| format!("读取 Google 翻译响应失败: {}", e))?;

    if !status.is_success() {
        return Err(format!("Google 翻译返回错误状态 {}\n请检查网络是否能访问 Google 服务", status));
    }

    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("解析 Google 翻译响应失败: {}\n可能是请求被拦截，请稍后重试", e))?;

    let mut translated_text = String::new();
    if let Some(sentences) = json.get(0).and_then(|v| v.as_array()) {
        for sentence in sentences {
            if let Some(t) = sentence.get(0).and_then(|v| v.as_str()) {
                translated_text.push_str(t);
            }
        }
    }

    Ok(translated_text.split('\n').map(|s| s.trim().to_string()).collect())
}

// ═══════════════════════════════════════
//  有道翻译
// ═══════════════════════════════════════

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct YoudaoResponse {
    #[serde(rename = "errorCode")]
    error_code: String,
    translation: Option<Vec<String>>,
    query: Option<String>,
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn youdao_truncate(q: &str) -> String {
    let len = q.len();
    if len <= 20 {
        q.to_string()
    } else {
        format!("{}{}{}", &q[..10], len, &q[len - 10..])
    }
}

async fn translate_youdao(
    client: &reqwest::Client,
    texts: &[String],
    app_key: &str,
    app_secret: &str,
) -> Result<Vec<String>, String> {
    let text = texts.join("\n");
    let salt = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());
    let curtime = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs());

    let input = youdao_truncate(&text);
    let sign_str = format!("{}{}{}{}{}", app_key, input, salt, curtime, app_secret);
    let sign = sha256_hex(&sign_str);

    let params = [
        ("q", text.as_str()),
        ("from", "en"),
        ("to", "zh-CHS"),
        ("appKey", app_key),
        ("salt", &salt),
        ("sign", &sign),
        ("signType", "v3"),
        ("curtime", &curtime),
    ];

    let resp = client
        .post("https://openapi.youdao.com/api")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("有道翻译请求失败: {}\n请检查网络连接", e))?;

    let body = resp.text().await
        .map_err(|e| format!("读取有道翻译响应失败: {}", e))?;

    let youdao_resp: YoudaoResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析有道翻译响应失败: {}", e))?;

    if youdao_resp.error_code != "0" {
        let hint = match youdao_resp.error_code.as_str() {
            "101" => "缺少必要参数，请检查配置",
            "102" => "不支持的语言类型",
            "103" => "翻译文本过长，请减少标签数量",
            "108" => "应用 ID 无效，请检查设置中的有道翻译应用 ID",
            "110" => "超出请求限制，请稍后重试",
            "111" => "开发者账号无效",
            "113" => "查询内容为空",
            "202" => "签名错误，请检查设置中的有道翻译应用密钥是否正确",
            "401" => "账户已欠费，请前往有道智云平台充值",
            "411" => "访问频率受限，请稍后重试",
            _ => "请参考有道翻译错误码文档",
        };
        return Err(format!("有道翻译错误 [{}]: {}", youdao_resp.error_code, hint));
    }

    match youdao_resp.translation {
        Some(arr) => {
            // 有道返回整段翻译在一个元素中，按 \n 拆分
            let joined = arr.join("");
            Ok(joined.split('\n').map(|s| s.trim().to_string()).collect())
        }
        None => Err("有道翻译返回空结果".to_string()),
    }
}

// ═══════════════════════════════════════
//  微软必应翻译
// ═══════════════════════════════════════

#[derive(Debug, Deserialize)]
struct BingTransResponse {
    translations: Vec<BingTranslation>,
}

#[derive(Debug, Deserialize)]
struct BingTranslation {
    text: String,
}

#[derive(Debug, Deserialize)]
struct BingErrorResponse {
    error: Option<BingError>,
}

#[derive(Debug, Deserialize)]
struct BingError {
    code: Option<i64>,
    message: Option<String>,
}

async fn translate_bing(
    client: &reqwest::Client,
    texts: &[String],
    subscription_key: &str,
    region: &str,
) -> Result<Vec<String>, String> {
    let body: Vec<serde_json::Value> = texts
        .iter()
        .map(|t| serde_json::json!({"Text": t}))
        .collect();

    let mut req = client
        .post("https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&from=en&to=zh-Hans")
        .header("Ocp-Apim-Subscription-Key", subscription_key)
        .header("Content-Type", "application/json")
        .json(&body);

    if !region.is_empty() {
        req = req.header("Ocp-Apim-Subscription-Region", region);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("必应翻译请求失败: {}\n请检查网络连接", e))?;

    let status = resp.status();
    let resp_body = resp.text().await
        .map_err(|e| format!("读取必应翻译响应失败: {}", e))?;

    if !status.is_success() {
        // 尝试解析错误
        if let Ok(err_resp) = serde_json::from_str::<BingErrorResponse>(&resp_body) {
            if let Some(err) = err_resp.error {
                let hint = match err.code.unwrap_or(0) {
                    401000 => "请求未授权，请检查设置中的必应翻译订阅密钥",
                    401001 => "订阅密钥无效，请检查设置中的必应翻译订阅密钥",
                    403001 => "请求超出免费额度，请升级订阅计划",
                    429000 | 429001 | 429002 => "请求过于频繁，请稍后重试",
                    _ => "请参考微软翻译 API 文档",
                };
                return Err(format!("必应翻译错误 [{}]: {}\n{}", 
                    err.code.unwrap_or(0),
                    err.message.unwrap_or_default(),
                    hint
                ));
            }
        }
        return Err(format!("必应翻译返回错误状态 {}", status));
    }

    let results: Vec<BingTransResponse> = serde_json::from_str(&resp_body)
        .map_err(|e| format!("解析必应翻译响应失败: {}", e))?;

    Ok(results.iter().map(|r| {
        r.translations.first().map(|t| t.text.trim().to_string()).unwrap_or_default()
    }).collect())
}

// ═══════════════════════════════════════
//  统一翻译入口
// ═══════════════════════════════════════

#[tauri::command]
pub async fn translate_tags(
    app: tauri::AppHandle,
    tags: Vec<String>,
    target_lang: String,
    provider: String,
    baidu_appid: Option<String>,
    baidu_key: Option<String>,
    youdao_app_key: Option<String>,
    youdao_app_secret: Option<String>,
    bing_key: Option<String>,
    bing_region: Option<String>,
) -> Result<TranslateResult, String> {
    if tags.is_empty() {
        return Ok(TranslateResult { translations: vec![], cached_count: 0, translated_count: 0 });
    }

    let conn = open_db()?;

    // 1. 查缓存
    let mut cached: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut uncached: Vec<String> = Vec::new();

    for tag in &tags {
        let mut stmt = conn.prepare("SELECT translated FROM translations WHERE tag = ?1 AND lang = ?2")
            .map_err(|e| format!("查询缓存失败: {}", e))?;
        let result: Result<String, _> = stmt.query_row(rusqlite::params![tag, &target_lang], |row| row.get(0));
        match result {
            Ok(tr) => { cached.insert(tag.clone(), tr); },
            Err(_) => { uncached.push(tag.clone()); },
        }
    }

    let cached_count = cached.len();
    let total_count = tags.len();
    let mut translated_count = 0;

    // 发送初始进度（已缓存的部分）
    {
        use tauri::Emitter;
        let _ = app.emit("translate-progress", serde_json::json!({
            "current": cached_count,
            "total": total_count
        }));
    }

    // 2. 翻译未缓存的
    if !uncached.is_empty() {
        let prepared: Vec<String> = uncached.iter().map(|t| t.replace('_', " ")).collect();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

        // 不同供应商批量大小和 QPS 限制不同
        let batch_size = match provider.as_str() {
            "baidu" => 20,   // QPS=1
            "youdao" => 20,  // QPS 有限
            "bing" => 25,    // 单次最多 25 个文本
            _ => 50,         // Google
        };

        for chunk_start in (0..prepared.len()).step_by(batch_size) {
            let chunk_end = std::cmp::min(chunk_start + batch_size, prepared.len());
            let chunk = &prepared[chunk_start..chunk_end];
            let original_chunk = &uncached[chunk_start..chunk_end];

            // 需要 QPS 限制的供应商
            if chunk_start > 0 && matches!(provider.as_str(), "baidu" | "youdao") {
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            }

            let translated_lines = match provider.as_str() {
                "baidu" => {
                    let appid = baidu_appid.as_deref().unwrap_or("");
                    let key = baidu_key.as_deref().unwrap_or("");
                    if appid.is_empty() || key.is_empty() {
                        return Err("百度翻译需要配置 APP ID 和密钥\n请在「设置 → 翻译设置」中填写".to_string());
                    }
                    translate_baidu(&client, &chunk.to_vec(), appid, key).await?
                }
                "youdao" => {
                    let app_key = youdao_app_key.as_deref().unwrap_or("");
                    let app_secret = youdao_app_secret.as_deref().unwrap_or("");
                    if app_key.is_empty() || app_secret.is_empty() {
                        return Err("有道翻译需要配置应用 ID 和应用密钥\n请在「设置 → 翻译设置」中填写".to_string());
                    }
                    translate_youdao(&client, &chunk.to_vec(), app_key, app_secret).await?
                }
                "bing" => {
                    let key = bing_key.as_deref().unwrap_or("");
                    if key.is_empty() {
                        return Err("必应翻译需要配置订阅密钥\n请在「设置 → 翻译设置」中填写".to_string());
                    }
                    let region = bing_region.as_deref().unwrap_or("");
                    translate_bing(&client, &chunk.to_vec(), key, region).await?
                }
                _ => {
                    translate_google(&client, &chunk.to_vec(), &target_lang).await?
                }
            };

            for (i, original) in original_chunk.iter().enumerate() {
                let tr = translated_lines
                    .get(i)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();

                let final_tr = if tr.to_lowercase() == original.replace('_', " ").to_lowercase() {
                    String::new()
                } else {
                    tr
                };

                let _ = conn.execute(
                    "INSERT OR REPLACE INTO translations (tag, translated, lang) VALUES (?1, ?2, ?3)",
                    rusqlite::params![original, &final_tr, &target_lang],
                );

                cached.insert(original.clone(), final_tr);
                translated_count += 1;
            }

            // 发送翻译批次进度
            {
                use tauri::Emitter;
                let _ = app.emit("translate-progress", serde_json::json!({
                    "current": cached_count + translated_count,
                    "total": total_count
                }));
            }
        }
    }

    // 3. 按原始顺序返回
    let translations: Vec<TranslatedItem> = tags.iter().map(|tag| {
        TranslatedItem {
            source: tag.clone(),
            translated: cached.get(tag).cloned().unwrap_or_default(),
        }
    }).collect();

    Ok(TranslateResult { translations, cached_count, translated_count })
}

/// 获取翻译缓存统计
#[tauri::command]
pub fn get_translation_cache_stats() -> Result<CacheStats, String> {
    let conn = open_db()?;
    let total: usize = conn.query_row("SELECT COUNT(*) FROM translations", [], |row| row.get(0))
        .map_err(|e| format!("查询缓存统计失败: {}", e))?;

    let db_path = get_db_path();
    let db_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    Ok(CacheStats { total, db_size_bytes })
}

/// 清空翻译缓存
#[tauri::command]
pub fn clear_translation_cache() -> Result<(), String> {
    let conn = open_db()?;
    conn.execute("DELETE FROM translations", [])
        .map_err(|e| format!("清空翻译缓存失败: {}", e))?;
    conn.execute("VACUUM", [])
        .map_err(|e| format!("压缩数据库失败: {}", e))?;
    Ok(())
}

/// 测试翻译供应商可用性：翻译 "hello" 并返回结果
#[tauri::command]
pub async fn test_translation(
    provider: String,
    baidu_appid: Option<String>,
    baidu_key: Option<String>,
    youdao_app_key: Option<String>,
    youdao_app_secret: Option<String>,
    bing_key: Option<String>,
    bing_region: Option<String>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

    let test_texts = vec!["hello".to_string()];

    let results = match provider.as_str() {
        "baidu" => {
            let appid = baidu_appid.as_deref().unwrap_or("");
            let key = baidu_key.as_deref().unwrap_or("");
            if appid.is_empty() || key.is_empty() {
                return Err("请先填写百度翻译 APP ID 和密钥".to_string());
            }
            translate_baidu(&client, &test_texts, appid, key).await?
        }
        "youdao" => {
            let app_key = youdao_app_key.as_deref().unwrap_or("");
            let app_secret = youdao_app_secret.as_deref().unwrap_or("");
            if app_key.is_empty() || app_secret.is_empty() {
                return Err("请先填写有道翻译应用 ID 和应用密钥".to_string());
            }
            translate_youdao(&client, &test_texts, app_key, app_secret).await?
        }
        "bing" => {
            let key = bing_key.as_deref().unwrap_or("");
            if key.is_empty() {
                return Err("请先填写必应翻译订阅密钥".to_string());
            }
            let region = bing_region.as_deref().unwrap_or("");
            translate_bing(&client, &test_texts, key, region).await?
        }
        _ => {
            translate_google(&client, &test_texts, "zh-CN").await?
        }
    };

    let translated = results.first().cloned().unwrap_or_default();
    Ok(format!("hello → {}", translated))
}
