use std::path::Path;

/// 工作流节点沿用各功能的处理入口，仅为进度事件绑定本次请求。
#[tauri::command]
pub async fn execute_workflow_node(
    app: tauri::AppHandle,
    command: String,
    options: serde_json::Value,
    request_id: String,
) -> Result<serde_json::Value, String> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("工作流请求标识无效".into());
    }
    super::PROGRESS_REQUEST_ID
        .scope(Some(request_id.into()), async move {
            macro_rules! run {
                ($command:path) => {{
                    let options = serde_json::from_value(options)
                        .map_err(|e| format!("工作流节点参数无效: {}", e))?;
                    let result = $command(app, options).await?;
                    serde_json::to_value(result)
                        .map_err(|e| format!("读取工作流节点结果失败: {}", e))
                }};
            }
            match command.as_str() {
                "scale_images" => run!(super::image_scale::scale_images),
                "crop_images" => run!(super::image_crop::crop_images),
                "flip_images" => run!(super::image_flip::flip_images),
                "convert_format" => run!(super::format_convert::convert_format),
                "convert_alpha" => run!(super::alpha_convert::convert_alpha),
                "blur_noise_images" => run!(super::blur_noise::blur_noise_images),
                "perspective_transform" => run!(super::perspective::perspective_transform),
                "start_upscale" => run!(super::upscale::start_upscale),
                "start_person_crop" => run!(super::person_crop::start_person_crop),
                "start_aesthetic_scoring" => run!(super::aesthetic::start_aesthetic_scoring),
                "start_tagging" => run!(super::tagger::start_tagging),
                "start_llm_tagging" => run!(super::tagger::llm_tagger::start_llm_tagging),
                "analyze_buckets" => run!(super::bucket_preview::analyze_buckets),
                "filter_by_resolution" => run!(super::resolution_filter::filter_by_resolution),
                "execute_rename" => run!(super::batch_rename::execute_rename),
                _ => Err(format!("不支持的工作流命令: {}", command)),
            }
        })
        .await
}

/// 保存工作流 JSON 到指定路径
#[tauri::command]
pub async fn save_workflow(path: String, data: String) -> Result<(), String> {
    let file_path = Path::new(&path);
    let data = without_workflow_secrets(&data)?;

    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    super::config_paths::write_file_atomic(file_path, data.as_bytes())
        .map_err(|e| format!("写入工作流失败: {}", e))?;

    Ok(())
}

/// 加载工作流 JSON
#[tauri::command]
pub async fn load_workflow(path: String) -> Result<String, String> {
    let file_path = Path::new(&path);

    if !file_path.exists() {
        return Err(format!("工作流文件不存在: {}", path));
    }

    let data = std::fs::read_to_string(file_path).map_err(|e| format!("读取工作流失败: {}", e))?;
    without_workflow_secrets(&data)
}

fn without_workflow_secrets(data: &str) -> Result<String, String> {
    let mut document: serde_json::Value =
        serde_json::from_str(data).map_err(|e| format!("解析工作流失败: {}", e))?;
    if let Some(nodes) = document.get_mut("nodes").and_then(|v| v.as_array_mut()) {
        for node in nodes {
            if let Some(params) = node
                .pointer_mut("/data/params")
                .and_then(|v| v.as_object_mut())
            {
                params.remove("api_key");
            }
        }
    }
    serde_json::to_string(&document).map_err(|e| format!("序列化工作流失败: {}", e))
}

/// 清理工作流临时目录（{dir}/.workflow_temp）。
///
/// 取消工作流时刚强杀完当前节点的子进程（Python/NCNN），Windows 上其打开的
/// 文件句柄可能尚未释放，首次删除会报拒绝访问——失败后短暂等待并重试。
#[tauri::command]
pub async fn cleanup_workflow_temp(dir: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let temp_dir = Path::new(&dir).join(".workflow_temp");

        if !temp_dir.exists() {
            return Ok(());
        }

        let mut last_err = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            match std::fs::remove_dir_all(&temp_dir) {
                Ok(_) => return Ok(()),
                Err(e) => last_err = e.to_string(),
            }
            if !temp_dir.exists() {
                return Ok(());
            }
        }
        Err(format!("清理临时目录失败: {}", last_err))
    })
    .await
    .map_err(|e| format!("清理任务执行失败: {}", e))?
}

