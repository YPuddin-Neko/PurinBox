mod commands;

use commands::aesthetic::{cancel_aesthetic_scoring, start_aesthetic_scoring};
use commands::alpha_convert::{cancel_alpha, convert_alpha};
use commands::api_config::{load_api_config, save_api_config};
use commands::batch_rename::{execute_rename, preview_rename};
use commands::blur_noise::{blur_noise_images, cancel_blur_noise};
use commands::bucket_preview::{analyze_buckets, export_buckets, recommend_bucket_params};
use commands::dedup_rename::{
    cancel_dedup_rename, execute_dedup_rename, export_unmatched_files, scan_dedup_rename,
};
use commands::file_keeper::{cancel_keeper, keep_specified_files};
use commands::format_convert::{cancel_convert, convert_format};
use commands::image_cluster::{
    cancel_image_cluster, force_cancel_image_cluster, start_image_cluster,
};
use commands::image_crop::{cancel_crop, crop_images};
use commands::image_dedup::{cancel_image_dedup, delete_dedup_files, start_image_dedup};
use commands::image_flip::{cancel_flip, flip_images};
use commands::image_scale::{cancel_scale, scale_images};
use commands::person_crop::{
    cancel_person_crop, cancel_person_crop_download, download_person_crop_model,
    get_person_crop_models, start_person_crop,
};
use commands::perspective::{cancel_perspective, perspective_transform};
use commands::proxy_config::{load_proxy_config, save_proxy_config};
use commands::python_env::{deploy_python_env, get_python_env_info, reset_python_env};
use commands::resolution_filter::{cancel_filter, filter_by_resolution};
use commands::sd_metadata::{export_sd_tags, read_single_sd_metadata, scan_sd_metadata};
use commands::tag_db::{
    cancel_tag_db_download, check_tag_db_update, clear_tag_db, download_danbooru_tags,
    get_tag_db_stats, is_tag_db_busy, search_tags, translate_tag_db,
};
use commands::tag_manager::{
    load_caption_dataset, load_json_dataset, load_tag_dataset, save_all_caption_files,
    save_all_json_files, save_all_tag_files, save_caption_file, save_single_json_file,
    save_single_tag_file,
};
use commands::tag_refine::{cancel_tag_refining, start_tag_refining};
use commands::tag_sort::{cancel_tag_sorting, start_tag_sorting};
use commands::tagger::llm_tagger::{cancel_llm_tagging, fetch_llm_models, start_llm_tagging};
use commands::tagger::{
    cancel_gpu_runtime_download, cancel_tagger_download, cancel_tagging, check_cuda_available,
    detect_onnx_model_info, download_gpu_runtime, get_gpu_runtime_status, get_tagger_models,
    import_local_tagger_model, remove_custom_tagger_model, start_tagging,
};
use commands::translator::{
    clear_translation_cache, export_translation_csv, get_cache_path, get_translation_cache_stats,
    import_translation_csv, set_cache_path, test_translation, translate_tags,
};
use commands::upscale::{
    cancel_upscale, cancel_upscale_download, download_upscale_engine, force_cancel_upscale,
    get_upscale_engines, start_upscale,
};
use commands::{
    apply_concept_repeats, check_for_updates, frontend_ready, get_system_stats,
    scan_concept_folders, scan_images,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Python 子进程方式推理，无需在 Rust 侧初始化 ONNX Runtime

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            scan_images,
            scan_concept_folders,
            apply_concept_repeats,
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
            deploy_python_env,
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
            export_translation_csv,
            import_translation_csv,
            start_tag_sorting,
            cancel_tag_sorting,
            start_tag_refining,
            cancel_tag_refining,
            save_api_config,
            load_api_config,
            save_proxy_config,
            load_proxy_config,
            analyze_buckets,
            export_buckets,
            recommend_bucket_params,
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
            scan_dedup_rename,
            cancel_dedup_rename,
            execute_dedup_rename,
            export_unmatched_files,
            scan_sd_metadata,
            export_sd_tags,
            read_single_sd_metadata,
            check_for_updates,
            frontend_ready,
            get_tag_db_stats,
            download_danbooru_tags,
            cancel_tag_db_download,
            clear_tag_db,
            search_tags,
            translate_tag_db,
            is_tag_db_busy,
            check_tag_db_update,
            start_aesthetic_scoring,
            cancel_aesthetic_scoring,
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

            // 启动看门狗（仅 Windows release）：WebView2 偶发会在初始化阶段卡死
            // （例如恰逢系统后台更新 WebView2 运行时），表现为窗口永久白屏且无响应，
            // 用户只能手动杀进程。前端加载完成后会立即调用 frontend_ready 命令；
            // 超时未收到信号说明界面已卡死——自动重启一次，仍失败则弹原生提示框。
            #[cfg(all(target_os = "windows", not(debug_assertions)))]
            std::thread::spawn(|| {
                use std::sync::atomic::Ordering;

                const STARTUP_TIMEOUT_SECS: u64 = 20;
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_secs(STARTUP_TIMEOUT_SECS);
                while std::time::Instant::now() < deadline {
                    if commands::FRONTEND_READY.load(Ordering::SeqCst) {
                        return; // 界面正常加载，看门狗退出
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }

                if std::env::var("PURINBOX_STARTUP_RETRIED").is_err() {
                    // 第一次卡死：直接重新拉起自身后退出。
                    // 不走 AppHandle::restart()——它依赖事件循环，而此时事件循环正卡死。
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe)
                            .args(std::env::args().skip(1))
                            .env("PURINBOX_STARTUP_RETRIED", "1")
                            .spawn();
                    }
                } else {
                    // 重启后仍卡死：用原生消息框告知用户（不依赖应用事件循环）
                    unsafe {
                        use windows::core::w;
                        use windows::Win32::UI::WindowsAndMessaging::{
                            MessageBoxW, MB_ICONERROR, MB_OK,
                        };
                        MessageBoxW(
                            None,
                            w!("界面初始化失败，可能是系统 WebView2 组件暂时不可用。\n请稍后重新打开应用；若反复出现，请尝试重启系统或重新安装 Microsoft Edge WebView2 Runtime。\n\nUI failed to initialize (WebView2 may be temporarily unavailable).\nPlease reopen the app later, or reinstall the WebView2 Runtime if it keeps happening."),
                            w!("PurinBox"),
                            MB_OK | MB_ICONERROR,
                        );
                    }
                }
                std::process::exit(1);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
