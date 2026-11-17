// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod modules;
mod db;
mod theme;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            // Initialize SQLite database
            db::init_db().map_err(|e| e.to_string())?;

            // Apply persisted theme to webview on startup
            theme::apply_on_startup(app);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // System info
            modules::system::get_system_info,
            modules::system::get_processes,
            // File management
            modules::file::list_large_files,
            modules::file::clean_temp_files,
            modules::file::scan_downloads,
            modules::file::find_duplicates,
            // Database operations (memories & settings)
            db::list_memories,
            db::add_memory,
            db::delete_memory,
            db::get_setting,
            db::set_setting,
            db::list_settings,
            // Model management
            modules::model::list_ollama_models,
            modules::model::pull_ollama_model,
            modules::model::delete_ollama_model,
            // Scheduled tasks
            modules::tasks::list_tasks,
            modules::tasks::add_task,
            modules::tasks::delete_task,
            modules::tasks::toggle_task,
            modules::tasks::run_now,
            // Service health probes
            modules::service::list_services,
            modules::service::probe_service,
            modules::service::run_all_probes,
            // Clipboard history
            modules::clipboard::clip_list,
            modules::clipboard::clip_add,
            modules::clipboard::clip_toggle_pin,
            modules::clipboard::clip_delete,
            // Memory + AI intent analysis
            modules::memory::mem_list,
            modules::memory::mem_add,
            modules::memory::mem_delete,
            modules::memory::mem_analyze,
            modules::memory::mem_search,
            // Notification system (M7)
            modules::notification::notify_push,
            modules::notification::notify_list,
            modules::notification::notify_clear_read,
            modules::notification::notify_mark_read,
            modules::notification::notify_get_prefs,
            modules::notification::notify_set_prefs,
        ])
        .setup(|_app| {
            modules::tasks::spawn_task_runner();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
