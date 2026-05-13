mod commands;

use commands::image_scale::{scale_images, cancel_scale};
use commands::image_crop::{crop_images, cancel_crop};
use commands::image_flip::{flip_images, cancel_flip};
use commands::person_crop::{start_person_crop, cancel_person_crop, get_person_crop_models, download_person_crop_model, cancel_person_crop_download};
use commands::resolution_filter::{filter_by_resolution, cancel_filter};
use commands::file_keeper::{keep_specified_files, cancel_keeper};
use commands::format_convert::{convert_format, cancel_convert};
use commands::alpha_convert::{convert_alpha, cancel_alpha};
use commands::batch_rename::{preview_rename, execute_rename};
use commands::tagger::{
    get_tagger_models, detect_onnx_model_info,
    import_local_tagger_model, remove_custom_tagger_model,
    check_cuda_available, start_tagging, cancel_tagger_download,
    get_gpu_runtime_status, download_gpu_runtime, cancel_gpu_runtime_download,
    cancel_tagging,
};
use commands::python_env::{reset_python_env, get_python_env_info};
use commands::tagger::llm_tagger::{start_llm_tagging, fetch_llm_models, cancel_llm_tagging};
use commands::tag_manager::{load_tag_dataset, save_single_tag_file, save_all_tag_files, save_caption_file, save_all_caption_files, load_caption_dataset, load_json_dataset, save_single_json_file, save_all_json_files};
use commands::translator::{translate_tags, get_translation_cache_stats, clear_translation_cache, test_translation, get_cache_path, set_cache_path};
use commands::tag_sort::{start_tag_sorting, cancel_tag_sorting};
use commands::api_config::{save_api_config, load_api_config};
use commands::proxy_config::{save_proxy_config, load_proxy_config};
use commands::bucket_preview::{analyze_buckets, export_buckets};
use commands::perspective::{perspective_transform, cancel_perspective};
use commands::blur_noise::{blur_noise_images, cancel_blur_noise};
use commands::upscale::{get_upscale_engines, download_upscale_engine, cancel_upscale_download, start_upscale, cancel_upscale, force_cancel_upscale};
use commands::image_cluster::{start_image_cluster, cancel_image_cluster, force_cancel_image_cluster};
use commands::image_dedup::{start_image_dedup, cancel_image_dedup, delete_dedup_files};
use commands::{scan_images, get_system_stats, check_for_updates};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Python 子进程方式推理，无需在 Rust 侧初始化 ONNX Runtime

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            scan_images,
            scale_images,
            cancel_scale,
            crop_images,
            cancel_crop,
            flip_images,
            cancel_flip,
            start_person_crop,
            cancel_person_crop,
            get_person_crop_models,
            download_person_crop_model,
            cancel_person_crop_download,
            filter_by_resolution,
            cancel_filter,
            keep_specified_files,
            cancel_keeper,
            convert_format,
            cancel_convert,
            convert_alpha,
            cancel_alpha,
            preview_rename,
            execute_rename,
            get_tagger_models,
            detect_onnx_model_info,
            import_local_tagger_model,
            remove_custom_tagger_model,
            check_cuda_available,
            start_tagging,
            cancel_tagger_download,
            get_gpu_runtime_status,
            download_gpu_runtime,
            cancel_gpu_runtime_download,
            cancel_tagging,
            reset_python_env,
            get_python_env_info,
            start_llm_tagging,
            fetch_llm_models,
            cancel_llm_tagging,
            get_system_stats,
            load_tag_dataset,
            save_single_tag_file,
            save_all_tag_files,
            save_caption_file,
            save_all_caption_files,
            load_caption_dataset,
            load_json_dataset,
            save_single_json_file,
            save_all_json_files,
            translate_tags,
            get_translation_cache_stats,
            clear_translation_cache,
            test_translation,
            get_cache_path,
            set_cache_path,
            start_tag_sorting,
            cancel_tag_sorting,
            save_api_config,
            load_api_config,
            save_proxy_config,
            load_proxy_config,
            analyze_buckets,
            export_buckets,
            perspective_transform,
            cancel_perspective,
            blur_noise_images,
            cancel_blur_noise,
            get_upscale_engines,
            download_upscale_engine,
            cancel_upscale_download,
            start_upscale,
            cancel_upscale,
            force_cancel_upscale,
            start_image_cluster,
            cancel_image_cluster,
            force_cancel_image_cluster,
            start_image_dedup,
            cancel_image_dedup,
            delete_dedup_files,
            check_for_updates,
        ])
        .setup(|app| {
            // 初始化翻译缓存数据库路径（默认使用 exe 根目录/tagcache/）
            commands::translator::init_db_path(None);

            // Windows: 禁用 WebView2 的默认右键菜单（前端已有自定义右键菜单）
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| {
                        unsafe {
                            use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings;
                            let core = webview.controller().CoreWebView2().unwrap();
                            let settings: ICoreWebView2Settings = core.Settings().unwrap();
                            settings.SetAreDefaultContextMenusEnabled(false).unwrap_or(());
                        }
                    });
                }
            }

            // 所有平台: 禁用 WebView 缩放快捷键（Ctrl+滚轮/Ctrl++/-），防止意外缩放
            #[cfg(not(target_os = "windows"))]
            let _ = app;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
