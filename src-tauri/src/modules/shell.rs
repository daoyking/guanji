//! 管家的「执行类」工具：终端命令、打开目标、文本文件读写。
//! 内置安全护栏：危险命令拦截、写保护目录、输出与大小上限。
use serde_json::json;
use std::path::PathBuf;
use std::process::Command as StdCommand;

/// 执行 shell 命令的安全黑名单（小写匹配，命中即拒绝）
const DENIED_PATTERNS: &[&str] = &[
    "rm -rf /", "rm -rf ~", "rm -fr /", "rm -fr ~", // 根目录/主目录递归删除
    "sudo ", "su -", "mkfs", "dd if=", "diskutil erase",
    ":(){", ":(){ :", // fork bomb
    "shutdown", "reboot", "halt -", "killall -9 kernel",
    "chmod 777 /", "chown -r root",
];

/// 只允许写入的用户根目录下的路径前缀（防止破坏系统）
fn is_write_allowed(path: &PathBuf) -> bool {
    let s = path.to_string_lossy().to_string();
    let denied = ["/System", "/usr", "/bin", "/sbin", "/etc", "/var", "/Library"];
    !denied.iter().any(|p| s == *p || s.starts_with(&format!("{}/", p)))
}

fn is_dangerous(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    DENIED_PATTERNS.iter().find(|p| lower.contains(**p)).copied()
}

/// 执行一条 shell 命令，返回结构化结果（含危险拦截）
#[tauri::command]
pub async fn run_command(command: String, cwd: Option<String>) -> Result<String, String> {
    if command.trim().is_empty() {
        return Err("命令不能为空".to_string());
    }
    if let Some(p) = is_dangerous(&command) {
        return Err(format!("安全拦截：命令命中危险模式「{}」，已拒绝执行", p));
    }

    let mut cmd = StdCommand::new("sh");
    cmd.arg("-c").arg(&command);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // 在阻塞线程池执行，避免卡住 async runtime
    let out = tokio::task::spawn_blocking(move || cmd.output()).await
        .map_err(|e| format!("命令任务失败: {}", e))?
        .map_err(|e| format!("命令启动失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let ok = out.status.success();
    let combined = if ok { stdout } else { format!("{}\n[stderr] {}", stdout, stderr) };
    let combined = if combined.len() > 4000 {
        format!("{}…（已截断，共{}字符）", &combined[..4000], combined.len())
    } else { combined };

    Ok(json!({
        "ok": ok,
        "exit_code": out.status.code().unwrap_or(-1),
        "output": combined
    }).to_string())
}

/// 用系统默认程序打开应用 / 文件 / URL（macOS `open`）
#[tauri::command]
pub async fn open_target(target: String) -> Result<String, String> {
    if target.trim().is_empty() {
        return Err("目标不能为空".to_string());
    }
    let t = target.clone();
    let out = tokio::task::spawn_blocking(move || StdCommand::new("open").arg(&t).output()).await
        .map_err(|e| format!("打开任务失败: {}", e))?
        .map_err(|e| format!("打开失败: {}", e))?;
    if out.status.success() {
        Ok(format!("已打开：{}", target))
    } else {
        Err(format!("打开失败：{}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// 读取文本文件（限制 100KB，拒绝二进制）
#[tauri::command]
pub async fn read_text_file(path: String, max_chars: Option<usize>) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if !p.exists() { return Err(format!("文件不存在：{}", path)); }
    if !p.is_file() { return Err(format!("不是文件：{}", path)); }
    let meta = std::fs::metadata(&p).map_err(|e| e.to_string())?;
    if meta.len() > 100 * 1024 { return Err("文件超过 100KB，请使用专门工具处理大文件".to_string()); }
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| "文件不是 UTF-8 文本，无法读取".to_string())?;
    let limit = max_chars.unwrap_or(8000);
    if text.len() > limit {
        Ok(format!("{}…（已截断，共{}字符）", &text[..limit], text.len()))
    } else { Ok(text) }
}

/// 写入文本文件（拒绝系统目录，自动创建父目录，可选追加）
#[tauri::command]
pub async fn write_text_file(path: String, content: String, append: Option<bool>) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if !is_write_allowed(&p) {
        return Err(format!("安全拦截：不允许写入系统目录 {}", path));
    }
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
    }
    use std::io::Write;
    let result = if append.unwrap_or(false) {
        std::fs::OpenOptions::new().create(true).append(true).open(&p)
            .and_then(|mut f| f.write_all(content.as_bytes()))
    } else {
        std::fs::write(&p, content.as_bytes())
    };
    result.map_err(|e| format!("写入失败: {}", e))?;
    Ok(format!("已写入 {} 字节到 {}", content.len(), path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_dangerous_commands() {
        assert!(is_dangerous("rm -rf /").is_some());
        assert!(is_dangerous("sudo rm -rf /tmp/x").is_some());
        assert!(is_dangerous("mkfs.ext4 /dev/disk0").is_some());
        assert!(is_dangerous("echo hello").is_none());
        assert!(is_dangerous("ls -la ~").is_none());
    }

    #[test]
    fn protects_system_write_paths() {
        assert!(!is_write_allowed(&PathBuf::from("/System/Library/x")));
        assert!(!is_write_allowed(&PathBuf::from("/usr/local/bin/x")));
        assert!(is_write_allowed(&PathBuf::from("/Users/jindy/notes.md")));
        assert!(is_write_allowed(&PathBuf::from("/tmp/guanji-test.txt")));
    }
}
