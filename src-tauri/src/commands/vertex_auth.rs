use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Google Service Account JSON 密钥文件结构（只读取需要的字段）
#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: Option<String>,
}

/// 缓存的 access token
struct CachedToken {
    token: String,
    /// 过期时间（Unix 秒）
    expires_at: u64,
}

static TOKEN_CACHE: Mutex<Option<CachedToken>> = Mutex::new(None);

/// JWT Claims
#[derive(Serialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
}

/// Token 响应
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// 从 Service Account JSON 文件获取 access token
/// 带内存缓存，过期前 60 秒自动刷新
pub async fn get_vertex_access_token(sa_json_path: &str) -> Result<String, String> {
    // 检查缓存
    {
        let cache = TOKEN_CACHE.lock().map_err(|e| format!("锁获取失败: {}", e))?;
        if let Some(ref cached) = *cache {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            // 还有 60 秒以上才过期，直接返回缓存
            if cached.expires_at > now + 60 {
                return Ok(cached.token.clone());
            }
        }
    }

    // 读取 SA JSON 文件
    let sa_content = std::fs::read_to_string(sa_json_path)
        .map_err(|e| format!("读取服务账号文件失败: {}", e))?;
    let sa_key: ServiceAccountKey = serde_json::from_str(&sa_content)
        .map_err(|e| format!("解析服务账号 JSON 失败: {}", e))?;

    let token_uri = sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());

    // 构建 JWT
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = Claims {
        iss: sa_key.client_email.clone(),
        scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
        aud: token_uri.clone(),
        iat: now,
        exp: now + 3600, // 1 小时有效期
    };

    // RS256 签名
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(sa_key.private_key.as_bytes())
        .map_err(|e| format!("解析私钥失败: {}", e))?;

    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    let jwt = jsonwebtoken::encode(&header, &claims, &encoding_key)
        .map_err(|e| format!("JWT 签名失败: {}", e))?;

    // 用 JWT 换取 access token
    let client = crate::commands::proxy_config::build_http_client_for_llm()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let resp = client
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ])
        .send()
        .await
        .map_err(|e| format!("Token 请求失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token 请求错误 ({}): {}", status, body));
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Token 响应解析失败: {}", e))?;

    let expires_at = now + token_resp.expires_in.max(300); // 至少 5 分钟

    // 写入缓存
    {
        let mut cache = TOKEN_CACHE.lock().map_err(|e| format!("锁获取失败: {}", e))?;
        *cache = Some(CachedToken {
            token: token_resp.access_token.clone(),
            expires_at,
        });
    }

    Ok(token_resp.access_token)
}

/// 清除 token 缓存（切换账号时调用）
#[allow(dead_code)]
pub fn clear_token_cache() {
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        *cache = None;
    }
}
