use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Emitter;

use super::config_paths::default_tagcache_dir;

/// 翻译缓存数据库路径（可运行时修改）
static DB_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

const DB_FILE_NAME: &str = "tag_translations.db";
const CACHE_PATH_CONFIG_FILE: &str = "translation_cache.json";
const TRANSLATE_PROGRESS_EVENT: &str = "translate-progress";

#[derive(Default, Serialize, Deserialize)]
struct CachePathConfig {
    #[serde(default)]
    directory: Option<PathBuf>,
}

const CREATE_TRANSLATIONS_SQL: &str = "CREATE TABLE IF NOT EXISTS translations (
    tag TEXT NOT NULL,
    translated TEXT NOT NULL,
    lang TEXT NOT NULL DEFAULT 'zh-CN',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    PRIMARY KEY (tag, lang)
);";

const UPSERT_TRANSLATION_SQL: &str =
    "INSERT OR REPLACE INTO translations (tag, translated, lang) VALUES (?1, ?2, ?3)";

fn get_db_path() -> PathBuf {
    cached_db_path(&DB_PATH, || {
        let config: CachePathConfig =
            super::config_paths::load_json_config_or_default(CACHE_PATH_CONFIG_FILE);
        config.directory.unwrap_or_else(default_tagcache_dir)
    })
}

fn cached_db_path(
    cache: &Mutex<Option<PathBuf>>,
    load_directory: impl FnOnce() -> PathBuf,
) -> PathBuf {
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(|| load_directory().join(DB_FILE_NAME))
        .clone()
}

pub fn open_db() -> Result<Connection, String> {
    let path = get_db_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开翻译缓存数据库失败: {}", e))?;
    conn.execute_batch(CREATE_TRANSLATIONS_SQL)
        .map_err(|e| format!("创建翻译缓存表失败: {}", e))?;
    // 旧表的 PRIMARY KEY 只有 tag 一列（不含 lang），按新 schema 重建
    let needs_migrate = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('translations') WHERE pk > 0",
            [],
            |row| row.get::<_, i64>(0),
        )
        .is_ok_and(|pk_cols| pk_cols < 2);
    if needs_migrate {
        let _ = conn.execute_batch(&format!(
            "DROP TABLE translations; {}",
            CREATE_TRANSLATIONS_SQL
        ));
    }
    Ok(conn)
}

/// 按 (tag, lang) 查缓存，只返回命中的条目
pub(super) fn lookup_cached<'a>(
    conn: &Connection,
    tags: impl IntoIterator<Item = &'a str>,
    lang: &str,
) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT translated FROM translations WHERE tag = ?1 AND lang = ?2")?;
    let mut found = HashMap::new();
    for tag in tags {
        if let Ok(tr) = stmt.query_row(rusqlite::params![tag, lang], |row| row.get::<_, String>(0))
        {
            found.insert(tag.to_string(), tr);
        }
    }
    Ok(found)
}

/// 一个事务写入一批 (tag, 译文)；只有准备语句失败才返回 Err
pub(super) fn upsert_translations(
    conn: &Connection,
    pairs: &[(String, String)],
    lang: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(UPSERT_TRANSLATION_SQL)?;
    let _ = conn.execute_batch("BEGIN");
    for (tag, translated) in pairs {
        // 单行失败跳过，不拖垮整批
        let _ = stmt.execute(rusqlite::params![tag, translated, lang]);
    }
    // 提交失败就回滚：调用方可能一直持有这个连接，不能留着没结束的事务
    if conn.execute_batch("COMMIT").is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    Ok(())
}

