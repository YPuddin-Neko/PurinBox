//! 工作流节点命令的运行时测试。
//!
//! 每个测试的参数 JSON 对应前端 nodeDefinitions.buildOptions 产出的字段——
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

#[tokio::test]
async fn workflow_secrets_never_leave_legacy_load_or_save() {
    let root =
        std::env::temp_dir().join(format!("purinbox_workflow_secrets_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let legacy = root.join("legacy.purin");
    let saved = root.join("saved.purin");
    let original = json!({"nodes":[{"data":{"type":"llm-tagger","params":{"api_key":"test-only-secret", "model_name":"test-model"}}}], "edges":[]}).to_string();
    std::fs::write(&legacy, &original).unwrap();
    let loaded = super::workflow::load_workflow(legacy.to_string_lossy().into())
        .await
        .unwrap();
    assert!(!loaded.contains("test-only-secret"));
    assert!(!loaded.contains("api_key"));
    assert!(loaded.contains("test-model"));
    assert_eq!(std::fs::read_to_string(&legacy).unwrap(), original);
    super::workflow::save_workflow(saved.to_string_lossy().into(), original)
        .await
        .unwrap();
    assert!(!std::fs::read_to_string(saved).unwrap().contains("api_key"));
    cleanup(&root);
}

#[tokio::test]
async fn workflow_output_copies_actual_images_and_sidecars() {
    let root = std::env::temp_dir().join(format!("purinbox_workflow_copy_{}", std::process::id()));
    let input = root.join("input");
    let output = input.join("output");
    std::fs::create_dir_all(input.join("nested")).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(input.join("nested/image.png"), b"image").unwrap();
    for ext in ["txt", "json", "caption"] {
        std::fs::write(input.join(format!("nested/image.{ext}")), b"current tags").unwrap();
    }
    std::fs::write(output.join("old.png"), b"old").unwrap();
    let copy = || {
        super::workflow::carry_tag_sidecars(
            input.to_string_lossy().into(),
            output.to_string_lossy().into(),
            true,
            Some(true),
        )
    };
    assert_eq!(copy().await.unwrap(), 1);
    assert_eq!(
        std::fs::read(output.join("nested/image.png")).unwrap(),
        b"image"
    );
    for ext in ["txt", "json", "caption"] {
        assert_eq!(
            std::fs::read(output.join(format!("nested/image.{ext}"))).unwrap(),
            b"current tags"
        );
    }
    assert!(!output.join("output").exists());
    assert!(input.join("nested/image.png").exists());
    std::fs::write(output.join("nested/image.txt"), b"stale tags").unwrap();
    assert_eq!(copy().await.unwrap(), 1);
    assert_eq!(
        std::fs::read(output.join("nested/image.txt")).unwrap(),
        b"current tags"
    );
    assert_eq!(
        super::workflow::carry_tag_sidecars(
            input.to_string_lossy().into(),
            input.to_string_lossy().into(),
            true,
            Some(true)
        )
        .await
        .unwrap(),
        0
    );
    cleanup(&root);
}

#[tokio::test]
async fn workflow_sidecars_keep_existing_and_match_relative_paths() {
    let root =
        std::env::temp_dir().join(format!("purinbox_workflow_sidecars_{}", std::process::id()));
    let input = root.join("input");
    let output = root.join("output");
    for dir in [&input, &output] {
        std::fs::create_dir_all(dir.join("nested")).unwrap();
    }
    std::fs::write(input.join("nested/image.txt"), b"source").unwrap();
    std::fs::write(input.join("nested/image.json"), b"json").unwrap();
    std::fs::write(output.join("nested/image.webp"), b"image").unwrap();
    std::fs::write(output.join("nested/image.txt"), b"existing").unwrap();
    assert_eq!(
        super::workflow::carry_tag_sidecars(
            input.to_string_lossy().into(),
            output.to_string_lossy().into(),
            true,
            None
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        std::fs::read(output.join("nested/image.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        std::fs::read(output.join("nested/image.json")).unwrap(),
        b"json"
    );
    cleanup(&root);
}

/// 生成测试图集：5 张不同尺寸/格式（含 1 张 JPG 与 1 张带透明通道）
fn make_dataset(tag: &str) -> (PathBuf, PathBuf) {
    let root =
        std::env::temp_dir().join(format!("purinbox_wf_nodes_{}_{}", tag, std::process::id()));
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

/// 每张源图都有同名输出，且输出的实际编码格式与源图一致
fn assert_formats_kept(input: &Path, out: &Path) {
    let real_format = |p: &Path| image::guess_format(&std::fs::read(p).unwrap()).unwrap();
    for entry in std::fs::read_dir(input).unwrap().flatten() {
        let src = entry.path();
        let dst = out.join(entry.file_name());
        assert!(dst.exists(), "缺少输出 {}", dst.display());
        assert_eq!(real_format(&dst), real_format(&src), "{}", dst.display());
    }
}

fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

/// 纯 Rust 图像节点用例：input_path / output_path / recursive 这几个公共字段由宏注入，
/// `{ … }` 里写该节点自己的字段。断言无失败且 5 张图全部输出；
/// 可选的 `|input, out| …` 追加该节点自己的断言。
macro_rules! image_node_test {
    (
        $name:ident, $node:literal, $what:literal, $cmd:ident, { $($field:tt)* }
        $(, |$input:ident, $out:ident| $check:expr)? $(,)?
    ) => {
        #[tokio::test]
        async fn $name() {
            let app = tauri::test::mock_app();
            let (root, input) = make_dataset($node);
            let out = root.join("out");

            let opts = serde_json::from_value(json!({
                "input_path": input.to_string_lossy(),
                "output_path": out.to_string_lossy(),
                "recursive": false,
                $($field)*
            }))
            .expect(concat!($node, " 参数与前端不兼容"));

            let r = $cmd(app.handle().clone(), opts).await.unwrap();
            assert_eq!(r.fail_count, 0, concat!($what, "不应有失败: {:?}"), r.errors);
            assert_eq!(file_count(&out), 5, "输出目录应包含全部 5 张图");
            $({
                let ($input, $out) = (input.as_path(), out.as_path());
                $check;
            })?
            cleanup(&root);
        }
    };
}

image_node_test!(node_scale, "scale", "缩放", scale_images, {
    "mode": "upscale",
    "target_width": 1024,
    "target_height": 1024,
    "down_target_width": 0,
    "down_target_height": 0,
}, |input, out| assert_formats_kept(input, out));

image_node_test!(node_crop, "crop", "裁切", crop_images, {
    "mode": "center",
    "crop_anchor": "center",
    "target_width": 256,
    "target_height": 256,
    "aspect_ratio": 1.0,
    "crop_top": 0, "crop_bottom": 0, "crop_left": 0, "crop_right": 0,
}, |input, out| assert_formats_kept(input, out));

image_node_test!(node_flip, "flip", "翻转", flip_images, {
    "direction": "horizontal",
}, |input, out| assert_formats_kept(input, out));

// 已是目标格式的文件也要复制到输出目录，否则下游节点会拿到不完整的数据集
image_node_test!(node_format_convert, "format-convert", "格式转换", convert_format, {
    "target_format": "png",
});

image_node_test!(node_alpha_convert, "alpha-convert", "透明通道转换", convert_alpha, {
    "background": "white",
}, |input, out| {
    // 带透明通道的图转换后不应再有半透明像素
    let converted = image::open(out.join("d_alpha.png")).unwrap().to_rgba8();
    assert!(
        converted.pixels().all(|p| p[3] == 255),
        "转换后不应存在透明像素"
    );
    assert_formats_kept(input, out);
});

image_node_test!(node_blur_noise, "blur-noise", "模糊/噪点", blur_noise_images, {
    "blur_radius": 1.5,
    "noise_strength": 5,
}, |input, out| assert_formats_kept(input, out));

image_node_test!(node_perspective, "perspective", "透视变换", perspective_transform, {
    "intensity": 0.1,
}, |input, out| assert_formats_kept(input, out));

#[tokio::test]
async fn node_filter() {
    let app = tauri::test::mock_app();
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

    let r = filter_by_resolution(app.handle().clone(), opts)
        .await
        .unwrap();
    assert_eq!(r.fail_count, 0, "分辨率筛选不应有失败: {:?}", r.errors);
    let hit = file_count(&out);
    assert!(
        (1..5).contains(&hit),
        "应筛出部分低分辨率图片，实际 {}",
        hit
    );
    assert_eq!(file_count(&input), 5, "copy 模式不应动原目录");
    cleanup(&root);
}

#[tokio::test]
async fn node_rename() {
    let app = tauri::test::mock_app();
    let (root, input) = make_dataset("rename");
    // 添加同名标签文件，覆盖 rename_tags 联动。
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
    let app = tauri::test::mock_app();
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

    cleanup_workflow_temp(root.to_string_lossy().to_string())
        .await
        .unwrap();
    assert!(!root.join(".workflow_temp").exists(), "临时目录应被删除");

    // 再次清理应幂等
    cleanup_workflow_temp(root.to_string_lossy().to_string())
        .await
        .unwrap();
    cleanup(&root);
}

/// AI 节点仅校验参数形状（运行依赖模型/Python/网络，不在单测执行）。
/// JSON 字段对应节点 buildOptions 输出，反序列化失败表示前后端字段漂移。
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
        "copy_files": true, "batch_size": 1, "recursive": false,
    }))
    .expect("aesthetic 参数与前端不兼容");

    serde_json::from_value::<super::tagger::TaggerOptions>(json!({
        "input_path": "/i", "model_id": "wd-swinv2-tagger-v3",
        "general_threshold": 0.35, "character_threshold": 0.85,
        "enabled_categories": ["general", "character"], "use_gpu": true,
        "exclude_tags": "", "append_tags": "", "append_position": "append",
        "json_append_field": "tags",
        "replace_underscore": true, "output_format": "txt", "json_simplified": false,
        "escape_parentheses": false, "sort_by": "confidence",
        "existing_tags_action": "overwrite", "batch_size": 1, "recursive": false,
    }))
    .expect("tagger 参数与前端不兼容");

    serde_json::from_value::<super::tagger::llm_tagger::LlmTaggerOptions>(json!({
        "input_path": "/i", "api_endpoint": "", "api_key": "", "model_name": "",
        "system_prompt": "", "user_prompt": "", "temperature": 0.7,
        "max_tokens": -1, "image_size": 1024, "image_detail": "",
        "skip_existing": false, "output_format": "txt", "json_simplified": false,
        "request_interval_ms": -1, "concurrency": 1, "recursive": false,
    }))
    .expect("llm-tagger 参数与前端不兼容");
}
