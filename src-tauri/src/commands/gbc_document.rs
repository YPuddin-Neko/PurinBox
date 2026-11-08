use std::path::Path;
use serde::{Serialize, Deserialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
pub struct GbcDocument { pub path: String, pub image_path: String, pub content: String }

fn validate(value: &Value) -> Result<(), String> {
    let graph = value.get("graph").unwrap_or(value);
    let vertices = graph["vertices"].as_array().filter(|v| !v.is_empty()).ok_or("GBC 数据缺少区域节点")?;
    let mut ids = std::collections::HashSet::new();
    for vertex in vertices {
        let id = vertex["vertex_id"].as_str().ok_or("节点缺少 ID")?;
        if !ids.insert(id) { return Err("节点 ID 重复".into()); }
        if !vertex["descs"].as_array().is_some_and(|items| items.iter().all(|d| d["text"].is_string() && d["label"].is_string())) { return Err("节点描述格式无效".into()); }
        let b = &vertex["bbox"];
        let coords: Vec<f64> = ["left", "top", "right", "bottom"].iter().map(|k| b[k].as_f64().unwrap_or(f64::NAN)).collect();
        if coords.iter().any(|n| !n.is_finite() || !(0.0..=1.0).contains(n)) || coords[0] >= coords[2] || coords[1] >= coords[3] { return Err("区域坐标必须在 0 到 1 之间，且左上角小于右下角".into()); }
    }
    for vertex in vertices {
        for key in ["in_edges", "out_edges"] {
            for edge in vertex[key].as_array().ok_or("节点缺少关系列表")? {
                if !ids.contains(edge["source"].as_str().unwrap_or("\0")) || !ids.contains(edge["target"].as_str().unwrap_or("\0")) { return Err("关系指向不存在的节点".into()); }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn load_gbc_dataset(folder: String, recursive: bool) -> Result<Vec<GbcDocument>, String> {
    super::collect_image_files_with_recursive(Path::new(&folder), recursive)?
        .into_iter().filter(|image| image.with_extension("gbc.json").is_file())
        .map(|image| load_gbc_document(image.to_string_lossy().into()))
        .collect()
}

#[tauri::command]
pub fn load_gbc_document(path: String) -> Result<GbcDocument, String> {
    let selected = Path::new(&path);
    let graph_path = if path.to_lowercase().ends_with(".gbc.json") { selected.to_path_buf() } else { selected.with_extension("gbc.json") };
    let content = std::fs::read_to_string(&graph_path).map_err(|e| format!("无法读取 GBC 数据：{}", e))?;
    validate(&serde_json::from_str::<Value>(&content).map_err(|e| format!("GBC JSON 无效：{}", e))?)?;
    let images = super::collect_image_files_with_recursive(graph_path.parent().ok_or("GBC 路径无效")?, false)?;
    let matches: Vec<_> = images.into_iter().filter(|p| p.with_extension("gbc.json") == graph_path).collect();
    if matches.len() != 1 { return Err("请将 GBC 数据和唯一的同名图片放在同一目录".into()); }
    Ok(GbcDocument { path: graph_path.to_string_lossy().into(), image_path: matches[0].to_string_lossy().into(), content })
}

#[tauri::command]
pub fn save_gbc_document(path: String, content: String, original: String) -> Result<(), String> {
    if !path.to_lowercase().ends_with(".gbc.json") { return Err("请选择 .gbc.json 文件".into()); }
    validate(&serde_json::from_str::<Value>(&content).map_err(|e| format!("GBC JSON 无效：{}", e))?)?;
    if std::fs::read_to_string(&path).map_err(|e| e.to_string())? != original { return Err("文件已被其他操作修改，请重新打开".into()); }
    super::config_paths::write_file_atomic(Path::new(&path), content.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::TempDir;
    use serde_json::json;
    #[test]
    fn document_edit_preserves_fields_and_rejects_invalid_or_stale_writes() {
        let root = TempDir::new("gbc_document");
        let image = root.join("sample.png");
        image::RgbImage::new(16, 16).save(&image).unwrap();
        let path = image.with_extension("gbc.json");
        let mut graph = json!({"img_size":[16,16],"custom":"keep", "vertices":[{"vertex_id":"", "label":"image", "bbox":{"left":0,"top":0,"right":1,"bottom":1},"descs":[{"label":"detail","text":"original"}],"in_edges":[],"out_edges":[]}]});
        std::fs::write(&path, graph.to_string()).unwrap();
        image::RgbImage::new(16, 16).save(root.join("without_graph.png")).unwrap();
        let docs = load_gbc_dataset(image.parent().unwrap().to_string_lossy().into(), false).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].image_path, image.to_string_lossy());
        let doc = load_gbc_document(image.to_string_lossy().into()).unwrap();
        assert_eq!(doc.image_path, image.to_string_lossy());
        graph["vertices"][0]["descs"][0]["text"] = "edited".into();
        save_gbc_document(doc.path.clone(), graph.to_string(), doc.content.clone()).unwrap();
        assert!(save_gbc_document(doc.path.clone(), graph.to_string(), doc.content).is_err());
        let current = std::fs::read_to_string(&path).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&current).unwrap()["custom"], "keep");
        graph["vertices"][0]["bbox"]["left"] = 2.into();
        assert!(save_gbc_document(doc.path, graph.to_string(), current.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), current);
    }
}
