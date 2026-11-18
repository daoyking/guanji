//! M7: 主动推送策略调优 — 智能通知系统
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::collections::HashMap;
use uuid::Uuid;
use chrono::Local;
use tauri_plugin_notification::NotificationExt;
use tauri::AppHandle;

/// Notification priority levels
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}


/// Notification record stored in memory (volatile)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationItem {
    pub id: String,
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub category: String, // "system", "task", "memory", "file"
    pub created_at: u64,
    pub read: bool,
}

/// User notification preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyPrefs {
    pub enabled: bool,
    pub quiet_start: u32,   // HH from 0-23
    pub quiet_end: u32,     // HH from 0-23
    pub silent_categories: Vec<String>,
    pub max_per_hour: usize,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_start: 22,
            quiet_end: 7,
            silent_categories: vec![],
            max_per_hour: 20,
        }
    }
}

// Thread-safe state using Mutex with OnceLock-like pattern
struct AppState {
    notifications: Mutex<Vec<NotificationItem>>,
    prefs: Mutex<NotifyPrefs>,
    hourly_counts: Mutex<HashMap<String, usize>>,
}

unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}

static STATE: std::sync::OnceLock<AppState> = std::sync::OnceLock::new();

fn get_state() -> &'static AppState {
    STATE.get_or_init(|| AppState {
        notifications: Mutex::new(Vec::new()),
        prefs: Mutex::new(NotifyPrefs::default()),
        hourly_counts: Mutex::new(HashMap::new()),
    })
}

/// Check if current time is in quiet hours
fn is_quiet_hours(start: u32, end: u32) -> bool {
    let now = Local::now().format("%H").to_string().parse::<u32>().unwrap_or(0);
    if start < end {
        now >= start && now < end
    } else {
        // Crosses midnight (e.g., 22-7)
        now >= start || now < end
    }
}

// ─── Commands ────────────────────────────────────────────────

#[tauri::command]
pub async fn notify_push(app: AppHandle, title: String, body: String, category: Option<String>, priority: Option<String>) -> Result<String, String> {
    let cat = category.unwrap_or_else(|| "system".to_string());
    let pri_str = priority.unwrap_or_else(|| "Normal".to_string());
    let prio = match pri_str.as_str() {
        "Low" => Priority::Low,
        "High" => Priority::High,
        "Critical" => Priority::Critical,
        _ => Priority::Normal,
    };
    let ts = chrono::Utc::now().timestamp() as u64;
    let id = format!("n_{}", Uuid::new_v4().simple());

    let item = NotificationItem {
        id: id.clone(),
        title: title.clone(),
        body: body.clone(),
        priority: prio.clone(),
        category: cat.clone(),
        created_at: ts,
        read: false,
    };

    // Check prefs and rate limiting
    {
        let prefs = get_state().prefs.lock().map_err(|e| e.to_string())?;
        if !prefs.enabled { return Ok(id); }
        if prefs.silent_categories.contains(&cat) { return Ok(id); }
        if is_quiet_hours(prefs.quiet_start, prefs.quiet_end)
            && !matches!(prio, Priority::Critical) {
                return Ok(id);
            }
    }

    // Store notification
    {
        let mut notifs = get_state().notifications.lock().map_err(|e| e.to_string())?;
        notifs.insert(0, item);
        if notifs.len() > 100 {
            notifs.truncate(100);
        }
    }

    // Update hourly counts
    {
        let hour = Local::now().format("%Y%m%d%H").to_string();
        let mut counts = get_state().hourly_counts.lock().map_err(|e| e.to_string())?;
        *counts.entry(hour).or_insert(0) += 1;
    }

    // Show native OS notification
    if let Err(e) = app.notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
    {
        eprintln!("Failed to show notification: {}", e);
    }

    Ok(id)
}

#[tauri::command]
pub async fn notify_list() -> Result<Vec<NotificationItem>, String> {
    let notifs = get_state().notifications.lock().map_err(|e| e.to_string())?;
    Ok(notifs.clone())
}

#[tauri::command]
pub async fn notify_clear_read() -> Result<usize, String> {
    let mut notifs = get_state().notifications.lock().map_err(|e| e.to_string())?;
    let before = notifs.len();
    notifs.retain(|n| !n.read);
    Ok(before - notifs.len())
}

#[tauri::command]
pub async fn notify_mark_read(id: String) -> Result<(), String> {
    let mut notifs = get_state().notifications.lock().map_err(|e| e.to_string())?;
    if let Some(item) = notifs.iter_mut().find(|n| n.id == id) {
        item.read = true;
    }
    Ok(())
}

#[tauri::command]
pub async fn notify_get_prefs() -> Result<NotifyPrefs, String> {
    let prefs = get_state().prefs.lock().unwrap();
    Ok(prefs.clone())
}

#[tauri::command]
pub async fn notify_set_prefs(new_prefs: NotifyPrefs) -> Result<(), String> {
    let mut prefs = get_state().prefs.lock().map_err(|e| e.to_string())?;
    *prefs = new_prefs;
    Ok(())
}
