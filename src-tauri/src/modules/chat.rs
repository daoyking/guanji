//! AI 管家对话历史持久化 —— 存 SQLite，重启应用后自动恢复。
//! 每条消息（含工具调用轨迹）以 JSON 存储，支持追加 / 更新 / 清空。

use serde::{Deserialize, Serialize};
use crate::db::with_db;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default = "empty_traces")]
    pub traces: Vec<serde_json::Value>,
    #[serde(default)]
    pub steps: Option<usize>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub time: String,
}

fn empty_traces() -> Vec<serde_json::Value> { vec![] }

const MAX_KEEP: usize = 200;

/// 读取对话历史（最近 MAX_KEEP 条，按时间正序）
#[tauri::command]
pub fn chat_history() -> Result<Vec<ChatMessage>, String> {
    with_db(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, role, content, traces, steps, model, time \
             FROM (SELECT *, rowid AS _rid FROM chat_messages ORDER BY created_at DESC, _rid DESC LIMIT ?1) \
             ORDER BY _rid ASC",
        )?;
        let rows = stmt.query_map([MAX_KEEP as i64], |r| {
            let traces_raw: String = r.get(3)?;
            let traces: Vec<serde_json::Value> =
                serde_json::from_str(&traces_raw).unwrap_or_default();
            Ok(ChatMessage {
                id: r.get(0)?,
                role: r.get(1)?,
                content: r.get(2)?,
                traces,
                steps: r.get(4)?,
                model: r.get(5)?,
                time: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
}

/// 新增或更新一条消息（按 id upsert）。工具轨迹随步骤实时更新也会走到这里。
#[tauri::command]
pub fn chat_upsert(msg: ChatMessage) -> Result<(), String> {
    let traces_raw = serde_json::to_string(&msg.traces).unwrap_or_else(|_| "[]".to_string());
    with_db(|conn| {
        conn.execute(
            "INSERT INTO chat_messages (id, role, content, traces, steps, model, time, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now')) \
             ON CONFLICT(id) DO UPDATE SET \
               content = excluded.content, traces = excluded.traces, \
               steps = excluded.steps, model = excluded.model, time = excluded.time",
            rusqlite::params![
                msg.id, msg.role, msg.content, traces_raw, msg.steps, msg.model, msg.time
            ],
        )?;
        // 只保留最近 MAX_KEEP 条，防止无限增长
        conn.execute(
            "DELETE FROM chat_messages WHERE id NOT IN \
             (SELECT id FROM chat_messages ORDER BY created_at DESC, rowid DESC LIMIT ?1)",
            rusqlite::params![MAX_KEEP as i64],
        )?;
        Ok(())
    })
}

/// 清空全部对话历史
#[tauri::command]
pub fn chat_clear() -> Result<(), String> {
    with_db(|conn| {
        conn.execute("DELETE FROM chat_messages", [])?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_roundtrip_with_traces() {
        let msg = ChatMessage {
            id: "a-123".into(),
            role: "assistant".into(),
            content: "已完成".into(),
            traces: vec![serde_json::json!({"step":1,"tool":"notify","status":"done"})],
            steps: Some(2),
            model: Some("qwen2.5:3b".into()),
            time: "12:30".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, msg.id);
        assert_eq!(back.traces.len(), 1);
        assert_eq!(back.steps, Some(2));
        assert_eq!(back.model.as_deref(), Some("qwen2.5:3b"));
    }

    #[test]
    fn chat_message_defaults_for_legacy() {
        // 老数据缺 traces/steps/model/time 时应有默认值
        let back: ChatMessage = serde_json::from_str(r#"{"id":"x","role":"user","content":"hi"}"#).unwrap();
        assert!(back.traces.is_empty());
        assert!(back.steps.is_none());
        assert!(back.model.is_none());
        assert_eq!(back.time, "");
    }
}