/// 某语言的缓存条数，查询失败按 0
pub(super) fn count_for_lang(conn: &Connection, lang: &str) -> usize {
    conn.query_row(
        "SELECT COUNT(*) FROM translations WHERE lang = ?1",
        [lang],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

pub(super) fn tags_for_lang(conn: &Connection, lang: &str) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT tag FROM translations WHERE lang = ?1")?;
    let rows = stmt.query_map([lang], |row| row.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn get_cache_path() -> String {
    let path = get_db_path();
    path.parent()
        .unwrap_or(path.as_path())
        .to_string_lossy()
        .to_string()
}

/// 修改缓存路径（空字符串则重置为默认路径）
#[tauri::command]
pub fn set_cache_path(path: String) -> Result<String, String> {
    let cache_dir = if path.is_empty() {
        default_tagcache_dir()
    } else {
        PathBuf::from(&path)
    };
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("创建缓存目录失败: {}", e))?;
    let db_path = cache_dir.join(DB_FILE_NAME);
    let mut guard = DB_PATH.lock().unwrap_or_else(|e| e.into_inner());
    super::config_paths::save_json_config(
        CACHE_PATH_CONFIG_FILE,
        &CachePathConfig {
            directory: if path.is_empty() {
                None
            } else {
                Some(cache_dir.clone())
            },
        },
        "写入翻译缓存路径配置失败",
    )?;
    *guard = Some(db_path);
    Ok(cache_dir.to_string_lossy().to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateResult {
    pub translations: Vec<TranslatedItem>,
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
    pub zh_cn: usize,
    pub ja: usize,
    pub ko: usize,
}

// ═══════════════════════════════════════
//  目标语言代码映射
//  应用内统一使用 zh-CN / ja / ko（见设置页选项），各服务商语言代码不同。
//  未知值按 zh-CN 处理。
// ═══════════════════════════════════════

fn baidu_lang_code(target_lang: &str) -> &'static str {
    match target_lang {
        "ja" => "jp",
        "ko" => "kor",
        _ => "zh",
    }
}

fn youdao_lang_code(target_lang: &str) -> &'static str {
    match target_lang {
        "ja" => "ja",
        "ko" => "ko",
        _ => "zh-CHS",
    }
}

fn bing_lang_code(target_lang: &str) -> &'static str {
    match target_lang {
        "ja" => "ja",
        "ko" => "ko",
        _ => "zh-Hans",
    }
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
struct BaiduTransItem {
    dst: String,
}

// 百度翻译 API 签名要求使用 MD5（非安全/加密用途，仅用于 API 请求签名）
// See: https://fanyi-api.baidu.com/doc/21
fn baidu_sign(input: &str) -> String {
    format!("{:x}", md5::compute(input.as_bytes()))
}

async fn translate_baidu(
    client: &reqwest::Client,
    texts: &[String],
    target_lang: &str,
    appid: &str,
    secret_key: &str,
) -> Result<Vec<String>, String> {
    let text = texts.join("\n");
    let salt = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let sign_str = format!("{}{}{}{}", appid, text, salt, secret_key);
    let sign = baidu_sign(&sign_str);

    let params = [
        ("q", text.as_str()),
        ("from", "en"),
        ("to", baidu_lang_code(target_lang)),
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

    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取百度翻译响应失败: {}", e))?;

    let baidu_resp: BaiduResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析百度翻译响应失败: {}", e))?;

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
        Some(items) => Ok(items
            .iter()
            .map(|item| item.dst.trim().to_string())
            .collect()),
        None => Err("百度翻译返回空结果".to_string()),
    }
}

// ═══════════════════════════════════════
//  Google 翻译
// ═══════════════════════════════════════

pub(super) async fn translate_google(
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
        .map_err(|e| {
            format!(
                "Google 翻译请求失败: {}\n请检查网络是否能访问 Google 服务（可能需要代理/VPN）",
                e
            )
        })?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 Google 翻译响应失败: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Google 翻译返回错误状态 {}\n请检查网络是否能访问 Google 服务",
            status
        ));
    }

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        format!(
            "解析 Google 翻译响应失败: {}\n可能是请求被拦截，请稍后重试",
            e
        )
    })?;

    let mut translated_text = String::new();
    if let Some(sentences) = json.get(0).and_then(|v| v.as_array()) {
        for sentence in sentences {
            if let Some(t) = sentence.get(0).and_then(|v| v.as_str()) {
                translated_text.push_str(t);
            }
        }
    }

    Ok(translated_text
        .split('\n')
        .map(|s| s.trim().to_string())
        .collect())
}

// ═══════════════════════════════════════
//  有道翻译
// ═══════════════════════════════════════

#[derive(Debug, Deserialize)]
struct YoudaoResponse {
    #[serde(rename = "errorCode")]
    error_code: String,
    translation: Option<Vec<String>>,
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn youdao_truncate(q: &str) -> String {
    // 有道签名规则按字符数截取，必须按 char 边界切分，按字节切 UTF-8 多字节文本会 panic
    let chars: Vec<char> = q.chars().collect();
    let len = chars.len();
    if len <= 20 {
        q.to_string()
    } else {
        let head: String = chars[..10].iter().collect();
        let tail: String = chars[len - 10..].iter().collect();
        format!("{}{}{}", head, len, tail)
    }
}

async fn translate_youdao(
    client: &reqwest::Client,
    texts: &[String],
    target_lang: &str,
    app_key: &str,
    app_secret: &str,
) -> Result<Vec<String>, String> {
    let text = texts.join("\n");
    let salt = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let curtime = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );

    let input = youdao_truncate(&text);
    let sign_str = format!("{}{}{}{}{}", app_key, input, salt, curtime, app_secret);
    let sign = sha256_hex(&sign_str);

