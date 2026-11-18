//! Database layer using SQLite (via rusqlite bundled)
use rusqlite::{Connection, Result as SqlResult};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

#[derive(Serialize, Clone, Debug)]
pub struct MemoryItem {
    pub id: i64,
    pub content: String,
    pub created_at: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct AppSettings {
    pub key: String,
    pub value: String,
}

/// Initialize the database (call once at app startup)
pub fn init_db() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = dirs::data_dir()
        .or_else(dirs::state_dir)
        .unwrap_or_else(|| dirs::cache_dir().unwrap_or_default());
    let guanji_dir = db_path.join("guanji");
    std::fs::create_dir_all(&guanji_dir)?;
    let conn_path = guanji_dir.join("db.sqlite3");
    let conn = Connection::open(conn_path)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS scheduled_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            command TEXT NOT NULL,
            schedule TEXT NOT NULL DEFAULT 'daily',
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run TEXT,
            next_run TEXT
        );
        CREATE TABLE IF NOT EXISTS clip_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            kind TEXT NOT NULL DEFAULT 'text',
            created_at REAL NOT NULL,
            pinned INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            message TEXT NOT NULL,
            remind_at TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            fired INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            traces TEXT NOT NULL DEFAULT '[]',
            steps INTEGER,
            model TEXT,
            time TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    // 幂等 migration：老库的 memories 表可能缺少 tag/summary 列。
    // mem_add / 经验沉淀 / 语义搜索都依赖这两列，缺列会导致运行时 SQL 报错。
    {
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memories)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<SqlResult<Vec<_>>>()?;
        if !cols.iter().any(|c| c == "tag") {
            conn.execute("ALTER TABLE memories ADD COLUMN tag TEXT", [])?;
        }
        if !cols.iter().any(|c| c == "summary") {
            conn.execute("ALTER TABLE memories ADD COLUMN summary TEXT", [])?;
        }
    }
    DB.set(Mutex::new(conn))
        .map_err(|_| "Database already initialized".to_string())?;
    Ok(())
}

pub(crate) fn with_db<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&Connection) -> Result<R, rusqlite::Error>,
{
    let conn = DB.get().ok_or("Database not initialized")?;
    let guard = conn.lock().map_err(|e| e.to_string())?;
    f(&guard).map_err(|e| e.to_string())
}

/// Synchronous getter for theme (used at startup before event loop)
pub fn get_setting_inner(key: &str) -> Result<String, String> {
    let conn = DB.get().ok_or("Database not initialized")?;
    let guard = conn.lock().map_err(|e| e.to_string())?;
    guard.query_row(
        "SELECT value FROM settings WHERE key = :key",
        rusqlite::params![key],
        |row| row.get(0),
    )
    .or_else(|_| Ok(String::new()))
}
// ─── Memories ───────────────────────────────────────────────

#[tauri::command]
pub async fn list_memories() -> Result<Vec<MemoryItem>, String> {
    tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, content, created_at FROM memories ORDER BY created_at DESC LIMIT 50",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(MemoryItem {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn add_memory(content: String) -> Result<i64, String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "INSERT INTO memories (content) VALUES (:content)",
            )?;
            let id = stmt.insert(rusqlite::params![content])?;
            Ok(id)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_memory(id: i64) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            conn.execute("DELETE FROM memories WHERE id = :id", rusqlite::params![id])?;
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ─── Settings ───────────────────────────────────────────────

#[tauri::command]
pub async fn get_setting(key: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = :key",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .or_else(|_| Ok(String::new()))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_setting(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (:key, :value)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![&key, &value],
            )?;
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_settings() -> Result<Vec<AppSettings>, String> {
    tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
            let rows = stmt.query_map([], |row| Ok(AppSettings {
                key: row.get(0)?,
                value: row.get(1)?,
            }))?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
