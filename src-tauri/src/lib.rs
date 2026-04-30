mod commands;

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
};
use commands::tagger::llm_tagger::start_llm_tagging;
use commands::{scan_images, get_system_stats};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 启动时检查是否有 GPU Runtime，有则设置 ORT_DYLIB_PATH
    let gpu_pref = std::env::var("AIT_USE_GPU").unwrap_or_default();
    if gpu_pref == "1" {
        commands::tagger::gpu_runtime::setup_gpu_runtime_env();
    }

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
            start_llm_tagging,
            get_system_stats,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
