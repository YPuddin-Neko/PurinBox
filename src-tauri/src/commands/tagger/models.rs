use serde::{Deserialize, Serialize};
use super::get_models_dir;

/// 模型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repo_id: String,
    pub model_filename: String,
    pub tags_filename: String,
    pub input_size: u32,
    pub is_builtin: bool,
}

/// 获取内置模型列表
pub fn get_builtin_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "wd-swinv2-tagger-v3".into(),
            name: "WD SwinV2 Tagger v3".into(),
            description: "基于 SwinV2 的高精度打标模型，推荐使用".into(),
            repo_id: "SmilingWolf/wd-swinv2-tagger-v3".into(),
            model_filename: "model.onnx".into(),
            tags_filename: "selected_tags.csv".into(),
            input_size: 448,
            is_builtin: true,
        },
        ModelDefinition {
            id: "wd-vit-tagger-v3".into(),
            name: "WD ViT Tagger v3".into(),
            description: "基于 Vision Transformer 的打标模型".into(),
            repo_id: "SmilingWolf/wd-vit-tagger-v3".into(),
            model_filename: "model.onnx".into(),
            tags_filename: "selected_tags.csv".into(),
            input_size: 448,
            is_builtin: true,
        },
        ModelDefinition {
            id: "wd-convnext-tagger-v3".into(),
            name: "WD ConvNeXt Tagger v3".into(),
            description: "基于 ConvNeXt 架构的打标模型，速度较快".into(),
            repo_id: "SmilingWolf/wd-convnext-tagger-v3".into(),
            model_filename: "model.onnx".into(),
            tags_filename: "selected_tags.csv".into(),
            input_size: 448,
            is_builtin: true,
        },
        ModelDefinition {
            id: "wd-eva02-large-tagger-v3".into(),
            name: "WD EVA02 Large Tagger v3".into(),
            description: "基于 EVA02 Large 的高精度模型，体积较大".into(),
            repo_id: "SmilingWolf/wd-eva02-large-tagger-v3".into(),
            model_filename: "model.onnx".into(),
            tags_filename: "selected_tags.csv".into(),
            input_size: 448,
            is_builtin: true,
        },
        ModelDefinition {
            id: "wd-v1-4-moat-tagger-v2".into(),
            name: "WD MOAT Tagger v2".into(),
            description: "基于 MOAT 架构的 v2 打标模型".into(),
            repo_id: "SmilingWolf/wd-v1-4-moat-tagger-v2".into(),
            model_filename: "model.onnx".into(),
            tags_filename: "selected_tags.csv".into(),
            input_size: 448,
            is_builtin: true,
        },
        ModelDefinition {
            id: "cl-tagger-1-02".into(),
            name: "CL Tagger v1.02".into(),
            description: "CL (CLIP-Like) 打标模型 v1.02".into(),
            repo_id: "p1atdev/CL-Tagger-1.02".into(),
            model_filename: "model.onnx".into(),
            tags_filename: "selected_tags.csv".into(),
            input_size: 448,
            is_builtin: true,
        },
    ]
}

/// 自定义模型配置文件路径
fn custom_models_path() -> std::path::PathBuf {
    get_models_dir().join("custom_models.json")
}

/// 加载自定义模型列表
pub fn load_custom_models() -> Result<Vec<ModelDefinition>, String> {
    let path = custom_models_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取自定义模型配置失败: {}", e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("解析自定义模型配置失败: {}", e))
}

/// 添加自定义模型
pub fn add_custom_model(id: String, name: String, repo_id: String, input_size: u32) -> Result<(), String> {
    let mut models = load_custom_models().unwrap_or_default();

    if models.iter().any(|m| m.id == id) {
        return Err(format!("模型 ID '{}' 已存在", id));
    }

    models.push(ModelDefinition {
        id,
        name,
        description: "用户自定义模型".into(),
        repo_id,
        model_filename: "model.onnx".into(),
        tags_filename: "selected_tags.csv".into(),
        input_size,
        is_builtin: false,
    });

    let dir = custom_models_path().parent().unwrap().to_path_buf();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    let json = serde_json::to_string_pretty(&models)
        .map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(custom_models_path(), json)
        .map_err(|e| format!("写入配置失败: {}", e))?;

    Ok(())
}

/// 根据 ID 查找模型（含内置和自定义）
pub fn find_model(id: &str) -> Option<ModelDefinition> {
    let all = get_builtin_models();
    if let Some(m) = all.into_iter().find(|m| m.id == id) {
        return Some(m);
    }
    let custom = load_custom_models().unwrap_or_default();
    custom.into_iter().find(|m| m.id == id)
}
