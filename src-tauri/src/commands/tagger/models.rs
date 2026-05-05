use serde::{Deserialize, Serialize};
use super::get_models_dir;

/// 输入数据格式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[derive(Default)]
#[allow(clippy::upper_case_acronyms)]
pub enum InputFormat {
    /// [Batch, Height, Width, Channels] — TensorFlow 风格
    #[default]
    NHWC,
    /// [Batch, Channels, Height, Width] — PyTorch 风格
    NCHW,
}


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
    #[serde(default)]
    pub input_format: InputFormat,
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
            input_format: InputFormat::NHWC,
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
            input_format: InputFormat::NHWC,
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
            input_format: InputFormat::NHWC,
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
            input_format: InputFormat::NHWC,
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
            input_format: InputFormat::NHWC,
        },
        ModelDefinition {
            id: "cl-tagger-1-02".into(),
            name: "CL Tagger v1.02".into(),
            description: "CL (CLIP-Like) 打标模型 v1.02，高精度 CLIP 架构".into(),
            repo_id: "cella110n/cl_tagger".into(),
            model_filename: "cl_tagger_1_02/model.onnx".into(),
            tags_filename: "cl_tagger_1_02/tag_mapping.json".into(),
            input_size: 448,
            is_builtin: true,
            input_format: InputFormat::NHWC,
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

/// 保存自定义模型列表
fn save_custom_models(models: &[ModelDefinition]) -> Result<(), String> {
    let dir = custom_models_path().parent().unwrap().to_path_buf();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let json = serde_json::to_string_pretty(models)
        .map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(custom_models_path(), json)
        .map_err(|e| format!("写入配置失败: {}", e))?;
    Ok(())
}

/// 添加自定义模型（本地导入）
pub fn add_local_model(
    name: String,
    model_path: String,
    tags_path: String,
    input_size: u32,
    input_format: InputFormat,
) -> Result<String, String> {
    // 生成 ID
    let id = format!("custom-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    let mut models = load_custom_models().unwrap_or_default();
    if models.iter().any(|m| m.name == name) {
        return Err(format!("名称 '{}' 已存在", name));
    }

    // 将文件复制到模型目录
    let model_dir = super::get_model_dir(&id);
    if !model_dir.exists() {
        std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    // 复制 model.onnx
    let src_model = std::path::Path::new(&model_path);
    let dest_model = model_dir.join("model.onnx");
    std::fs::copy(src_model, &dest_model)
        .map_err(|e| format!("复制模型文件失败: {}", e))?;

    // 复制标签文件（保留原始扩展名）
    let src_tags = std::path::Path::new(&tags_path);
    let tags_ext = src_tags.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("csv");
    let tags_dest_name = format!("tags.{}", tags_ext);
    let dest_tags = model_dir.join(&tags_dest_name);
    std::fs::copy(src_tags, &dest_tags)
        .map_err(|e| format!("复制标签文件失败: {}", e))?;

    models.push(ModelDefinition {
        id: id.clone(),
        name,
        description: "本地导入的自定义模型".into(),
        repo_id: String::new(),
        model_filename: "model.onnx".into(),
        tags_filename: tags_dest_name,
        input_size,
        is_builtin: false,
        input_format,
    });

    save_custom_models(&models)?;
    Ok(id)
}

/// 删除自定义模型
pub fn remove_custom_model(id: &str) -> Result<(), String> {
    let mut models = load_custom_models().unwrap_or_default();
    let orig_len = models.len();
    models.retain(|m| m.id != id);
    if models.len() == orig_len {
        return Err("模型不存在".into());
    }
    save_custom_models(&models)?;

    // 删除模型文件目录
    let model_dir = super::get_model_dir(id);
    if model_dir.exists() {
        let _ = std::fs::remove_dir_all(&model_dir);
    }
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
