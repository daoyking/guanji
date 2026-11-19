//! Service manager — health checks for local AI services
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceStatus {
    pub id: String,          // unique key: "ollama" | "orcarouter" | "chroma" | "searxng"
    pub name: String,        // display name
    pub url: String,         // base URL
    pub healthy: bool,       // last health check result
    pub mode: String,        // "probe_level_2" (HTTP body) / "probe_level_1" (TCP)
    pub icon: String,        // emoji icon
    pub detail: Option<String>, // e.g. "v0.40.15 · 5 models"
}

// ─── Service registry ───────────────────────────────────────────

fn default_services() -> Vec<ServiceStatus> {
    vec![
        ServiceStatus {
            id: "ollama".to_string(),
            name: "Ollama".to_string(),
            url: "http://localhost:11434".to_string(),
            healthy: false,
            mode: "probe_level_2".to_string(),
            icon: "🦙".to_string(),
            detail: None,
        },
        ServiceStatus {
            id: "orcarouter".to_string(),
            name: "OrcaRouter".to_string(),
            url: "http://localhost:7001".to_string(),
            healthy: false,
            mode: "probe_level_2".to_string(),
            icon: "🐋".to_string(),
            detail: None,
        },
        ServiceStatus {
            id: "chroma".to_string(),
            name: "ChromaDB".to_string(),
            url: "http://localhost:8100".to_string(),
            healthy: false,
            mode: "probe_level_2".to_string(),
            icon: "🧬".to_string(),
            detail: None,
        },
        ServiceStatus {
            id: "searxng".to_string(),
            name: "SearXNG".to_string(),
            url: "http://localhost:8081".to_string(),
            healthy: false,
            mode: "probe_level_2".to_string(),
            icon: "🔍".to_string(),
            detail: None,
        },
    ]
}

// ─── Health Check Commands ──────────────────────────────────────

#[tauri::command]
pub async fn list_services() -> Vec<ServiceStatus> {
    default_services()
}

#[tauri::command]
pub async fn probe_service(service_id: String) -> Result<ServiceStatus, String> {
    let url = match service_id.as_str() {
        "ollama" => "http://localhost:11434/api/version",
        "orcarouter" => "http://localhost:7001/",
        "chroma" => "http://localhost:8100/api/v1/info",
        "searxng" => "http://localhost:8081/search?q=test&format=json",
        _ => return Err(format!("Unknown service: {}", service_id)),
    };

    let mut result = match service_id.as_str() {
        "ollama" => ServiceStatus { id: service_id, name: "Ollama".into(), url: "http://localhost:11434".into(), ..default_services()[0].clone() },
        "orcarouter" => ServiceStatus { id: service_id, name: "OrcaRouter".into(), url: "http://localhost:7001".into(), ..default_services()[1].clone() },
        "chroma" => ServiceStatus { id: service_id, name: "ChromaDB".into(), url: "http://localhost:8100".into(), ..default_services()[2].clone() },
        "searxng" => ServiceStatus { id: service_id, name: "SearXNG".into(), url: "http://localhost:8081".into(), ..default_services()[3].clone() },
        _ => return Err(format!("Unknown service: {}", service_id)),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("reqwest build failed: {}", e))?;

    let resp = client.get(url).send().await;
    match resp {
        Ok(r) => {
            if r.status().is_success() {
                result.healthy = true;
                let text = r.text().await.unwrap_or_default();
                result.detail = Some(crate::modules::truncate_chars(&text, 80).0);
            } else {
                result.healthy = false;
                result.detail = Some(format!("status={}", r.status()));
            }
        }
        Err(e) => {
            result.healthy = false;
            result.detail = Some(format!("{}", e));
        }
    }

    Ok(result)
}

#[tauri::command]
pub async fn run_all_probes() -> Vec<ServiceStatus> {
    let ids = vec!["ollama".to_string(), "orcarouter".to_string(), "chroma".to_string(), "searxng".to_string()];
    let mut tasks = Vec::new();
    for id in ids {
        tasks.push(tokio::spawn(probe_service(id)));
    }
    let mut results = Vec::with_capacity(tasks.len());
    for t in tasks {
        results.push(t.await.unwrap_or_else(|_| Err("probe cancelled".to_string())).unwrap_or_else(|e| {
            ServiceStatus {
                id: "unknown".to_string(),
                name: "?".into(),
                url: "".into(),
                healthy: false,
                mode: "error".into(),
                icon: "⚠️".into(),
                detail: Some(e),
            }
        }));
    }
    results.sort_by_key(|s| s.id.clone());
    results
}
