//! Theme persistence module: read saved theme from DB and apply it
//! to the Tauri webview on startup.

use super::db;
use tauri::Manager;

/// Called once during app startup (inside setup closure).
/// Reads the "theme" setting from SQLite and applies it to all open windows.
pub fn apply_on_startup(app: &tauri::App) {
    let theme = db::get_setting_inner("theme").unwrap_or_default();
    if !theme.is_empty() {
        apply_to_windows(app, &theme);
    }
}

/// Add/remove `.dark` class on every window's document.
fn apply_to_windows(app: &tauri::App, theme: &str) {
    let is_dark = theme == "dark";
    for (_name, win) in app.webview_windows() {
        // Inject JS directly into the loaded page so the user never sees a flash.
        let js = format!(
            "if(document.documentElement){{document.documentElement.classList.toggle('dark', {})}}",
            is_dark
        );
        let _ = win.eval(&js);
    }
}
