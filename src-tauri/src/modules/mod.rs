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
use std::process::Command;
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

/// 用 AppleScript 让 Finder 把目标移到废纸篓。
/// 相比 fs::rename，Finder 走系统授权，能处理 root 属主应用、系统保护项、
/// 以及跨磁盘（外接卷）移动，是 macOS 上“移到废纸篓”的官方途径。
fn move_to_trash_via_finder(path: &Path) -> Result<(), String> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("无法获取当前目录: {}", e))?
            .join(path)
    };
    let raw = abs.to_string_lossy();
    // AppleScript 字符串转义：反斜杠与双引号
    let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("tell application \"Finder\" to delete POSIX file \"{}\"", escaped);
    let out = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("无法调用 Finder: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { "Finder 拒绝移动该项".into() } else { err })
    }
}

/// 把文件或目录移到废纸篓（可恢复，不永久删除）。
/// 优先用 fs::rename（快、无副作用）；遇到权限不足（os error 13）、
/// 跨磁盘（os error 18）等失败时，回退到 Finder，由系统处理授权与跨卷移动。
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
    match fs::rename(path, &dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            // 权限不足 / 跨磁盘 / 系统保护项：交给 Finder 走系统授权
            move_to_trash_via_finder(path).map_err(|finder_err| {
                format!("移动失败（{}）；Finder 兜底也失败：{}", e, finder_err)
            })
        }
    }
}

/// 按【字符数】安全截断字符串（UTF-8 中文按字节切片会 panic）。
/// 超长时追加省略标记，返回 (截断文本, 原字符数)。
pub fn truncate_chars(s: &str, max: usize) -> (String, usize) {
    let total = s.chars().count();
    if total <= max {
        return (s.to_string(), total);
    }
    let cut: String = s.chars().take(max).collect();
    (format!("{}…（已截断，共{}字符）", cut, total), total)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_handles_chinese_without_panic() {
        let s = "中".repeat(100);
        let (cut, total) = truncate_chars(&s, 10);
        assert_eq!(total, 100);
        assert!(cut.starts_with(&"中".repeat(10)));
        assert!(cut.contains("已截断"));
    }

    #[test]
    fn truncate_chars_short_string_unchanged() {
        let (cut, total) = truncate_chars("你好abc", 10);
        assert_eq!(cut, "你好abc");
        assert_eq!(total, 5);
    }
}
