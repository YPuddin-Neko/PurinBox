mod commands;
use tauri::Manager;

use commands::image_scale::scale_images;
use commands::image_flip::flip_images;
use commands::resolution_filter::filter_by_resolution;
use commands::file_keeper::keep_specified_files;
use commands::format_convert::convert_format;
use commands::alpha_convert::convert_alpha;
use commands::batch_rename::{preview_rename, execute_rename};
use commands::tagger::{
    get_tagger_models, detect_onnx_model_info,
    import_local_tagger_model, remove_custom_tagger_model,
    check_cuda_available, start_tagging, cancel_tagger_download,
    get_gpu_runtime_status, download_gpu_runtime, cancel_gpu_runtime_download,
    cancel_tagging,
};
use commands::tagger::llm_tagger::{start_llm_tagging, fetch_llm_models, cancel_llm_tagging};
use commands::tag_manager::{load_tag_dataset, save_single_tag_file, save_all_tag_files};
use commands::translator::{translate_tags, get_translation_cache_stats, clear_translation_cache, test_translation};
use commands::{scan_images, get_system_stats};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Python 子进程方式推理，无需在 Rust 侧初始化 ONNX Runtime

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan_images,
            scale_images,
            flip_images,
            filter_by_resolution,
            keep_specified_files,
            convert_format,
            convert_alpha,
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
            start_llm_tagging,
            fetch_llm_models,
            cancel_llm_tagging,
            get_system_stats,
            load_tag_dataset,
            save_single_tag_file,
            save_all_tag_files,
            translate_tags,
            get_translation_cache_stats,
            clear_translation_cache,
            test_translation,
        ])
        .setup(|app| {
            // 初始化翻译缓存数据库路径
            let app_data = app.path().app_data_dir().expect("无法获取 app data 目录");
            commands::translator::init_db_path(app_data);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
