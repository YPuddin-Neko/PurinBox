use serde::{Deserialize, Serialize};

use super::config_paths::{b64_decode, b64_encode, load_json_config_or_default, save_json_config};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HuggingFaceConfig {
    #[serde(default)]
    pub token_encoded: String,
}

const CONFIG_FILE: &str = "huggingface_config.json";

#[tauri::command]
pub fn save_huggingface_config(token: String) -> Result<(), String> {
    let config = HuggingFaceConfig {
        token_encoded: b64_encode(token.trim()),
    };
    save_json_config(CONFIG_FILE, &config, "Hugging Face 配置")
}

#[tauri::command]
pub fn load_huggingface_config() -> Result<String, String> {
    Ok(load_huggingface_token_internal())
}

pub fn load_huggingface_token_internal() -> String {
    b64_decode(&load_json_config_or_default::<HuggingFaceConfig>(CONFIG_FILE).token_encoded)
}

pub fn apply_huggingface_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let token = load_huggingface_token_internal();
    let token = token.trim();
    if token.is_empty() {
        request
    } else {
        request.bearer_auth(token)
    }
}
