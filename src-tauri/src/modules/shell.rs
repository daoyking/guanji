//! 管家的「执行类」工具：终端命令、打开目标、文本文件读写。
//! 内置安全护栏：危险命令拦截、写保护目录、输出与大小上限。
use serde_json::json;
use std::path::PathBuf;
use std::process::Command as StdCommand;

/// 执行 shell 命令的安全黑名单（小写匹配，命中即拒绝）—— 第一层粗筛。
/// 注意：不在此放 `rm -rf /` 这类模式——contains 匹配会误伤 `/Users/...`。
/// 递归删除的目标路径由 check_destructive 做结构化判定。
const DENIED_PATTERNS: &[&str] = &[
    "sudo ", "su -", "mkfs", "dd if=", "diskutil erase",
    ":(){", ":(){ :", // fork bomb
    "shutdown", "reboot", "halt -", "killall -9 kernel",
    "chmod 777 /", "chown -r root",
];

/// 破坏性命令词：出现在任一子命令里即进入严格审查。
const DESTRUCTIVE_BINS: &[&str] = &[
    "rm", "rmdir", "unlink", "shred", "truncate",
    "mkfs", "dd", "diskutil", "chown", "chmod",
];

/// 把 shell 命令按分隔符切成子命令，用于识别 `echo ok; rm -rf ~` 这类夹带。
fn split_subcommands(command: &str) -> Vec<String> {
    let normalized = command.replace('\n', ";");
    normalized
        .split(|c: char| c == ';' || c == '|' || c == '&')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 展开常见的 home 变量写法，便于路径判定（不执行，仅字符串替换）。
/// 输入应为小写字符串（命令已 lowercased）。
fn expand_home_vars(s: &str) -> String {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "/users/unknown".to_string());
    s.replace("${home}", &home)
        .replace("$home", &home)
        .replace("~", &home)
}

