//! Module for Ollama model management (list / pull / delete)
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,           // bytes
    pub digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ModelDetails>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization_level: Option<String>,
}

/// Get base URL for Ollama from env or default to localhost
fn ollama_url() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

/// List all locally installed Ollama models
#[tauri::command]
pub async fn list_ollama_models() -> Result<Vec<OllamaModel>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/tags", ollama_url());
    match client.get(&url).send().await {
        Ok(res) => {
            let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
            let models_arr = body
                .get("models")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let out: Vec<OllamaModel> = serde_json::from_value(serde_json::Value::Array(models_arr))
                .map_err(|e| e.to_string())?;
            Ok(out)
        }
        Err(e) => Err(format!("Failed to reach Ollama: {}", e)),
    }
}

/// Pull a model by name (e.g. "qwen2.5:7b") — returns last progress line
#[tauri::command]
pub async fn pull_ollama_model(name: String) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/pull", ollama_url());
    let body = serde_json::json!({ "name": name, "stream": false });
    match client.post(&url).json(&body).send().await {
        Ok(res) => {
            let text = res.text().await.map_err(|e| e.to_string())?;
            // Parse final status line
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                let status = val.get("status").map(|s| s.as_str().unwrap_or("done")).unwrap_or("done");
                Ok(format!("Pull completed: {}", status))
            } else {
                Ok(text.chars().take(200).collect())
            }
        }
        Err(e) => Err(format!("Pull failed: {}", e)),
    }
}

/// Delete a model by name
#[tauri::command]
pub async fn delete_ollama_model(name: String) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/delete", ollama_url());
    let body = serde_json::json!({ "name": name });
    match client.delete(&url).json(&body).send().await {
        Ok(_) => Ok(format!("Deleted model: {}", name)),
        Err(e) => Err(format!("Delete failed: {}", e)),
    }
}
