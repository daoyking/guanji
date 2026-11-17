//! Clipboard manager — history tracking with pinned pins
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use rusqlite::Result as SqlResult;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipEntry {
    pub id: i64,
    pub content: String,
    pub kind: String, // "text" | "html"
    pub created_at: u64,
    pub pinned: bool,
}

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[tauri::command]
pub async fn clip_list() -> Result<Vec<ClipEntry>, String> {
    tokio::task::spawn_blocking(|| {
        crate::db::with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, content, kind, created_at, pinned FROM clip_entries \
                 ORDER BY pinned DESC, created_at DESC LIMIT 50",
            )?;
            let rows = stmt.query_map([], |row| Ok(ClipEntry {
                id: row.get(0)?,
                content: row.get(1)?,
                kind: row.get(2)?,
                created_at: row.get(3)?,
                pinned: row.get(4)?,
            }))?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clip_add(content: String, kind: String) -> Result<i64, String> {
    tokio::task::spawn_blocking(move || {
        let ts = now_ts();
        crate::db::with_db(move |conn| {
            let id = conn.execute(
                "INSERT INTO clip_entries (content, kind, created_at, pinned) VALUES (?1, ?2, ?3, 0)",
                rusqlite::params![&content, &kind, ts],
            )?;
            Ok(id as i64)
        })
    })
    .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clip_toggle_pin(id: i64) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        crate::db::with_db(move |conn| {
            let current: bool = conn.query_row(
                "SELECT pinned FROM clip_entries WHERE id = ?1", rusqlite::params![id],
                |r| r.get(0),
            ).unwrap_or(false);
            let new_state = !current;
            conn.execute("UPDATE clip_entries SET pinned = ?1 WHERE id = ?2", rusqlite::params![new_state, id])?;
            Ok(new_state)
        })
    })
    .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clip_delete(id: i64) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        crate::db::with_db(move |conn| {
            conn.execute("DELETE FROM clip_entries WHERE id = ?1", rusqlite::params![id])?;
            Ok(())
        })
    })
    .await.map_err(|e| e.to_string())?
}