    let params = [
        ("q", text.as_str()),
        ("from", "en"),
        ("to", youdao_lang_code(target_lang)),
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

    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取有道翻译响应失败: {}", e))?;

    let youdao_resp: YoudaoResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析有道翻译响应失败: {}", e))?;

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
        return Err(format!(
            "有道翻译错误 [{}]: {}",
            youdao_resp.error_code, hint
        ));
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
    target_lang: &str,
    subscription_key: &str,
    region: &str,
) -> Result<Vec<String>, String> {
    let body: Vec<serde_json::Value> = texts
        .iter()
        .map(|t| serde_json::json!({"Text": t}))
        .collect();

    let mut req = client
        .post(format!(
            "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&from=en&to={}",
            bing_lang_code(target_lang)
        ))
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
    let resp_body = resp
        .text()
        .await
        .map_err(|e| format!("读取必应翻译响应失败: {}", e))?;

    if !status.is_success() {
        if let Ok(err_resp) = serde_json::from_str::<BingErrorResponse>(&resp_body) {
            if let Some(err) = err_resp.error {
                let hint = match err.code.unwrap_or(0) {
                    401000 => "请求未授权，请检查设置中的必应翻译订阅密钥",
                    401001 => "订阅密钥无效，请检查设置中的必应翻译订阅密钥",
                    403001 => "请求超出免费额度，请升级订阅计划",
                    429000..=429002 => "请求过于频繁，请稍后重试",
                    _ => "请参考微软翻译 API 文档",
                };
                return Err(format!(
                    "必应翻译错误 [{}]: {}\n{}",
                    err.code.unwrap_or(0),
                    err.message.unwrap_or_default(),
                    hint
                ));
            }
        }
        return Err(format!("必应翻译返回错误状态 {}", status));
    }

    let results: Vec<BingTransResponse> =
        serde_json::from_str(&resp_body).map_err(|e| format!("解析必应翻译响应失败: {}", e))?;

    Ok(results
        .iter()
        .map(|r| {
            r.translations
                .first()
                .map(|t| t.text.trim().to_string())
                .unwrap_or_default()
        })
        .collect())
}

// ═══════════════════════════════════════
//  统一翻译入口
// ═══════════════════════════════════════

/// 各服务商的凭据，未填为空串
struct ProviderCreds<'a> {
    baidu_appid: &'a str,
    baidu_key: &'a str,
    youdao_key: &'a str,
    youdao_secret: &'a str,
    bing_key: &'a str,
    bing_region: &'a str,
}

impl<'a> ProviderCreds<'a> {
    /// 取自翻译命令的参数（顺序同命令参数），没传的按空串
    fn new(
        baidu_appid: &'a Option<String>,
        baidu_key: &'a Option<String>,
        youdao_key: &'a Option<String>,
        youdao_secret: &'a Option<String>,
        bing_key: &'a Option<String>,
        bing_region: &'a Option<String>,
    ) -> Self {
        let text = |value: &'a Option<String>| value.as_deref().unwrap_or("");
        ProviderCreds {
            baidu_appid: text(baidu_appid),
            baidu_key: text(baidu_key),
            youdao_key: text(youdao_key),
            youdao_secret: text(youdao_secret),
            bing_key: text(bing_key),
            bing_region: text(bing_region),
        }
    }
}

/// 缺凭据时返回 (服务商名, 缺的项)，两处调用各自拼提示文案。
/// 百度那一项带前导空格，两句文案里中文与 "APP ID" 之间都有这个空格
fn missing_creds(provider: &str, c: &ProviderCreds) -> Option<(&'static str, &'static str)> {
    match provider {
        "baidu" if c.baidu_appid.is_empty() || c.baidu_key.is_empty() => {
            Some(("百度翻译", " APP ID 和密钥"))
        }
        "youdao" if c.youdao_key.is_empty() || c.youdao_secret.is_empty() => {
            Some(("有道翻译", "应用 ID 和应用密钥"))
        }
        "bing" if c.bing_key.is_empty() => Some(("必应翻译", "订阅密钥")),
        _ => None,
    }
}

