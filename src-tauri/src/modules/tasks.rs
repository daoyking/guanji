//! Scheduled task management — CRUD + background cron runner
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time;
use crate::db::with_db;
use rusqlite::Result as SqlResult;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskItem {
    pub id: String,
    pub name: String,
    pub command: String,
    pub schedule: String, // "daily" | "weekly" | "hourly"
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
}

fn interval_secs(schedule: &str) -> u64 {
    match schedule {
        "hourly" => 3600,
        "minutely" => 60,
        "weekly" => 7 * 24 * 3600,
        _ => 24 * 3600,
    }
}

fn next_run_time(schedule: &str) -> String {
    let secs = interval_secs(schedule);
    chrono::Utc::now()
        .checked_add_signed(chrono::TimeDelta::seconds(secs as i64))
        .map(|t| t.to_rfc3339())
        .unwrap_or_default()
}

fn run_task_command(id: String, cmd: String) {
    use std::process::Command;
    match Command::new("sh").arg("-c").arg(&cmd).output() {
        Ok(out) => {
            if out.status.success() {
                eprintln!("[task {}] success", id);
            } else {
                eprintln!("[task {}] failed", id);
            }
        }
        Err(e) => eprintln!("[task {}] error: {}", id, e),
    }
}

// ─── CRUD Commands ──────────────────────────────────────────────

#[tauri::command]
pub async fn list_tasks() -> Result<Vec<TaskItem>, String> {
    tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, command, schedule, enabled, last_run, next_run \
                 FROM scheduled_tasks ORDER BY id DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TaskItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    command: row.get(2)?,
                    schedule: row.get(3)?,
                    enabled: row.get(4)?,
                    last_run: row.get(5).ok(),
                    next_run: row.get(6).ok(),
                })
            })?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn add_task(name: String, command: String, schedule: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            let id = uuid::Uuid::new_v4().to_string();
            let nr = next_run_time(&schedule);
            conn.execute(
                "INSERT INTO scheduled_tasks (id, name, command, schedule, enabled, next_run) \
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                rusqlite::params![&id, &name, &command, &schedule, &nr],
            )?;
            Ok(id)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_task(id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            conn.execute("DELETE FROM scheduled_tasks WHERE id = ?1", rusqlite::params![&id])?;
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn toggle_task(id: String) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            let current: bool = conn
                .query_row("SELECT enabled FROM scheduled_tasks WHERE id = ?1", rusqlite::params![&id], |r| r.get(0))
                .unwrap_or(false);
            let new_state = !current;
            conn.execute("UPDATE scheduled_tasks SET enabled = ?1 WHERE id = ?2", rusqlite::params![new_state, &id])?;
            Ok(new_state)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn run_now(id: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let cmd: String = with_db(|conn| {
            conn.query_row("SELECT command FROM scheduled_tasks WHERE id = ?1", rusqlite::params![&id], |r| r.get(0))
        })?;
        let bt = tokio::runtime::Handle::current();
        let id_clone = id.clone();
        let cmd_clone = cmd.clone();
        bt.spawn(async move {
            time::sleep(Duration::from_millis(100)).await;
            run_task_command(id_clone, cmd_clone);
        });
        Ok("任务已提交执行".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ─── Background Runner ──────────────────────────────────────────

pub fn spawn_task_runner() {
    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        loop {
            time::sleep(Duration::from_secs(60)).await;
            let now = chrono::Utc::now().to_rfc3339();
            let now_for_query = now.clone();

            let should_run: Vec<(String, String, String)> = match tokio::task::spawn_blocking(move || {
                with_db(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id, command, schedule FROM scheduled_tasks \
                         WHERE enabled = 1 AND (next_run IS NULL OR next_run <= ?1)",
                    )?;
                    let rows = stmt.query_map([&now_for_query], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })?;
                    rows.collect::<SqlResult<Vec<_>>>()
                })
            })
            .await {
                Ok(result) => match result {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            for (tid, cmd, schedule) in should_run {
                let new_next = next_run_time(&schedule);
                let run_now_str = now.clone();
                let tid_clone = tid.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = with_db(|conn| {
                        conn.execute(
                            "UPDATE scheduled_tasks SET last_run = ?1, next_run = ?2 WHERE id = ?3",
                            rusqlite::params![&run_now_str, &new_next, &tid_clone],
                        )?;
                        Ok(())
                    });
                }).await;
                eprintln!("[cron] running task {} → {}", tid, cmd);
                run_task_command(tid, cmd);
            }
        }
    });
}