/// 去掉引号，取路径 token。
fn unquote(token: &str) -> String {
    token.trim().trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// 规范化路径：合并 . 和 ..，统一小写。
fn normalize_path(s: &str) -> String {
    std::path::Path::new(s)
        .components()
        .fold(std::path::PathBuf::new(), |mut acc, c| {
            use std::path::Component::*;
            match c {
                RootDir => acc.push("/"),
                CurDir => {}
                ParentDir => { acc.pop(); }
                Normal(x) => acc.push(x),
                Prefix(x) => acc.push(x.as_os_str()),
            }
            acc
        })
        .to_string_lossy()
        .to_lowercase()
}

/// 判断一个受删除目标是否命中保护名单。
fn is_protected_target(norm: &str) -> bool {
    if norm == "/" || norm == "/users" || norm == "/home" {
        return true;
    }
    // 用户主目录本身（精确匹配，不含其子目录）
    if let Some(home) = dirs::home_dir() {
        if norm == home.to_string_lossy().to_lowercase().trim_end_matches('/') {
            return true;
        }
    }
    // 系统级目录及其子路径
    ["/system", "/usr", "/bin", "/sbin", "/etc", "/var", "/library", "/volumes", "/private"]
        .iter()
        .any(|p| norm == *p || norm.starts_with(&format!("{}/", p)))
}

/// 判断一个破坏性子命令是否安全。返回 Some(原因) 表示应拒绝。
fn check_destructive(sub: &str) -> Option<String> {
    let lower = sub.to_lowercase();
    // 命令替换 / 管道注入：破坏性命令里出现 $( ) / 反引号 / <( 一律拒绝
    if lower.contains("$(") || lower.contains('`') || lower.contains("<(") || lower.contains(">(") {
        return Some("破坏性命令中不允许命令替换或进程替换".to_string());
    }

    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let bin = *tokens.first().unwrap_or(&"");
    // 允许 /bin/rm、/usr/bin/rm 等带路径的可执行文件
    let bin_name = bin.rsplit('/').next().unwrap_or(bin);
    if !DESTRUCTIVE_BINS.contains(&bin_name) {
        return None;
    }

    // dd / diskutil / chmod / chown 已由粗黑名单覆盖大部分，这里只校验 rm 系的路径目标
    if !matches!(bin_name, "rm" | "rmdir" | "unlink" | "shred" | "truncate") {
        return None;
    }

    // 解析递归标志：rm -rf / -fr / -R / -r 都算递归；rmdir 天然递归删空目录。
    // 支持 -rf / -fr / -rF 等组合，以及 --recursive。
    let recursive = bin_name == "rmdir"
        || tokens.iter().skip(1).any(|t| {
            if t.starts_with("--") {
                *t == "--recursive"
            } else if t.starts_with('-') {
                t[1..].contains('r') || t[1..].contains('R')
            } else {
                false
            }
        });

    for tok in tokens.iter().skip(1) {
        let t = unquote(tok);
        if t.starts_with('-') {
            continue; // 跳过选项
        }
        let expanded = expand_home_vars(&t);
        let norm = normalize_path(&expanded);
        let is_root_del = recursive && is_protected_target(&norm);
        if is_root_del {
            return Some(format!("拒绝递归删除受保护路径：{}", t));
        }
    }
    None
}

fn is_dangerous(command: &str) -> Option<String> {
    let lower = command.to_lowercase();
    if let Some(p) = DENIED_PATTERNS.iter().find(|p| lower.contains(**p)) {
        return Some(format!("命令命中危险模式「{}」", p));
    }
    for sub in split_subcommands(command) {
        if let Some(reason) = check_destructive(&sub) {
            return Some(reason);
        }
    }
    None
}

/// 只允许写入的用户根目录下的路径前缀（防止破坏系统）
fn is_write_allowed(path: &PathBuf) -> bool {
    let s = path.to_string_lossy().to_string();
    let denied = ["/System", "/usr", "/bin", "/sbin", "/etc", "/var", "/Library"];
    !denied.iter().any(|p| s == *p || s.starts_with(&format!("{}/", p)))
}

/// 执行一条 shell 命令，返回结构化结果（含危险拦截）
#[tauri::command]
pub async fn run_command(command: String, cwd: Option<String>) -> Result<String, String> {
    if command.trim().is_empty() {
        return Err("命令不能为空".to_string());
    }
    if let Some(p) = is_dangerous(&command) {
        return Err(format!("安全拦截：{}，已拒绝执行", p));
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
    let (combined, _) = crate::modules::truncate_chars(&combined, 4000);

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
    let (cut, total) = crate::modules::truncate_chars(&text, limit);
    if total > limit { Ok(cut) } else { Ok(text) }
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
    fn blocks_bypass_attempts() {
        // 变量展开
        assert!(is_dangerous("rm -rf $HOME").is_some());
        assert!(is_dangerous("rm -rf \"$HOME\"").is_some());
        assert!(is_dangerous("rm -rf ${HOME}").is_some());
        // 带路径的可执行文件
        assert!(is_dangerous("/bin/rm -rf /").is_some());
        // 分号/管道夹带
        assert!(is_dangerous("echo ok; rm -rf ~").is_some());
        assert!(is_dangerous("echo ok && rm -rf /Users").is_some());
        // 换行夹带
        assert!(is_dangerous("echo ok\nrm -rf /").is_some());
        // 命令替换
        assert!(is_dangerous("rm -rf $(echo /)").is_some());
        assert!(is_dangerous("rm -rf `echo /`").is_some());
        // 参数顺序 / flag 组合
        assert!(is_dangerous("rm -r -f /").is_some());
        assert!(is_dangerous("rm -fr /").is_some());
        assert!(is_dangerous("rm --recursive /").is_some());
        assert!(is_dangerous("/bin/rm -Rf ~").is_some());
    }

    #[test]
    fn allows_safe_destructive_commands() {
        // 用户目录内、非递归的普通删除应放行
        assert!(is_dangerous("rm /tmp/guanji-test.txt").is_none());
        assert!(is_dangerous("rm -rf /Users/jindy/Archive/GuanJia/target/debug junk").is_none());
        assert!(is_dangerous("rm -f ./dist/old.js").is_none());
    }

    #[test]
    fn protects_system_write_paths() {
        assert!(!is_write_allowed(&PathBuf::from("/System/Library/x")));
        assert!(!is_write_allowed(&PathBuf::from("/usr/local/bin/x")));
        assert!(is_write_allowed(&PathBuf::from("/Users/jindy/notes.md")));
        assert!(is_write_allowed(&PathBuf::from("/tmp/guanji-test.txt")));
    }
}
