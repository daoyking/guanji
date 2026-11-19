//! 云模型服务商管理（OpenAI 兼容协议）。
//! 本机 Ollama 作为内置 provider（ID = "ollama"）特殊处理，不入库。
use serde::{Deserialize, Serialize};
use crate::db::with_db;

pub const LOCAL_PROVIDER_ID: &str = "ollama";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
}

/// 规范化用户填写的 Base URL：补协议、去尾部斜杠。
/// 用户常填 `api.deepseek.com` 或 `https://api.deepseek.com/v1/`，需统一。
pub fn normalize_base_url(input: &str) -> String {
    let mut s = input.trim().to_string();
    if s.is_empty() {
        return s;
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        s = format!("https://{}", s);
    }
    while s.ends_with('/') {
        s.pop();
    }
    s
}

/// 拼出具体的 API 端点。base_url 应指向 OpenAI 兼容根（含 /v1 或厂商自定义路径）。
pub fn api_url(base_url: &str, endpoint: &str) -> String {
    format!("{}/{}", normalize_base_url(base_url), endpoint.trim_start_matches('/'))
}

/// 云请求客户端：走系统代理（与本机 Ollama 的 no_proxy 相反）。
pub fn cloud_http_client() -> reqwest::Client {
    reqwest::Client::new()
}

fn row_to_provider(row: &rusqlite::Row) -> rusqlite::Result<Provider> {
    Ok(Provider {
        id: row.get(0)?,
        name: row.get(1)?,
        base_url: row.get(2)?,
        api_key: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
    })
}

/// 列出所有云 provider（不含内置 Ollama）
#[tauri::command]
pub fn provider_list() -> Result<Vec<Provider>, String> {
    with_db(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, base_url, api_key, enabled FROM model_providers ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map([], row_to_provider)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
}

fn new_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("p{}_{}", ts, std::process::id())
}

/// 添加/更新云 provider。id 为空则新建，否则更新
#[tauri::command]
pub fn provider_save(
    id: Option<String>,
    name: String,
    base_url: String,
    api_key: String,
    enabled: bool,
) -> Result<Provider, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("服务商名称不能为空".into());
    }
    let base_url = normalize_base_url(&base_url);
    if base_url.is_empty() {
        return Err("Base URL 不能为空".into());
    }
    let pid = id.filter(|s| !s.trim().is_empty()).unwrap_or_else(new_id);
    let p = Provider { id: pid.clone(), name, base_url, api_key, enabled };
    let saved = with_db(|conn| {
        conn.execute(
            "INSERT INTO model_providers (id, name, base_url, api_key, enabled) VALUES (?1,?2,?3,?4,?5) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, base_url=excluded.base_url, \
             api_key=excluded.api_key, enabled=excluded.enabled",
            rusqlite::params![p.id, p.name, p.base_url, p.api_key, if p.enabled { 1i64 } else { 0i64 }],
        )?;
        Ok(())
    });
    saved.map_err(|e| format!("保存服务商失败：{}", e))?;
    Ok(p)
}

#[tauri::command]
pub fn provider_delete(id: String) -> Result<String, String> {
    if id == LOCAL_PROVIDER_ID {
        return Err("内置 Ollama 不可删除".into());
    }
    with_db(|conn| {
        conn.execute("DELETE FROM model_providers WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    })
    .map_err(|e| format!("删除服务商失败：{}", e))?;
    Ok(format!("已删除服务商 {}", id))
}

/// 从 OpenAI 兼容的 /models 端点拉取模型 ID 列表
pub async fn fetch_cloud_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let url = api_url(base_url, "models");
    let mut req = cloud_http_client().get(&url).timeout(std::time::Duration::from_secs(20));
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }
    let resp = req.send().await.map_err(|e| format!("无法连接 {}：{}", url, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {}：{}", status, text.chars().take(160).collect::<String>()));
    }
    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("响应解析失败：{}", e))?;
    let ids: Vec<String> = body
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .collect();
    Ok(ids)
}

/// 列出某个云 provider 的模型（测试连接 + 拉列表）
#[tauri::command]
pub async fn provider_list_models(id: String) -> Result<Vec<String>, String> {
    let p = with_db(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, base_url, api_key, enabled FROM model_providers WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], row_to_provider)?;
        match rows.next() {
            Some(r) => r,
            None => Err(rusqlite::Error::QueryReturnedNoRows),
        }
    })
    .map_err(|_| format!("服务商 {} 不存在", id))?;
    fetch_cloud_models(&p.base_url, &p.api_key).await
}

/// 测试 provider 连通性：拉 /models，返回模型数量
#[tauri::command]
pub async fn provider_test(id: String) -> Result<String, String> {
    let ids = provider_list_models(id).await?;
    Ok(format!("连接成功，发现 {} 个模型", ids.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_scheme_and_trims_slash() {
        assert_eq!(normalize_base_url("api.deepseek.com/"), "https://api.deepseek.com");
        assert_eq!(normalize_base_url("https://api.openai.com/v1/"), "https://api.openai.com/v1");
        assert_eq!(normalize_base_url("  http://localhost:7001/v1  "), "http://localhost:7001/v1");
    }

    #[test]
    fn api_url_joins_correctly() {
        assert_eq!(
            api_url("https://api.deepseek.com/v1", "chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            api_url("https://api.deepseek.com/v1/", "/models"),
            "https://api.deepseek.com/v1/models"
        );
    }
}
