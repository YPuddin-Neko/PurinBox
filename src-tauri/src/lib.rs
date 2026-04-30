mod commands;

use commands::image_scale::scale_images;
use commands::image_flip::flip_images;
use commands::resolution_filter::filter_by_resolution;
use commands::file_keeper::keep_specified_files;
use commands::format_convert::convert_format;
use commands::alpha_convert::convert_alpha;
use commands::batch_rename::{preview_rename, execute_rename};
use commands::tagger::{
    get_tagger_models, add_custom_tagger_model,
    check_cuda_available, start_tagging,
};
use commands::tagger::llm_tagger::start_llm_tagging;
use commands::scan_images;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            add_custom_tagger_model,
            check_cuda_available,
            start_tagging,
            start_llm_tagging,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
