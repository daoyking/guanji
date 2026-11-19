pub mod system;
pub mod file;
pub mod model;
pub mod tasks;
pub mod service;
pub mod clipboard;
pub mod memory;
pub mod notification;
pub mod agent;
pub mod shell;
pub mod reminder;
pub mod chat;
pub mod provider;
pub mod apps;

use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// 递归计算目录大小（字节）。max_depth 为 0 表示不限制。
/// 符号链接不跟随，避免循环。
pub fn dir_size_limited(path: &Path, max_depth: u32) -> u64 {
    fn walk(path: &Path, depth: u32, max_depth: u32) -> u64 {
        let mut size = 0u64;
        if path.is_file() {
            return fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }
        if !path.is_dir() {
            return 0;
        }
        let entries = match fs::read_dir(path) { Ok(e) => e, Err(_) => return 0 };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if max_depth == 0 || depth < max_depth {
                    size += walk(&p, depth + 1, max_depth);
                } else {
                    size += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                }
            } else if let Ok(m) = fs::metadata(&p) {
                size += m.len();
            }
        }
        size
    }
    walk(path, 0, max_depth)
}

/// 完整递归目录大小（无深度限制）
pub fn dir_size(path: &Path) -> u64 {
    dir_size_limited(path, 0)
}

/// 把文件或目录移到废纸篓（可恢复，不永久删除）。
/// 同名目标自动加时间戳后缀，避免覆盖已有内容。
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("无法定位用户主目录")?;
    let trash = home.join(".Trash");
    fs::create_dir_all(&trash).map_err(|e| format!("无法访问废纸篓: {}", e))?;

    let name = path.file_name().and_then(|n| n.to_str())
        .ok_or("无法获取文件名")?;
    let mut dest = trash.join(name);
    if dest.exists() {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        dest = trash.join(format!("{}.{}", name, stamp));
    }
    fs::rename(path, &dest).map_err(|e| {
        if e.raw_os_error() == Some(18) {
            "跨磁盘移动失败，请手动处理".into()
        } else {
            format!("移动失败: {}", e)
        }
    })
}

/// 本机 HTTP 客户端：显式禁用系统/环境代理。
/// reqwest::Client::new() 默认读取 HTTP_PROXY / macOS 系统代理，
/// 会把发往 localhost:11434 等本机服务的请求转给代理，导致 502 Bad Gateway。
/// 所有访问本机 Ollama / 本地服务的请求都应使用它。
pub fn local_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
