//! AI-powered memory system — intent analysis + semantic search via Ollama
use serde::{Deserialize, Serialize};
use crate::db::with_db;
use rusqlite::Result as SqlResult;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MemoryItem {
    pub id: i64,
    pub content: String,
    pub tag: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct IntentJson {
    tag: Option<String>,
    summary: Option<String>,
    #[allow(dead_code)]
    keywords: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SimilarityResult {
    pub id: i64,
    pub content: String,
    pub score: f32,
}

fn ollama_url() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

const INTENT_PROMPT: &str = r#"Analyze the following text and return a JSON object with these fields only:
- "tag": one of: meeting, idea, code, personal, todo
- "summary": one short sentence summarizing the content (max 60 chars)
- "keywords": array of 2-4 relevant keywords (short words)

Only respond with valid JSON, no other text.

Text: {{TEXT}}"#;

// ─── CRUD Commands ──────────────────────────────────────────────

#[tauri::command]
pub async fn mem_list() -> Result<Vec<MemoryItem>, String> {
    tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, content, COALESCE(tag,''), COALESCE(summary,''), created_at \
                 FROM memories ORDER BY created_at DESC LIMIT 50",
            )?;
            let rows = stmt.query_map([], |row| Ok(MemoryItem {
                id: row.get(0)?,
                content: row.get(1)?,
                tag: match row.get::<_, Option<String>>(2) {
                    Ok(t) => t,
                    Err(_) => None,
                },
                summary: match row.get::<_, Option<String>>(3) {
                    Ok(s) => s,
                    Err(_) => None,
                },
                created_at: row.get(4)?,
            }))?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn mem_add(content: String, tag: Option<String>, summary: Option<String>) -> Result<i64, String> {
    tokio::task::spawn_blocking(move || {
        let ts = chrono::Utc::now().to_rfc3339();
        with_db(move |conn| {
            let id = conn.execute(
                "INSERT INTO memories (content, created_at, tag, summary) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![&content, &ts, &tag, &summary],
            )?;
            Ok(id as i64)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn mem_delete(id: i64) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        with_db(move |conn| {
            conn.execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])?;
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn mem_analyze(text: String) -> Result<IntentJson, String> {
    let prompt = INTENT_PROMPT.replace("{{TEXT}}", &text);
    let url = format!("{}/api/generate", ollama_url());
    let body = serde_json::json!({
        "model": "qwen2.5:3b",
        "prompt": prompt,
        "stream": false,
        "options": { "temperature": 0.1, "num_predict": 200 }
    });

    let resp = crate::modules::local_http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ollama request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("ollama HTTP error: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await
        .map_err(|e| format!("parse response: {}", e))?;

    let raw_text: String = json["response"]
        .as_str()
        .ok_or("no 'response' field in ollama output")?
        .to_string();

    parse_intent_from_llm(&raw_text)
}

fn parse_intent_from_llm(s: &str) -> Result<IntentJson, String> {
    let stripped = s.trim()
        .trim_start_matches('`')
        .trim_start_matches("json");
    let json_str = if stripped.ends_with("```") {
        stripped[..stripped.len()-3].trim()
    } else {
        stripped
    };
    serde_json::from_str(json_str)
        .map_err(|e| format!("LLM returned invalid JSON: {} | raw: {:?}", e, json_str))
}

// ─── Embedding-based Semantic Search ────────────────────────────

async fn embed_text(text: &str) -> Result<Vec<f32>, String> {
    // Ollama 新版端点 /api/embed：请求用 input（可字符串或数组），响应 embeddings 为二维数组。
    // 旧端点 /api/embeddings 已废弃（实测返回 404），body 用 prompt / 响应为单个 embedding。
    let url = format!("{}/api/embed", ollama_url());
    let body = serde_json::json!({
        "model": "bge-m3",
        "input": text
    });
    let resp = crate::modules::local_http_client()
        .post(&url)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{}", e))?;

    if !resp.status().is_success() {
        return Err(format!("embed error HTTP {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await
        .map_err(|e| format!("embed parse: {}", e))?;

    // 新端点：{"embeddings": [[...]]}；兼容旧端点：{"embedding": [...]}
    let values: Option<&Vec<serde_json::Value>> = json.get("embeddings")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_array())
        .or_else(|| json.get("embedding").and_then(|v| v.as_array()));
    let arr = values.ok_or("missing 'embeddings' field".to_string())?;
    Ok(arr.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x,y)| x*y).sum();
    let na: f32 = a.iter().map(|x| x*x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x*x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

#[tauri::command]
pub async fn mem_search(query: String, top_n: Option<usize>) -> Result<Vec<SimilarityResult>, String> {
    let limit = top_n.unwrap_or(10);
    let memories: Vec<(i64, String)> = tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, content FROM memories ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<SqlResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    let query_vec = embed_text(&query).await?;

    let mut scored: Vec<SimilarityResult> = Vec::new();
    for (id, content) in memories {
        let sample: String = if content.chars().count() > 500 {
            content.chars().take(500).collect()
        } else { content.clone() };
        if let Ok(mem_vec) = embed_text(&sample).await { scored.push(SimilarityResult {
            id,
            content,
            score: cosine_sim(&query_vec, &mem_vec),
        }) }
    }

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored)
}
