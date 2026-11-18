//! Module for file management (large files, cleanup, duplicates)

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Serialize, Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub size_mb: f64,
    pub modified: String,
}

const MB: u64 = 1_000_000u64;

fn dir_size(path: &Path) -> Result<u64, std::io::Error> {
    let mut size = 0u64;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                size += dir_size(&p)?;
            } else if let Ok(meta) = fs::metadata(&p) {
                size += meta.len();
            }
        }
    }
    Ok(size)
}

async fn collect_files(dir: &Path, threshold: u64, results: &mut Vec<FileEntry>) {
    if !dir.exists() || !dir.is_dir() { return; }
    let entries = match fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.components().count() - dir.components().count() < 3 {
                Box::pin(collect_files(&path, threshold, results)).await;
            }
        } else if let Ok(meta) = fs::metadata(&path) {
            if meta.len() >= threshold {
                results.push(FileEntry {
                    path: path.to_string_lossy().to_string(),
                    name: entry.file_name().to_string_lossy().to_string(),
                    size_mb: meta.len() as f64 / MB as f64,
                    modified: format!("{:?}", meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
                });
            }
        }
    }
}

/// Scan Downloads for large files
#[tauri::command]
pub async fn scan_downloads(threshold_mb: Option<f64>) -> Result<Vec<FileEntry>, String> {
    let threshold = (threshold_mb.unwrap_or(100.0) * MB as f64) as u64;
    // dirs::download_dir() 已指向 ~/Downloads，切勿再 join("Downloads") 造成嵌套
    let download_dir = dirs::download_dir().ok_or("Cannot find Downloads directory")?;
    let mut results = Vec::new();
    collect_files(&download_dir, threshold, &mut results).await;
    results.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap());
    Ok(results)
}

/// List large files across common user directories
#[tauri::command]
pub async fn list_large_files(threshold_mb: f64, limit: usize) -> Result<Vec<FileEntry>, String> {
    let threshold = (threshold_mb * MB as f64) as u64;
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let dirs = vec![
        home.join("Downloads"),
        home.join("Desktop"),
        home.join("Documents"),
        home.join("Videos"),
        home.join("Music"),
    ];
    let mut results = Vec::new();
    for dir in &dirs {
        if dir.exists() { collect_files(dir, threshold, &mut results).await; }
    }
    results.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap());
    results.truncate(limit);
    Ok(results)
}

/// Clean temp/cache files and report what could be freed
#[tauri::command]
pub async fn clean_temp_files() -> Result<serde_json::Value, String> {
    let mut freed_bytes = 0u64;
    let dirs_to_scan = vec![
        ("cache_tencent", dirs::cache_dir().map(|d| d.join("Tencent"))),
        ("cache_home", dirs::home_dir().map(|d| d.join(".cache"))),
        ("temp", None),
    ];
    for (_label, opt) in dirs_to_scan {
        if let Some(path) = opt {
            if path.exists() && path.is_dir() {
                if let Ok(size) = dir_size(&path) { freed_bytes += size; }
            }
        }
    }
    Ok(serde_json::json!({
        "freed_bytes": freed_bytes,
        "cleaned_count": 0,
        "message": format!("扫描到 {} MB 可清理的缓存文件（未实际删除，请手动确认）", freed_bytes / 1_000_000)
    }))
}

/// Find duplicate files by size in given paths
#[tauri::command]
pub async fn find_duplicates(paths: Vec<String>) -> Result<Vec<Vec<FileEntry>>, String> {
    let mut size_map: std::collections::HashMap<u64, Vec<(String, String)>> = std::collections::HashMap::new();
    for path_str in paths {
        let path = PathBuf::from(&path_str);
        if path.exists() && path.is_dir() {
            let mut stack = vec![path.clone()];
            while let Some(current) = stack.pop() {
                if let Ok(entries) = fs::read_dir(&current) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() { 
                            if current.components().count() < 5 {
                                stack.push(p.clone());
                            }
                        } else if let Ok(meta) = fs::metadata(&p) {
                            size_map.entry(meta.len()).or_default()
                                .push((meta.len().to_string(), p.to_string_lossy().to_string()));
                        }
                    }
                }
            }
        }
    }
    let mut groups: Vec<Vec<FileEntry>> = size_map.into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(_, files)| files.iter().map(|(size_str, path)| {
            let size_mb = size_str.parse::<f64>().unwrap_or(0.0) / MB as f64;
            FileEntry {
                path: path.clone(),
                name: Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                size_mb,
                modified: "".to_string(),
            }
        }).collect())
        .collect();
    groups.sort_by(|a, b| b[0].size_mb.partial_cmp(&a[0].size_mb).unwrap());
    Ok(groups)
}
