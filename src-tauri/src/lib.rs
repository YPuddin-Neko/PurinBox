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
    cancel_tagging,
};
use commands::tagger::llm_tagger::start_llm_tagging;
use commands::{scan_images, get_system_stats};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ！！！必须在任何 ort 调用之前设置！！！
    // 设置 ORT_DYLIB_PATH 指向下载的 ONNX Runtime（GPU 版，同时支持 CPU）
    // ORT_DYLIB_PATH 只在 ort DLL 首次加载时生效，之后改无效
    let ort_loaded = commands::tagger::gpu_runtime::setup_ort_env();
    if ort_loaded {
        eprintln!("[AiTrainTools] ONNX Runtime 已设置: {:?}",
            std::env::var("ORT_DYLIB_PATH").unwrap_or_default());
    } else {
        eprintln!("[AiTrainTools] ONNX Runtime 未找到，首次打标时需要下载");
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
            cancel_tagging,
            start_llm_tagging,
            get_system_stats,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
