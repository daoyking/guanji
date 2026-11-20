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

/// 收集明确归属该应用的用户数据路径（均在 ~/Library 下）。
/// 只用 bundle id / 应用名做精确匹配，避免误删其他应用数据。
fn app_data_candidates(app_name: &str, bundle_id: &str) -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let lib = home.join("Library");
    let mut v: Vec<PathBuf> = Vec::new();

    // bundle id 精确的容器/缓存目录
    if !bundle_id.is_empty() && !bundle_id.contains('/') {
        for base in [
            "Containers", "Group Containers", "Caches", "Saved Application State",
            "HTTPStorages", "WebKit", "Application Scripts", "Autosave Information",
        ] {
            v.push(lib.join(base).join(bundle_id));
        }
        v.push(lib.join("Application Support").join(bundle_id));
        v.push(lib.join("Logs").join(bundle_id));
        v.push(lib.join("Preferences").join(format!("{}.plist", bundle_id)));
        v.push(lib.join("LaunchAgents").join(format!("{}.plist", bundle_id)));
    }
    // 应用名目录（部分老应用不用 bundle id 命名）
    if !app_name.is_empty() && !app_name.contains('/') {
        for base in ["Application Support", "Caches", "Logs"] {
            v.push(lib.join(base).join(app_name));
        }
    }
    // ByHost 偏好：文件名以 bundle id 开头
    if !bundle_id.is_empty() {
        if let Ok(rd) = fs::read_dir(lib.join("Preferences/ByHost")) {
            let prefix = format!("{}.", bundle_id);
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    v.push(e.path());
                }
            }
        }
    }
    v
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

    let candidates = app_data_candidates(&app_name, &bundle_id);
    let mut freed = 0u64;
    let mut count = 0usize;
    let mut fails = Vec::new();
    for dir in candidates {
        if !dir.exists() {
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

/// 从程序坞移除指向该应用的图标（含分隔占位）。返回是否实际改动。
/// 直接改 plist 后重启 Dock；cfprefsd 缓存也一并失效，避免改动被回滚。
fn remove_dock_icon(app_path: &Path) -> Result<bool, String> {
    let canonical = app_path.canonicalize().unwrap_or_else(|_| app_path.to_path_buf());
    let script = r#"
import plistlib, os, sys, urllib.parse
target = os.path.realpath(sys.argv[1]).rstrip('/')
plist = os.path.expanduser('~/Library/Preferences/com.apple.dock.plist')
def norm(u):
    if not isinstance(u, str): return ''
    s = urllib.parse.unquote(u)
    if s.startswith('file://'): s = s[7:]
    return os.path.realpath(s).rstrip('/')
try:
    with open(plist, 'rb') as f: d = plistlib.load(f)
except Exception:
    print('0'); raise SystemExit(0)
changed, kept_all = False, True
for key in ('persistent-apps', 'persistent-others'):
    arr = d.get(key) or []
    keep = []
    for item in arr:
        td = item.get('tile-data') or {}
        fd = td.get('file-data') or {}
        matched = bool(target) and norm(fd.get('_CFURLString', '')) == target
        if matched: changed = True
        else: keep.append(item)
    if len(keep) != len(arr): d[key] = keep
if changed:
    try:
        with open(plist, 'wb') as f: plistlib.dump(d, f)
        print('1')
    except Exception:
        print('0')
else:
    print('0')
"#;
    let out = Command::new("python3")
        .arg("-c").arg(script)
        .arg(&canonical)
        .output()
        .map_err(|e| format!("无法更新程序坞: {}", e))?;
    let changed = out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "1";
    if changed {
        // cfprefsd 缓存会让 Dock 读回旧值，先失效缓存再重启 Dock
        let _ = Command::new("killall").arg("cfprefsd").output();
        let _ = Command::new("killall").arg("Dock").output();
    }
    Ok(changed)
}

/// 注销 LaunchServices 注册，清除「应用程序 / 启动台」里的残留图标记录。
fn unregister_launchservices(app_path: &Path) -> bool {
    let ls = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    Command::new(ls).arg("-u").arg(app_path).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

/// 卸载应用：应用本体 + 用户数据一起移到废纸篓，并清理程序坞与启动台残留。
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

    let (app_name, bundle_id) = read_plist(&canon);
    let display_name = if app_name.is_empty() {
        canon.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    } else { app_name.clone() };

    let mut notes: Vec<String> = Vec::new();
    let mut fails: Vec<String> = Vec::new();

    // 1) 先注销 LaunchServices / 移除 Dock 图标（需在应用移动前，路径还有效）
    if unregister_launchservices(&canon) {
        notes.push("已注销应用程序注册".into());
    }
    match remove_dock_icon(&canon) {
        Ok(true) => notes.push("已移除程序坞图标".into()),
        Ok(false) => {}
        Err(e) => fails.push(format!("程序坞图标: {}", e)),
    }

    // 2) 用户数据移到废纸篓
    let mut freed = 0u64;
    let mut data_count = 0usize;
    for dir in app_data_candidates(&app_name, &bundle_id) {
        if !dir.exists() { continue; }
        let size = crate::modules::dir_size(&dir);
        match crate::modules::move_to_trash(&dir) {
            Ok(_) => { freed += size; data_count += 1; }
            Err(e) => fails.push(format!("数据 {}: {}", dir.file_name().unwrap_or_default().to_string_lossy(), e)),
        }
    }

    // 3) 应用本体移到废纸篓（find 兜底处理权限/系统保护项）
    let size = crate::modules::dir_size(&canon);
    crate::modules::move_to_trash(&canon)?;

    let mut msg = format!(
        "已将「{}」及其 {} 项数据移到废纸篓（应用约 {:.1} MB，数据约 {:.1} MB，可恢复）",
        display_name, data_count, size as f64 / MB as f64, freed as f64 / MB as f64
    );
    if !notes.is_empty() { msg.push_str("\n· "); msg.push_str(&notes.join("；")); }
    if !fails.is_empty() { msg.push_str(&format!("\n部分项未完成：{}", fails.join("；"))); }
    Ok(msg)
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

    #[test]
    fn data_candidates_are_scoped_to_bundle_and_app_name() {
        let v = app_data_candidates("Obsidian", "md.obsidian");
        assert!(!v.is_empty());
        // 所有候选都在 ~/Library 下
        let home = dirs::home_dir().unwrap();
        let lib = home.join("Library");
        assert!(v.iter().all(|p| p.starts_with(&lib)), "存在越界路径");
        // 关键目录齐全
        assert!(v.iter().any(|p| p.ends_with("Containers/md.obsidian")));
        assert!(v.iter().any(|p| p.ends_with("Caches/md.obsidian")));
        assert!(v.iter().any(|p| p.ends_with("Application Support/Obsidian")));
        assert!(v.iter().any(|p| p.ends_with("Preferences/md.obsidian.plist")));
    }

    #[test]
    fn data_candidates_reject_path_traversal_in_identifier() {
        // 恶意 bundle id 含 / 时不得生成越界路径
        let v = app_data_candidates("x", "../../etc");
        assert!(v.iter().all(|p| p.starts_with(dirs::home_dir().unwrap().join("Library"))));
    }
}