async fn translate_via(
    client: &reqwest::Client,
    provider: &str,
    c: &ProviderCreds<'_>,
    texts: &[String],
    target_lang: &str,
) -> Result<Vec<String>, String> {
    match provider {
        "baidu" => translate_baidu(client, texts, target_lang, c.baidu_appid, c.baidu_key).await,
        "youdao" => {
            translate_youdao(client, texts, target_lang, c.youdao_key, c.youdao_secret).await
        }
        "bing" => translate_bing(client, texts, target_lang, c.bing_key, c.bing_region).await,
        _ => translate_google(client, texts, target_lang).await,
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
    translate_mode: Option<String>,
) -> Result<TranslateResult, String> {
    if tags.is_empty() {
        return Ok(TranslateResult {
            translations: vec![],
        });
    }

    // "text" = 自然语言整段翻译，"tags" = Danbooru 标签批量翻译（默认）
    let is_text_mode = translate_mode.as_deref() == Some("text");

    let conn = if is_text_mode { None } else { Some(open_db()?) };

    // 1. 查缓存（text 模式跳过）
    let mut cached: HashMap<String, String> = match &conn {
        Some(db) => lookup_cached(db, tags.iter().map(String::as_str), &target_lang)
            .map_err(|e| format!("查询缓存失败: {}", e))?,
        None => HashMap::new(),
    };
    let uncached: Vec<String> = tags
        .iter()
        .filter(|tag| !cached.contains_key(*tag))
        .cloned()
        .collect();

    let cached_count = cached.len();
    let total_count = tags.len();
    let mut translated_count = 0;
    // 三个标签编辑器常驻、共用这一个事件，进度带上本次调用的运行 ID
    let run_id = super::begin_run(TRANSLATE_PROGRESS_EVENT);
    let emit_progress = |current: usize| {
        let _ = app.emit(
            TRANSLATE_PROGRESS_EVENT,
            serde_json::json!({
                "current": current,
                "total": total_count,
                "run_id": run_id,
            }),
        );
    };

    // 发送初始进度（已缓存的部分）
    emit_progress(cached_count);

    // 2. 翻译未缓存的
    if !uncached.is_empty() {
        // text 模式：保留原文不做处理；tags 模式：下划线替换为空格
        let prepared: Cow<[String]> = if is_text_mode {
            Cow::Borrowed(uncached.as_slice())
        } else {
            Cow::Owned(uncached.iter().map(|t| t.replace('_', " ")).collect())
        };

        // text 模式用更长超时（长文本翻译可能较慢）
        let timeout_secs = if is_text_mode { 30 } else { 15 };
        let client = super::http_download::api_client(Duration::from_secs(timeout_secs))?;

        let creds = ProviderCreds::new(
            &baidu_appid,
            &baidu_key,
            &youdao_app_key,
            &youdao_app_secret,
            &bing_key,
            &bing_region,
        );
        if let Some((name, what)) = missing_creds(&provider, &creds) {
            return Err(format!(
                "{}需要配置{}\n请在「设置 → 翻译设置」中填写",
                name, what
            ));
        }

        // text 模式逐条独立翻译、不拆分结果；tags 模式按服务商分批
        let chunk_size = if is_text_mode {
            1
        } else {
            match provider.as_str() {
                "baidu" | "youdao" => 20,
                "bing" => 25,
                _ => 50,
            }
        };

        for (i, (chunk, originals)) in prepared
            .chunks(chunk_size)
            .zip(uncached.chunks(chunk_size))
            .enumerate()
        {
            if i > 0 && matches!(provider.as_str(), "baidu" | "youdao") {
                tokio::time::sleep(Duration::from_millis(1100)).await;
            }
            let lines = translate_via(&client, &provider, &creds, chunk, &target_lang).await?;

            if is_text_mode {
                // 百度/有道/Google 的结果按换行拆过，拼回整段；必应每条输入只回一项
                let joiner = if provider == "bing" { "" } else { "\n" };
                cached.insert(originals[0].clone(), lines.join(joiner).trim().to_string());
                translated_count += 1;
            } else {
                // 返回行数与请求行数不一致说明结果已错位，整批放弃，避免错误翻译写入缓存
                if lines.len() != originals.len() {
                    return Err(format!(
                        "翻译返回行数 ({}) 与请求行数 ({}) 不一致，已中止本次翻译以避免缓存错误结果，请重试",
                        lines.len(),
                        originals.len()
                    ));
                }
                let fresh: Vec<(String, String)> = originals
                    .iter()
                    .zip(&lines)
                    .map(|(original, tr)| {
                        let tr = tr.trim();
                        let final_tr =
                            if tr.to_lowercase() == original.replace('_', " ").to_lowercase() {
                                String::new()
                            } else {
                                tr.to_string()
                            };
                        (original.clone(), final_tr)
                    })
                    .collect();
                if let Some(db) = &conn {
                    let _ = upsert_translations(db, &fresh, &target_lang);
                }
                translated_count += fresh.len();
                cached.extend(fresh);
            }

            emit_progress(cached_count + translated_count);
        }
    }

    // 3. 按原始顺序返回
    let translations: Vec<TranslatedItem> = tags
        .iter()
        .map(|tag| TranslatedItem {
            source: tag.clone(),
            translated: cached.get(tag).cloned().unwrap_or_default(),
        })
        .collect();

    Ok(TranslateResult { translations })
}

/// 获取翻译缓存统计
#[tauri::command]
pub fn get_translation_cache_stats() -> Result<CacheStats, String> {
    let conn = open_db()?;
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM translations", [], |row| row.get(0))
        .map_err(|e| format!("查询缓存统计失败: {}", e))?;

    let db_size_bytes = std::fs::metadata(get_db_path())
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(CacheStats {
        total,
        db_size_bytes,
        zh_cn: count_for_lang(&conn, "zh-CN"),
        ja: count_for_lang(&conn, "ja"),
        ko: count_for_lang(&conn, "ko"),
    })
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
    let client = super::http_download::api_client(Duration::from_secs(10))?;

    let creds = ProviderCreds::new(
        &baidu_appid,
        &baidu_key,
        &youdao_app_key,
        &youdao_app_secret,
        &bing_key,
        &bing_region,
    );
    if let Some((name, what)) = missing_creds(&provider, &creds) {
        return Err(format!("请先填写{}{}", name, what));
    }

    let results =
        translate_via(&client, &provider, &creds, &["hello".to_string()], "zh-CN").await?;
    let translated = results.first().cloned().unwrap_or_default();
    Ok(format!("hello → {}", translated))
}

/// 导出翻译缓存为 CSV 文件
/// 格式: tag,translated,lang
#[tauri::command]
pub fn export_translation_csv(path: String) -> Result<u32, String> {
    let conn = open_db()?;
    let mut stmt = conn
        .prepare("SELECT tag, translated, lang FROM translations ORDER BY lang, tag")
        .map_err(|e| format!("查询失败: {}", e))?;

    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| format!("查询失败: {}", e))?
        .filter_map(|r| r.ok())
        .collect();

    std::fs::write(&path, translations_to_csv(&rows)?)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(rows.len() as u32)
}

