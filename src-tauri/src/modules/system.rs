//! Module for system monitoring (CPU, memory, processes)
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct CpuInfo {
    pub usage_percent: f32,
    pub core_count: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct MemoryInfo {
    pub total_mb: u64,
    pub used_mb: u64,
    pub free_mb: u64,
    pub used_percent: f32,
}

#[derive(Serialize, Clone, Debug)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub available_gb: f64,
    pub used_percent: f32,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub status: String,
}

/// Get CPU info
#[tauri::command]
pub fn get_cpu_info() -> Result<CpuInfo, String> {
    Ok({
        let mut sys = sysinfo::System::new_all();
        sys.refresh_cpu_usage();
        CpuInfo {
            usage_percent: sys.global_cpu_usage(),
            core_count: sys.physical_core_count().unwrap_or(0),
        }
    })
}

/// Get memory info
#[tauri::command]
pub fn get_memory_info() -> Result<MemoryInfo, String> {
    Ok({
        let sys = sysinfo::System::new_all();
        let total = sys.total_memory();
        let free = sys.free_memory();
        let used = total - free;
        MemoryInfo {
            total_mb: total / (1024 * 1024),
            used_mb: used / (1024 * 1024),
            free_mb: free / (1024 * 1024),
            used_percent: if total > 0 { used as f32 / total as f32 * 100.0 } else { 0.0 },
        }
    })
}

/// Get disk info for root partition
#[tauri::command]
pub fn get_disk_info() -> Result<Vec<DiskInfo>, String> {
    use sysinfo::Disks;
    Ok(Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| d.mount_point() == std::path::Path::new("/"))
        .map(|d| {
            let total = d.total_space();
            let available = d.available_space();
            DiskInfo {
                name: d.name().to_string_lossy().to_string(),
                mount_point: format!("{:?}", d.mount_point()),
                total_gb: total as f64 / 1_000_000_000.0,
                available_gb: available as f64 / 1_000_000_000.0,
                used_percent: if total > 0 { (total - available) as f32 / total as f32 * 100.0 } else { 0.0 },
            }
        })
        .collect())
}

/// Get top processes sorted by memory
#[tauri::command]
pub fn get_processes(limit: u32) -> Result<Vec<ProcessInfo>, String> {
    let limit = limit as usize;
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .filter(|p| !p.name().is_empty())
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_percent: p.cpu_usage(),
            memory_mb: p.memory() / (1024 * 1024),
            status: format!("{:?}", p.status()),
        })
        .collect();
    procs.sort_by(|a, b| b.memory_mb.cmp(&a.memory_mb));
    procs.truncate(limit);
    Ok(procs)
}

/// Get combined system info for dashboard
#[tauri::command]
pub async fn get_system_info() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(|| {
        let cpu = get_cpu_info()?;
        let mem = get_memory_info()?;
        let disks = get_disk_info()?;
        Ok(serde_json::json!({
            "cpu": cpu,
            "memory": mem,
            "disks": disks
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}
