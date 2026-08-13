//! 工作流节点命令的运行时测试。
//!
//! 每个测试的参数 JSON 逐字复刻前端 WorkflowEngine.buildCommandOptions 的输出——
//! serde 反序列化成功即证明字段与前端兼容，随后真实执行命令并校验产物。
//! 覆盖工作流中所有纯 Rust 节点；AI 节点（tagger/llm/upscale/person-crop/aesthetic）
//! 依赖模型与 Python 环境，仅做参数形状校验（见 options_shape_* 测试）。

use serde_json::json;
use std::path::{Path, PathBuf};

use super::alpha_convert::convert_alpha;
use super::batch_rename::execute_rename;
use super::blur_noise::blur_noise_images;
use super::bucket_preview::analyze_buckets;
use super::format_convert::convert_format;
use super::image_crop::crop_images;
use super::image_flip::flip_images;
use super::image_scale::scale_images;
use super::perspective::perspective_transform;
use super::resolution_filter::filter_by_resolution;
use super::workflow::cleanup_workflow_temp;

fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_app()
}

/// 生成测试图集：5 张不同尺寸/格式（含 1 张 JPG 与 1 张带透明通道）
fn make_dataset(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("purinbox_wf_nodes_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let input = root.join("input");
    std::fs::create_dir_all(&input).unwrap();
    image::RgbImage::from_pixel(512, 512, image::Rgb([200, 60, 60]))
        .save(input.join("a_512.png"))
        .unwrap();
    image::RgbImage::from_pixel(640, 384, image::Rgb([60, 200, 60]))
        .save(input.join("b_640x384.png"))
        .unwrap();
    image::RgbImage::from_pixel(300, 200, image::Rgb([60, 60, 200]))
        .save(input.join("c_300x200.jpg"))
        .unwrap();
    image::RgbaImage::from_pixel(512, 512, image::Rgba([255, 0, 0, 128]))
        .save(input.join("d_alpha.png"))
        .unwrap();
    image::RgbImage::from_pixel(1024, 768, image::Rgb([200, 200, 60]))
        .save(input.join("e_1024x768.png"))
        .unwrap();
    (root, input)
}

fn file_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|it| it.flatten().filter(|e| e.path().is_file()).count())
        .unwrap_or(0)
}

fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn node_scale() {
    let app = mock_app();
    let (root, input) = make_dataset("scale");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "mode": "upscale",
        "target_width": 1024,
        "target_height": 1024,
        "down_target_width": 0,
        "down_target_height": 0,
        "recursive": false,
    }))
    .expect("scale 参数与前端不兼容");

    let r = scale_images(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "缩放不应有失败: {:?}", r.errors);
    assert_eq!(file_count(&out), 5, "输出应包含全部 5 张图");
    cleanup(&root);
}

#[tokio::test]
async fn node_crop() {
    let app = mock_app();
    let (root, input) = make_dataset("crop");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "mode": "center",
        "crop_anchor": "center",
        "target_width": 256,
        "target_height": 256,
        "aspect_ratio": 1.0,
        "crop_top": 0, "crop_bottom": 0, "crop_left": 0, "crop_right": 0,
        "recursive": false,
    }))
    .expect("crop 参数与前端不兼容");

    let r = crop_images(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "裁切不应有失败: {:?}", r.errors);
    assert_eq!(file_count(&out), 5);
    cleanup(&root);
}

#[tokio::test]
async fn node_flip() {
    let app = mock_app();
    let (root, input) = make_dataset("flip");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "direction": "horizontal",
        "recursive": false,
    }))
    .expect("flip 参数与前端不兼容");

    let r = flip_images(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "翻转不应有失败: {:?}", r.errors);
    assert_eq!(file_count(&out), 5);
    cleanup(&root);
}