/// UTF-8 BOM + `tag,translated,lang` 表头 + 各行
fn translations_to_csv(rows: &[(String, String, String)]) -> Result<Vec<u8>, String> {
    let write_err = |e: csv::Error| format!("写入文件失败: {}", e);
    let mut wtr = csv::Writer::from_writer("\u{FEFF}".as_bytes().to_vec());
    wtr.write_record(["tag", "translated", "lang"])
        .map_err(write_err)?;
    for (tag, translated, lang) in rows {
        wtr.write_record([tag, translated, lang])
            .map_err(write_err)?;
    }
    wtr.into_inner().map_err(|e| format!("写入文件失败: {}", e))
}

/// 导入翻译缓存 CSV 文件
/// 格式要求: 第一行必须是 tag,translated,lang
#[tauri::command]
pub fn import_translation_csv(path: String) -> Result<(u32, u32, String), String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    let parsed = parse_translation_csv(&content)?;

    let conn = open_db()?;
    conn.execute_batch("BEGIN")
        .map_err(|e| format!("开始事务失败: {}", e))?;
    let mut stmt = conn
        .prepare(UPSERT_TRANSLATION_SQL)
        .map_err(|e| format!("准备语句失败: {}", e))?;

    let mut imported = 0u32;
    let mut skipped = parsed.skipped;
    for (tag, translated, lang) in &parsed.rows {
        if stmt
            .execute(rusqlite::params![tag, translated, lang])
            .is_ok()
        {
            imported += 1;
        } else {
            skipped += 1;
        }
    }

    drop(stmt);
    conn.execute_batch("COMMIT")
        .map_err(|e| format!("提交事务失败: {}", e))?;
    Ok((imported, skipped, parsed.errors.join("\n")))
}

/// 导入 CSV 的解析结果
#[derive(Debug, Default)]
struct ParsedTranslationCsv {
    /// 待写入的 (tag, translated, lang)，已 trim
    rows: Vec<(String, String, String)>,
    skipped: u32,
    /// 最多 5 条带行号的错误
    errors: Vec<String>,
}

impl ParsedTranslationCsv {
    /// 跳过一行；带原因时记进 errors
    fn skip(&mut self, error: Option<String>) {
        self.skipped += 1;
        if let Some(error) = error.filter(|_| self.errors.len() < 5) {
            self.errors.push(error);
        }
    }
}

