//! Application manager: scan installed apps, launch, clean app data, uninstall.
//! All destructive operations move to Trash (recoverable), never permanent delete.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Serialize, Clone, Debug)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size_mb: f64,
    pub category: String,
}

const MB: u64 = 1_000_000;

/// 应用候选目录：系统 + 用户。只取一级 .app，不递归。
fn app_directories() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    dirs
}

/// 一次调用 PlistBuddy 读取多个 key，避免每个应用启动 2-3 次进程（43 个应用实测 20s → 应大幅降低）。
fn read_plist(app: &Path) -> (String, String) {
    let plist = app.join("Contents/Info.plist");
    let fallback_name = app.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if !plist.exists() {
        return (fallback_name, String::new());
    }
    let script = "Print :CFBundleIdentifier; Print :CFBundleDisplayName; Print :CFBundleName";
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", script])
        .arg(&plist)
        .output();
    let stdout = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return (fallback_name, String::new()),
    };
    // PlistBuddy 出错时会在 stderr 输出但 stdout 可能仍有部分值，逐行解析成功取到的值
    let lines: Vec<&str> = stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let bundle = lines.first().map(|s| s.to_string()).unwrap_or_default();
    let display = lines.get(1).map(|s| s.to_string()).unwrap_or_default();
    let name = lines.get(2).map(|s| s.to_string()).unwrap_or_default();
    (if !display.is_empty() { display } else if !name.is_empty() { name } else { fallback_name }, bundle)
}

fn categorize(bundle_id: &str, name: &str) -> String {
    let s = format!("{} {}", bundle_id.to_lowercase(), name.to_lowercase());
    let has = |keys: &[&str]| keys.iter().any(|k| s.contains(k));

    if has(&["wechat", "weixin", "tencent", "qq", "telegram", "slack", "discord", "zoom", "feishu", "dingtalk", "message", "mail", "chat", "whatsapp", "微信", "钉钉", "飞书", "QQ"]) {
        "Communication".into()
    } else if has(&["xcode", "vscode", "visual studio", "terminal", "iterm", "git", "jetbrains", "goland", "pycharm", "sublime", "atom", "docker", "developer", "code"]) {
        "Development".into()
    } else if has(&["chrome", "safari", "firefox", "edge", "browser", "browser", "netflix", "spotify", "music", "video", "game", "steam", "bilibili", "youtube", "player"]) {
        "Entertainment".into()
    } else if has(&["office", "word", "excel", "powerpoint", "notion", "obsidian", "note", "calendar", "wps", "figma", "sketch", "pdf", "typora"]) {
        "Productivity".into()
    } else if has(&["cleaner", "manager", "monitor", "utility", "tool", "download", "compress", "unarchiver"]) {
        "Utility".into()
    } else {
        "Other".into()
    }
}

/// 扫描已安装应用（系统 /Applications + ~/Applications）
#[tauri::command]
pub fn list_installed_apps() -> Result<Vec<AppEntry>, String> {
    let mut apps = Vec::new();
    for dir in app_directories() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue, // 目录不存在或无权限，跳过
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            let (name, bundle_id) = read_plist(&path);
            if name.is_empty() {
                continue;
            }
            apps.push(AppEntry {
                id: if bundle_id.is_empty() { name.clone() } else { bundle_id.clone() },
                name,
                path: path.to_string_lossy().to_string(),
                // 应用大小只用于展示，限制深度避免扫描 Xcode/VSCode 内几十万文件
                size_mb: crate::modules::dir_size_limited(&path, 3) as f64 / MB as f64,
                category: categorize(&bundle_id, &path.file_name()
                    .map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
            });
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(apps)
}

/// 打开应用（用系统 open，走 LaunchServices）
#[tauri::command]
pub fn launch_app(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("应用不存在".into());
    }
    Command::new("open")
        .arg(&p)
        .spawn()
        .map_err(|e| format!("启动失败: {}", e))?;
    Ok(())
}

