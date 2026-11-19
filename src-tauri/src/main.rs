// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

mod modules;
mod db;
mod theme;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            // System info
            modules::system::get_system_info,
            modules::system::get_processes,
            modules::system::kill_process,
            modules::system::clean_memory,
            // File management
            modules::file::list_large_files,
            modules::file::trash_files,
            modules::file::clean_temp_files,
            modules::file::scan_downloads,
            modules::file::find_duplicates,
            modules::file::file_get_config,
            modules::file::file_set_scan_dirs,
            modules::file::file_set_ignore_dirs,
            // Application manager
            modules::apps::list_installed_apps,
            modules::apps::launch_app,
            modules::apps::cleanup_app_data,
            modules::apps::uninstall_app,
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
            // Cloud model providers (OpenAI-compatible)
            modules::provider::provider_list,
            modules::provider::provider_save,
            modules::provider::provider_delete,
            modules::provider::provider_list_models,
            modules::provider::provider_test,
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
            // AI Agent orchestration (M8)
            modules::agent::agent_chat,
            modules::agent::agent_model_name,
            modules::agent::agent_list_models,
            modules::agent::agent_probe_model,
            modules::agent::agent_set_model,
            modules::agent::agent_cancel,
            modules::agent::agent_confirm,
            // Butler execution tools (M8.1)
            modules::shell::run_command,
            modules::shell::open_target,
            modules::shell::read_text_file,
            modules::shell::write_text_file,
            modules::reminder::reminder_add,
            modules::reminder::reminder_list,
            modules::reminder::reminder_delete,
            // AI chat history persistence
            modules::chat::chat_history,
            modules::chat::chat_upsert,
            modules::chat::chat_clear,
        ])
        .setup(|app| {
            // Initialize SQLite database —— 必须最先执行，所有持久化功能都依赖它。
            // 注意：Tauri Builder 只保留最后一个 .setup()，多个 setup 会静默覆盖，
            // 曾因拆成两个 setup 导致 init_db 从未执行、全部 DB 功能失效。
            db::init_db().map_err(|e| e.to_string())?;

            // Agent 运行时控制状态（取消 / 破坏性操作确认桥接）
            app.manage(modules::agent::AgentControl::new());

            // Apply persisted theme to webview on startup
            theme::apply_on_startup(app);

            modules::tasks::spawn_task_runner();
            modules::reminder::spawn_reminder_runner(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
