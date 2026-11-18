//! 定时提醒：一次性提醒（到点弹系统通知），供 Agent 日程类指令使用。
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use crate::db::with_db;
use rusqlite::Result as SqlResult;
use std::time::Duration;
use tokio::time;

#[derive(Serialize, Clone, Debug)]
pub struct Reminder {
    pub id: i64,
    pub message: String,
    pub remind_at: String,
    pub fired: bool,
}

/// 解析相对时间（如 "5分钟后"、"30分钟后"、"2小时后"）或 ISO8601 绝对时间，
/// 返回 UTC RFC3339 字符串。
pub fn parse_remind_time(input: &str) -> Result<String, String> {
    let s = input.trim();
    let now = chrono::Local::now();
    // 「半小时后」等半量词特判
    if s == "半小时后" { return Ok((now + chrono::Duration::minutes(30)).to_rfc3339()); }
    let target = if let Some(n) = parse_cn_number(s, "分钟后") {
        now + chrono::Duration::minutes(n)
    } else if let Some(n) = parse_cn_number(s, "小时后") {
        now + chrono::Duration::hours(n)
    } else if let Some(n) = parse_cn_number(s, "天后") {
        now + chrono::Duration::days(n)
    } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        dt.with_timezone(&chrono::Local)
    } else {
        return Err("时间格式无法识别，试试「5分钟后」「2小时后」或 ISO8601 时间".to_string());
    };
    Ok(target.to_rfc3339())
}

fn parse_cn_number(s: &str, suffix: &str) -> Option<i64> {
    let idx = s.find(suffix)?;
    let num_part = s[..idx].trim();
    if let Ok(v) = num_part.parse::<i64>() {
        return Some(v);
    }
    // 简单中文数字
    match num_part {
        "一" => Some(1), "两" | "二" => Some(2), "三" => Some(3), "四" => Some(4),
        "五" => Some(5), "六" => Some(6), "七" => Some(7), "八" => Some(8),
        "九" => Some(9), "十" => Some(10), _ => None,
    }
}

#[tauri::command]
pub async fn reminder_add(message: String, when: String) -> Result<i64, String> {
    let remind_at = parse_remind_time(&when)?;
    let msg = message.clone();
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            let id = conn.execute(
                "INSERT INTO reminders (message, remind_at) VALUES (?1, ?2)",
                rusqlite::params![msg, remind_at],
            )?;
            Ok(id as i64)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn reminder_list() -> Result<Vec<Reminder>, String> {
    tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, message, remind_at, fired FROM reminders \
                 WHERE fired = 0 ORDER BY remind_at ASC LIMIT 50",
            )?;
            let rows = stmt.query_map([], |r| Ok(Reminder {
                id: r.get(0)?,
                message: r.get(1)?,
                remind_at: r.get(2)?,
                fired: r.get::<_, i32>(3)? != 0,
            }))?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn reminder_delete(id: i64) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        with_db(|conn| {
            conn.execute("DELETE FROM reminders WHERE id = ?1", rusqlite::params![id])?;
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 后台提醒轮询器：每 15 秒检查到点的提醒并弹通知
pub fn spawn_reminder_runner(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("reminder-runner".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build reminder runtime");
            rt.block_on(async move {
                loop {
                    time::sleep(Duration::from_secs(15)).await;
                    let now = chrono::Utc::now().to_rfc3339();
                    let now_q = now.clone();
                    let due: Vec<(i64, String)> = match tokio::task::spawn_blocking(move || {
                        with_db(|conn| {
                            let mut stmt = conn.prepare(
                                "SELECT id, message FROM reminders WHERE fired = 0 AND remind_at <= ?1",
                            )?;
                            let rows = stmt.query_map([&now_q], |r| {
                                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                            })?;
                            rows.collect::<SqlResult<Vec<_>>>()
                        })
                    })
                    .await {
                        Ok(Ok(v)) => v,
                        _ => continue,
                    };
                    for (id, message) in due {
                        let id_mark = id;
                        let _ = tokio::task::spawn_blocking(move || {
                            with_db(|conn| {
                                conn.execute("UPDATE reminders SET fired = 1 WHERE id = ?1",
                                    rusqlite::params![id_mark])?;
                                Ok(())
                            })
                        }).await;
                        let _ = app.emit("agent://reminder", serde_json::json!({
                            "id": id, "message": message
                        }));
                        send_notification(&app, "管家提醒", &message);
                    }
                }
            });
        })
        .expect("failed to spawn reminder runner thread");
}

fn send_notification(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_relative_minutes() {
        let r = parse_remind_time("5分钟后").unwrap();
        let dt = chrono::DateTime::parse_from_rfc3339(&r).unwrap();
        let diff = dt.signed_duration_since(chrono::Local::now()).num_minutes();
        assert!(diff >= 4 && diff <= 5, "diff={}", diff);
    }

    #[test]
    fn parse_relative_hours() {
        let r = parse_remind_time("2小时后").unwrap();
        let dt = chrono::DateTime::parse_from_rfc3339(&r).unwrap();
        let diff = dt.signed_duration_since(chrono::Local::now()).num_hours();
        assert!(diff == 1 || diff == 2, "diff={}", diff);
    }

    #[test]
    fn parse_absolute_time() {
        assert!(parse_remind_time("2026-12-01T09:00:00+08:00").is_ok());
    }

    #[test]
    fn reject_garbage() {
        assert!(parse_remind_time("随便什么时候").is_err());
    }
}