/// 清理单个应用的用户数据（容器/缓存/偏好），移到废纸篓可恢复。
/// 只清理明确属于该应用且位于 ~/Library 下的目录，不碰应用本体。
#[tauri::command]
pub fn cleanup_app_data(path: String) -> Result<String, String> {
    let app = PathBuf::from(&path);
    if !app.exists() || !app.is_dir() {
        return Err("应用不存在".into());
    }
    let (app_name, bundle_id) = read_plist(&app);
    if bundle_id.is_empty() && app_name.is_empty() {
        return Err("无法识别应用标识".into());
    }

    let home = dirs::home_dir().ok_or("无法定位用户主目录")?;
    let library = home.join("Library");

    // 应用数据的标准位置，只取明确归属于该 app 的项
    let mut candidates: Vec<PathBuf> = vec![
        library.join("Containers").join(&bundle_id),
        library.join("Group Containers").join(&bundle_id),
        library.join("Application Support").join(&app_name),
        library.join("Application Support").join(&bundle_id),
        library.join("Caches").join(&bundle_id),
        library.join("Logs").join(&app_name),
        library.join("Saved Application State").join(&bundle_id),
    ];
    candidates.retain(|p| !bundle_id.is_empty() || p.file_name().and_then(|n| n.to_str()) == Some(app_name.as_str()));

    let mut freed = 0u64;
    let mut count = 0usize;
    let mut fails = Vec::new();
    for dir in candidates {
        if !dir.exists() || !dir.is_dir() {
            continue;
        }
        let size = crate::modules::dir_size(&dir);
        match crate::modules::move_to_trash(&dir) {
            Ok(_) => {
                freed += size;
                count += 1;
            }
            Err(e) => fails.push(format!("{}: {}", dir.file_name().unwrap_or_default().to_string_lossy(), e)),
        }
    }

    let msg = format!(
        "已把 {} 项应用数据移到废纸篓，释放约 {:.1} MB（可从废纸篓恢复）{}",
        count,
        freed as f64 / MB as f64,
        if fails.is_empty() { String::new() } else { format!("\n失败 {} 项：{}", fails.len(), fails.join("; ")) }
    );
    Ok(msg)
}

/// 卸载应用：把 .app 整体移到废纸篓（可恢复，不永久删除）。
/// 安全闸门：只允许 /Applications 与 ~/Applications 下的 .app。
#[tauri::command]
pub fn uninstall_app(path: String) -> Result<String, String> {
    let app = PathBuf::from(&path);
    if !app.exists() || !app.is_dir() || app.extension().and_then(|e| e.to_str()) != Some("app") {
        return Err("无效的应用路径".into());
    }
    let canon = app.canonicalize().map_err(|e| format!("无法解析路径: {}", e))?;
    let allowed = app_directories();
    let in_allowed = allowed.iter().any(|d| {
        d.canonicalize().map(|cd| canon.starts_with(cd)).unwrap_or(false)
    });
    if !in_allowed {
        return Err("出于安全考虑，只允许卸载 /Applications 或 ~/Applications 中的应用".into());
    }

    let name = canon.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let size = crate::modules::dir_size(&canon);
    crate::modules::move_to_trash(&canon)?;
    Ok(format!("已将「{}」移到废纸篓（约 {:.1} MB），可从废纸篓恢复", name, size as f64 / MB as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_common_apps() {
        assert_eq!(categorize("com.tencent.xin", "微信"), "Communication");
        assert_eq!(categorize("com.microsoft.VSCode", "Visual Studio Code"), "Development");
        assert_eq!(categorize("com.google.Chrome", "Google Chrome"), "Entertainment");
        assert_eq!(categorize("md.obsidian", "Obsidian"), "Productivity");
        assert_eq!(categorize("com.unknown.app", "Something"), "Other");
    }

    #[test]
    fn app_directories_include_system_and_user() {
        let dirs = app_directories();
        assert_eq!(dirs[0], PathBuf::from("/Applications"));
        assert!(dirs.iter().any(|d| d.ends_with("Applications") && d != &PathBuf::from("/Applications")));
    }
}