/// 把输入目录里与输出目录图片同名（stem）的标签文件（.txt/.json/.caption）带到输出目录。
/// 图像处理节点只搬图片；打标节点在上游时，标签会被留在临时目录里随清理丢失。
/// 默认保留已有标签；copy_images 用于输出节点，复制实际产物并更新对应标签。
#[tauri::command]
pub async fn carry_tag_sidecars(
    input_path: String,
    output_path: String,
    recursive: bool,
    copy_images: Option<bool>,
) -> Result<u32, String> {
    tokio::task::spawn_blocking(move || {
        carry_tag_sidecars_sync(
            &input_path,
            &output_path,
            recursive,
            copy_images.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| format!("复制工作流产物失败: {}", e))?
}

fn carry_tag_sidecars_sync(
    input_path: &str,
    output_path: &str,
    recursive: bool,
    copy_images: bool,
) -> Result<u32, String> {
    let input = Path::new(&input_path);
    let output = Path::new(&output_path);
    if copy_images {
        let files =
            super::collect_image_files_with_recursive_excluding(input, recursive, Some(output))?;
        std::fs::create_dir_all(output).map_err(|e| format!("创建输出目录失败: {}", e))?;
        if std::fs::canonicalize(input).ok() == std::fs::canonicalize(output).ok() {
            return Ok(0);
        }
        let input_root = super::dir_of(input);
        let mut copied = 0;
        // 输出节点交付这一轮的最终产物：同名图片和标签直接覆盖，与其他功能写输出目录一致。
        // 下面补标签的分支用在中间步骤之间，输出目录里已有的标签是这一步自己写的、比输入侧新，所以跳过
        for file in files {
            let dest = super::same_name_output(&input_root, &file, output, recursive)?;
            std::fs::copy(&file, &dest)
                .map_err(|e| format!("复制图片失败 ({}): {}", file.display(), e))?;
            copied += 1;
            for (ext, sidecar) in super::tag_sidecars(&file) {
                if sidecar.is_file() {
                    std::fs::copy(&sidecar, dest.with_extension(ext))
                        .map_err(|e| format!("复制标签失败 ({}): {}", sidecar.display(), e))?;
                }
            }
        }
        return Ok(copied);
    }
    if !input.is_dir() || !output.is_dir() {
        return Ok(0);
    }
    // 先按相对路径索引输入侧的标签文件；没有标签文件时直接返回。
    let mut avail: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let walker = if recursive {
        walkdir::WalkDir::new(input)
    } else {
        walkdir::WalkDir::new(input).max_depth(1)
    };
    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| super::TAG_SIDECAR_EXTS.contains(&ext))
        {
            if let Ok(rel) = p.strip_prefix(input) {
                avail.insert(rel.to_path_buf());
            }
        }
    }
    if avail.is_empty() {
        return Ok(0);
    }

    let images = super::collect_image_files_with_recursive(output, recursive)?;
    let mut copied = 0u32;
    for img in images {
        // 输出图片相对输出根的位置，映射回输入根找同名标签
        let rel = img.strip_prefix(output).unwrap_or(&img);
        for (ext, rel_sc) in super::tag_sidecars(rel) {
            if !avail.contains(&rel_sc) {
                continue;
            }
            let src = input.join(&rel_sc);
            let dst = img.with_extension(ext);
            if !dst.exists() && src != dst && std::fs::copy(&src, &dst).is_ok() {
                copied += 1;
            }
        }
    }
    Ok(copied)
}