#[tokio::test]
async fn node_format_convert() {
    let app = mock_app();
    let (root, input) = make_dataset("convert");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "target_format": "png",
        "recursive": false,
    }))
    .expect("format-convert 参数与前端不兼容");

    let r = convert_format(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "格式转换不应有失败: {:?}", r.errors);
    // 工作流语义关键：已是目标格式的文件也必须出现在输出目录，
    // 否则下游节点会拿到不完整的数据集
    assert_eq!(
        file_count(&out),
        5,
        "输出目录应包含全部 5 张图（跳过转换的文件也应复制到输出）"
    );
    cleanup(&root);
}

#[tokio::test]
async fn node_alpha_convert() {
    let app = mock_app();
    let (root, input) = make_dataset("alpha");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "background": "white",
        "recursive": false,
    }))
    .expect("alpha-convert 参数与前端不兼容");

    let r = convert_alpha(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "透明通道转换不应有失败: {:?}", r.errors);
    assert_eq!(file_count(&out), 5, "输出目录应包含全部 5 张图");

    // 带透明通道的图转换后不应再有半透明像素
    let converted = image::open(out.join("d_alpha.png")).unwrap().to_rgba8();
    assert!(
        converted.pixels().all(|p| p[3] == 255),
        "转换后不应存在透明像素"
    );
    cleanup(&root);
}

#[tokio::test]
async fn node_blur_noise() {
    let app = mock_app();
    let (root, input) = make_dataset("blur");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "blur_radius": 1.5,
        "noise_strength": 5,
        "recursive": false,
    }))
    .expect("blur-noise 参数与前端不兼容");

    let r = blur_noise_images(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "模糊/噪点不应有失败: {:?}", r.errors);
    assert_eq!(file_count(&out), 5);
    cleanup(&root);
}

#[tokio::test]
async fn node_perspective() {
    let app = mock_app();
    let (root, input) = make_dataset("perspective");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "intensity": 0.1,
        "recursive": false,
    }))
    .expect("perspective 参数与前端不兼容");

    let r = perspective_transform(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "透视变换不应有失败: {:?}", r.errors);
    assert_eq!(file_count(&out), 5);
    cleanup(&root);
}

#[tokio::test]
async fn node_filter() {
    let app = mock_app();
    let (root, input) = make_dataset("filter");
    let out = root.join("out");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "output_path": out.to_string_lossy(),
        "action": "copy",
        "condition": "below_resolution",
        "width": 512,
        "height": 512,
        "recursive": false,
    }))
    .expect("filter 参数与前端不兼容");

    let r = filter_by_resolution(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "分辨率筛选不应有失败: {:?}", r.errors);
    let hit = file_count(&out);
    assert!(hit >= 1 && hit < 5, "应筛出部分低分辨率图片，实际 {}", hit);
    assert_eq!(file_count(&input), 5, "copy 模式不应动原目录");
    cleanup(&root);
}

#[tokio::test]
async fn node_rename() {
    let app = mock_app();
    let (root, input) = make_dataset("rename");
    // 放一个同名标签文件，验证 rename_tags 联动
    std::fs::write(input.join("a_512.txt"), "1girl, solo").unwrap();

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "prefix": "img_",
        "start_number": 1,
        "digit_count": 4,
        "shuffle": false,
        "rename_tags": true,
    }))
    .expect("rename 参数与前端不兼容");

    let r = execute_rename(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.fail_count, 0, "重命名不应有失败: {:?}", r.errors);

    let names: Vec<String> = std::fs::read_dir(&input)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        names.iter().filter(|n| n.starts_with("img_")).count() >= 5,
        "图片应按 img_XXXX 重命名，实际: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "img_0001.txt"),
        "标签文件应联动重命名，实际: {:?}",
        names
    );
    cleanup(&root);
}

#[tokio::test]
async fn node_bucket_assign() {
    let app = mock_app();
    let (root, input) = make_dataset("bucket");

    let opts = serde_json::from_value(json!({
        "input_path": input.to_string_lossy(),
        "res_width": 1024,
        "res_height": 1024,
        "steps": 64,
        "no_upscale": false,
        "recursive": false,
    }))
    .expect("bucket-assign 参数与前端不兼容");

    let r = analyze_buckets(app.handle().clone(), opts).await.unwrap();
    assert_eq!(r.total_images, 5);
    assert!(r.bucket_count > 0, "应至少产生一个桶");
    let sum: u32 = r.buckets.iter().map(|b| b.image_count).sum();
    assert_eq!(sum, 5, "各桶图片数之和应等于总数");
    cleanup(&root);
}