/// 校验表头并逐行检查列数与语言；不触碰数据库。
///
/// 一条记录只占一行：标签和译文都不含换行（翻译结果按行拆分后才入库，导出也就没有跨行字段）。
/// 引号个数为奇数的行引号不成对，整行跳过——按 CSV 规则读的话，没闭合的引号会把后面的行
/// 都吞进同一个字段
fn parse_translation_csv(content: &str) -> Result<ParsedTranslationCsv, String> {
    const VALID_LANGS: [&str; 3] = ["zh-CN", "ja", "ko"];
    // 应用自身导出带 UTF-8 BOM，比较表头前先剥掉，保证导出文件可直接再导入
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let content = content.replace("\r\n", "\n").replace('\r', "\n");
    // 空白行跳过、不计数，但行号按物理行算
    let mut lines = content
        .split('\n')
        .zip(1u64..)
        .filter(|(line, _)| !line.trim().is_empty());

    let Some((header, _)) = lines.next() else {
        return Err("CSV 文件为空".to_string());
    };
    let header = csv_fields(&normalize_csv_quote_spacing(header)).join(",");
    if header.to_lowercase().replace(' ', "") != "tag,translated,lang" {
        return Err(format!(
            "CSV 格式不正确。\n预期表头: tag,translated,lang\n实际表头: {}\n\n请确保 CSV 文件包含三列: tag（原始标签）、translated（翻译结果）、lang（语言代码，如 zh-CN、ja、ko）",
            header
        ));
    }

    let mut out = ParsedTranslationCsv::default();
    for (line, number) in lines {
        if line.matches('"').count() % 2 == 1 {
            out.skip(Some(format!("第 {} 行: 引号不成对，已跳过该行", number)));
            continue;
        }
        let fields = csv_fields(&normalize_csv_quote_spacing(line));
        if fields.len() < 3 {
            out.skip(Some(format!(
                "第 {} 行: 列数不足 ({}列，需要3列)",
                number,
                fields.len()
            )));
            continue;
        }

        let tag = fields[0].trim();
        let translated = fields[1].trim();
        let lang = fields[2].trim();

        if tag.is_empty() || translated.is_empty() {
            out.skip(None);
            continue;
        }

        if !VALID_LANGS.contains(&lang) {
            out.skip(Some(format!(
                "第 {} 行: 不支持的语言 '{}'（支持: zh-CN, ja, ko）",
                number, lang
            )));
            continue;
        }

        out.rows
            .push((tag.to_string(), translated.to_string(), lang.to_string()));
    }
    Ok(out)
}

/// 按 CSV 规则拆一行：引号内的逗号、转义的双引号
fn csv_fields(line: &str) -> Vec<String> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(line.as_bytes())
        .records()
        .next()
        .and_then(Result::ok)
        .map(|record| record.iter().map(str::to_string).collect())
        .unwrap_or_default()
}