#[tokio::test]
async fn node_cleanup_workflow_temp() {
    let root = std::env::temp_dir().join(format!("purinbox_wf_cleanup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let step_dir = root.join(".workflow_temp").join("step_1_scale");
    std::fs::create_dir_all(&step_dir).unwrap();
    std::fs::write(step_dir.join("residual.png"), vec![0u8; 2048]).unwrap();

    let freed = cleanup_workflow_temp(root.to_string_lossy().to_string())
        .await
        .unwrap();
    assert!(freed >= 2048, "应统计到释放的字节数，实际 {}", freed);
    assert!(!root.join(".workflow_temp").exists(), "临时目录应被删除");

    // 再次清理应幂等返回 0
    let freed_again = cleanup_workflow_temp(root.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(freed_again, 0);
    cleanup(&root);
}

/// AI 节点仅校验参数形状（运行依赖模型/Python/网络，不在单测执行）。
/// JSON 逐字复刻 buildCommandOptions 输出，反序列化失败 = 前后端字段漂移。
#[test]
fn options_shape_ai_nodes() {
    serde_json::from_value::<super::upscale::UpscaleOptions>(json!({
        "input_path": "/i", "output_path": "/o",
        "engine_id": "realcugan", "model_id": "models-se",
        "scale": 2, "denoise_level": -1, "tta": false,
        "gpu_id": 0, "tile_size": 0, "recursive": false,
    }))
    .expect("upscale 参数与前端不兼容");

    serde_json::from_value::<super::person_crop::PersonCropOptions>(json!({
        "input_path": "/i", "output_path": "/o", "use_gpu": true,
        "person_enabled": true, "person_conf": 0.3,
        "upper_enabled": false, "upper_conf": 0.3, "upper_tag": "",
        "head_enabled": false, "head_conf": 0.3, "head_tag": "", "head_scale": 1.5,
        "eyes_enabled": false, "eyes_conf": 0.3, "eyes_tag": "", "eyes_scale": 2.0,
        "keep_original_tags": true, "recursive": false,
    }))
    .expect("person-crop 参数与前端不兼容");

    serde_json::from_value::<super::aesthetic::AestheticOptions>(json!({
        "input_path": "/i", "output_path": "/o", "use_gpu": true,
        "move_files": true, "batch_size": 1, "recursive": false,
    }))
    .expect("aesthetic 参数与前端不兼容");

    serde_json::from_value::<super::tagger::TaggerOptions>(json!({
        "input_path": "/i", "model_id": "wd-swinv2-tagger-v3",
        "general_threshold": 0.35, "character_threshold": 0.85,
        "enabled_categories": ["general", "character"], "use_gpu": true,
        "exclude_tags": "", "append_tags": "", "append_position": "append",
        "replace_underscore": true, "output_format": "txt", "json_simplified": false,
        "escape_parentheses": false, "sort_by": "confidence",
        "existing_tags_action": "overwrite", "batch_size": 1, "recursive": false,
    }))
    .expect("tagger 参数与前端不兼容");

    serde_json::from_value::<super::tagger::llm_tagger::LlmTaggerOptions>(json!({
        "input_path": "/i", "api_endpoint": "", "api_key": "", "model_name": "",
        "system_prompt": "", "user_prompt": "", "temperature": 0.7,
        "max_tokens": -1, "image_size": 1024, "top_p": 0.9,
        "skip_existing": false, "output_format": "txt", "json_simplified": false,
        "request_interval_ms": -1, "concurrency": 1, "recursive": false,
    }))
    .expect("llm-tagger 参数与前端不兼容");
}