/// 兼容手写 CSV 的字段前空格；只移除字段开头、引号外的空格，不改引号内的字节。
fn normalize_csv_quote_spacing(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let (mut index, mut field_start, mut quoted) = (0, true, false);
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            result.push(byte);
            if byte == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    result.push(b'"');
                    index += 1;
                } else {
                    quoted = false;
                }
            }
        } else if field_start && matches!(byte, b' ' | b'\t') {
            let start = index;
            while bytes.get(index).is_some_and(|b| matches!(b, b' ' | b'\t')) {
                index += 1;
            }
            if bytes.get(index) != Some(&b'"') {
                result.extend_from_slice(&bytes[start..index]);
                field_start = false;
            }
            continue;
        } else {
            result.push(byte);
            if byte == b'"' && field_start {
                quoted = true;
            }
            field_start = matches!(byte, b',' | b'\r' | b'\n');
        }
        index += 1;
    }
    String::from_utf8(result).expect("only ASCII whitespace was removed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_directory_is_loaded_once_and_survives_a_fresh_cache() {
        let dir = crate::commands::test_support::TempDir::new("cache_config");
        let config_path = dir.join("translation_cache.json");
        let custom = dir.join("custom");
        let config = CachePathConfig {
            directory: Some(custom.clone()),
        };
        super::super::config_paths::write_file_atomic(
            &config_path,
            &serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let load = || {
            let config: CachePathConfig =
                serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
            config.directory.unwrap_or_else(|| dir.join("default"))
        };
        let cache = Mutex::new(None);
        assert_eq!(cached_db_path(&cache, load), custom.join(DB_FILE_NAME));
        assert_eq!(
            cached_db_path(&cache, || panic!("path must be cached")),
            custom.join(DB_FILE_NAME)
        );
        assert_eq!(
            cached_db_path(&Mutex::new(None), load),
            custom.join(DB_FILE_NAME)
        );

        std::fs::write(
            &config_path,
            serde_json::to_vec(&CachePathConfig::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            cached_db_path(&Mutex::new(None), load),
            dir.join("default").join(DB_FILE_NAME)
        );
    }

    #[test]
    fn import_csv_accepts_spaces_before_quotes_without_changing_quoted_text() {
        let content = concat!(
            "tag,translated,lang\r\n",
            "smile, \"微笑, 笑\", zh-CN\r\n",
            "\"a, \"\"inside\"\"\", \t\"line1, \"\" quote  line2\", ja\r\n",
            "short\r\n",
        );
        let parsed = parse_translation_csv(content).unwrap();
        assert_eq!(
            parsed.rows,
            rows(&[
                ("smile", "微笑, 笑", "zh-CN"),
                ("a, \"inside\"", "line1, \" quote  line2", "ja"),
            ])
        );
        assert_eq!(parsed.errors, ["第 4 行: 列数不足 (1列，需要3列)"]);
        assert_eq!(
            normalize_csv_quote_spacing("\"a,  \"\"b\"\"\""),
            "\"a,  \"\"b\"\"\""
        );
    }

    fn rows(items: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        items
            .iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
            .collect()
    }

    #[test]
    fn provider_lang_codes_cover_app_languages() {
        assert_eq!(baidu_lang_code("zh-CN"), "zh");
        assert_eq!(baidu_lang_code("ja"), "jp");
        assert_eq!(baidu_lang_code("ko"), "kor");
        assert_eq!(baidu_lang_code("unknown"), "zh");

        assert_eq!(youdao_lang_code("zh-CN"), "zh-CHS");
        assert_eq!(youdao_lang_code("ja"), "ja");
        assert_eq!(youdao_lang_code("ko"), "ko");

        assert_eq!(bing_lang_code("zh-CN"), "zh-Hans");
        assert_eq!(bing_lang_code("ja"), "ja");
        assert_eq!(bing_lang_code("ko"), "ko");
    }

    #[test]
    fn baidu_sign_is_lowercase_md5_hex() {
        assert_eq!(baidu_sign(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(baidu_sign("hello"), "5d41402abc4b2a76b9719d911017c592");
    }

    #[test]
    fn provider_creds_take_command_arguments_in_order() {
        let (appid, key, ykey, ysecret, bkey, region) = (
            Some("id".to_string()),
            None,
            Some("yk".to_string()),
            Some("ys".to_string()),
            None,
            Some("eastasia".to_string()),
        );
        let creds = ProviderCreds::new(&appid, &key, &ykey, &ysecret, &bkey, &region);
        assert_eq!(
            [
                creds.baidu_appid,
                creds.baidu_key,
                creds.youdao_key,
                creds.youdao_secret,
                creds.bing_key,
                creds.bing_region
            ],
            ["id", "", "yk", "ys", "", "eastasia"]
        );
        assert_eq!(
            missing_creds("baidu", &creds),
            Some(("百度翻译", " APP ID 和密钥"))
        );
        assert_eq!(missing_creds("youdao", &creds), None);
    }

    #[test]
    fn missing_creds_reports_provider_and_item() {
        let empty = ProviderCreds {
            baidu_appid: "",
            baidu_key: "",
            youdao_key: "",
            youdao_secret: "",
            bing_key: "",
            bing_region: "",
        };
        assert_eq!(
            missing_creds("baidu", &empty),
            Some(("百度翻译", " APP ID 和密钥"))
        );
        assert_eq!(
            missing_creds("youdao", &empty),
            Some(("有道翻译", "应用 ID 和应用密钥"))
        );
        assert_eq!(
            missing_creds("bing", &empty),
            Some(("必应翻译", "订阅密钥"))
        );
        assert_eq!(missing_creds("google", &empty), None);

        // 只填一半同样算缺；必应的区域可以不填
        let half = ProviderCreds {
            baidu_appid: "id",
            youdao_secret: "secret",
            bing_key: "key",
            ..empty
        };
        assert!(missing_creds("baidu", &half).is_some());
        assert!(missing_creds("youdao", &half).is_some());
        assert_eq!(missing_creds("bing", &half), None);
    }

    /// 带 BOM 的表头能识别；引号内逗号、转义引号按 CSV 规则解析
    #[test]
    fn import_csv_parses_bom_and_quoted_fields() {
        let content = "\u{FEFF}tag,translated,lang\n\
                       \"long_hair,braid\",\"他说\"\"好\"\"\",zh-CN\n\
                       smile , 微笑 ,ko\n";
        let parsed = parse_translation_csv(content).unwrap();
        assert_eq!(
            parsed.rows,
            rows(&[
                ("long_hair,braid", "他说\"好\"", "zh-CN"),
                ("smile", "微笑", "ko"),
            ])
        );
        assert_eq!(parsed.skipped, 0);
        assert!(parsed.errors.is_empty());
    }

    /// 列数不足、不支持的语言计入跳过并带行号报错，最多 5 条；
    /// 空字段只计跳过不报错，空白行不计数。行号按物理行算，空白行和 CRLF 不会让它错位
    #[test]
    fn import_csv_reports_line_numbers_and_caps_errors() {
        let content = "tag,translated,lang\r\n\
                       \"a,b\",x,zh-CN\r\n\
                       \r\n\
                       \r\n\
                       only,two\r\n   \r\n\
                       t,x,fr\r\n\
                       ,,\r\n\
                       c,d,ja\r\n\
                       bad1\r\nbad2\r\nbad3\r\nbad4\r\n";
        let parsed = parse_translation_csv(content).unwrap();
        assert_eq!(
            parsed.rows,
            rows(&[("a,b", "x", "zh-CN"), ("c", "d", "ja")])
        );
        assert_eq!(parsed.skipped, 7);
        assert_eq!(
            parsed.errors,
            vec![
                "第 5 行: 列数不足 (2列，需要3列)",
                "第 7 行: 不支持的语言 'fr'（支持: zh-CN, ja, ko）",
                "第 10 行: 列数不足 (1列，需要3列)",
                "第 11 行: 列数不足 (1列，需要3列)",
                "第 12 行: 列数不足 (1列，需要3列)",
            ]
        );
    }

    #[test]
    fn import_csv_line_numbers_count_blank_lines() {
        let parsed =
            parse_translation_csv("tag,translated,lang\n\n\"x,y\",z,ja\n\nshort\n").unwrap();
        assert_eq!(parsed.rows, rows(&[("x,y", "z", "ja")]));
        assert_eq!(parsed.errors, vec!["第 5 行: 列数不足 (1列，需要3列)"]);
    }

    /// 引号不成对的行整行跳过，后面的合法行照常导入——即使后面的行里又有成对的引号，
    /// 也不会和坏行拼成一条记录
    #[test]
    fn import_csv_skips_unclosed_quote_line_and_keeps_later_rows() {
        let content = concat!(
            "tag,translated,lang\n",
            "a,\"broken,ja\n",
            "b,ok,zh-CN\n",
            "c,\"x\",ko\n",
            "d,\"quoted, text\",ja\n",
            "\"e\n",
            "f\",split,ko\n",
            "stray\"quote,x,ja\n",
            "g,ok3,ja\n",
        );
        let parsed = parse_translation_csv(content).unwrap();
        assert_eq!(
            parsed.rows,
            rows(&[
                ("b", "ok", "zh-CN"),
                ("c", "x", "ko"),
                ("d", "quoted, text", "ja"),
                ("g", "ok3", "ja"),
            ])
        );
        assert_eq!(parsed.skipped, 4);
        assert_eq!(
            parsed.errors,
            [
                "第 2 行: 引号不成对，已跳过该行",
                "第 6 行: 引号不成对，已跳过该行",
                "第 7 行: 引号不成对，已跳过该行",
                "第 8 行: 引号不成对，已跳过该行",
            ]
        );
    }

    /// 引号一直到文件末尾都没闭合、或连着几行都有问题时，逐行恢复且行号准确
    #[test]
    fn import_csv_recovers_from_repeated_unclosed_quotes() {
        let content = concat!(
            "tag,translated,lang\r\n",
            "x,\"one\r\n",
            "\r\n",
            "y,\"two\r\n",
            "z,ok,ja\r\n",
            "w,\"three\r\n",
        );
        let parsed = parse_translation_csv(content).unwrap();
        assert_eq!(parsed.rows, rows(&[("z", "ok", "ja")]));
        assert_eq!(parsed.skipped, 3);
        assert_eq!(
            parsed.errors,
            [
                "第 2 行: 引号不成对，已跳过该行",
                "第 4 行: 引号不成对，已跳过该行",
                "第 6 行: 引号不成对，已跳过该行",
            ]
        );

        let parsed = parse_translation_csv("tag,translated,lang\nok,fine,ko\nx,\"y").unwrap();
        assert_eq!(parsed.rows, rows(&[("ok", "fine", "ko")]));
        assert_eq!(parsed.errors, ["第 3 行: 引号不成对，已跳过该行"]);

        // 只用 \r 换行的文件同样按物理行恢复
        let parsed =
            parse_translation_csv("tag,translated,lang\ra,\"broken,ja\rb,ok,zh-CN\rshort\r")
                .unwrap();
        assert_eq!(parsed.rows, rows(&[("b", "ok", "zh-CN")]));
        assert_eq!(
            parsed.errors,
            [
                "第 2 行: 引号不成对，已跳过该行",
                "第 4 行: 列数不足 (1列，需要3列)",
            ]
        );
    }

    #[test]
    fn import_csv_rejects_empty_or_wrong_header() {
        assert_eq!(parse_translation_csv("").unwrap_err(), "CSV 文件为空");
        assert_eq!(
            parse_translation_csv("\u{FEFF}").unwrap_err(),
            "CSV 文件为空"
        );
        let err = parse_translation_csv("source,target\na,b\n").unwrap_err();
        assert!(err.contains("实际表头: source,target"), "{err}");
        // 表头比较不区分大小写、忽略空格
        assert!(parse_translation_csv("Tag, Translated, Lang\n").is_ok());
    }

    /// 导出带 BOM；含逗号、引号的字段导出后能原样导回
    #[test]
    fn exported_csv_imports_back_unchanged() {
        let data = rows(&[
            ("long_hair", "长发", "zh-CN"),
            ("a,b", "x\"y\"", "ja"),
            ("\"quoted\"", "q", "ko"),
        ]);
        let text = String::from_utf8(translations_to_csv(&data).unwrap()).unwrap();
        assert!(text.starts_with("\u{FEFF}tag,translated,lang\n"));
        let parsed = parse_translation_csv(&text).unwrap();
        assert_eq!(parsed.rows, data);
        assert_eq!(parsed.skipped, 0);
    }
}
