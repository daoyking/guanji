//! Module for file management (large files, cleanup, duplicates)

use serde::{Deserialize, Serialize};
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

// ─── 用户可配置的扫描目录 / 忽略目录 ─────────────────────────

const KEY_SCAN_DIRS: &str = "file_scan_dirs";
const KEY_IGNORE_DIRS: &str = "file_ignore_dirs";

/// 默认扫描根目录（用户未配置时使用）
fn default_scan_dirs() -> Vec<String> {
    let home = dirs::home_dir().unwrap_or_default();
    ["Downloads", "Desktop", "Documents", "Videos", "Music"]
        .iter()
        .map(|d| home.join(d).to_string_lossy().to_string())
        .collect()
}

/// 默认忽略项：按目录名匹配（任意层级），避免扫到代码依赖/版本库/系统库
fn default_ignore_dirs() -> Vec<String> {
    vec![
        "node_modules".into(),
        ".git".into(),
        "Library".into(),
        ".cache".into(),
    ]
}

/// 从 settings 读取 JSON 数组；未配置时返回 default
fn read_list(key: &str, default: Vec<String>) -> Vec<String> {
    let raw = crate::db::get_setting_inner(key).unwrap_or_default();
    if raw.trim().is_empty() {
        return default;
    }
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or(default)
}

fn write_list(key: &str, list: &[String]) -> Result<(), String> {
    let raw = serde_json::to_string(list).map_err(|e| e.to_string())?;
    crate::db::with_db(|conn| {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, raw],
        )?;
        Ok(())
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileScanConfig {
    pub scan_dirs: Vec<String>,
    pub ignore_dirs: Vec<String>,
}

/// 读取完整配置（扫描目录 + 忽略目录）
#[tauri::command]
pub fn file_get_config() -> FileScanConfig {
    FileScanConfig {
        scan_dirs: read_list(KEY_SCAN_DIRS, default_scan_dirs()),
        ignore_dirs: read_list(KEY_IGNORE_DIRS, default_ignore_dirs()),
    }
}

#[tauri::command]
pub fn file_set_scan_dirs(dirs: Vec<String>) -> Result<(), String> {
    write_list(KEY_SCAN_DIRS, &dirs)
}

#[tauri::command]
pub fn file_set_ignore_dirs(dirs: Vec<String>) -> Result<(), String> {
    write_list(KEY_IGNORE_DIRS, &dirs)
}

/// 判断某个目录是否应被忽略。
/// 规则（对每条 ignore 项）：
/// 1. 含路径分隔符 / 或以 ~ 开头 → 视为路径：规范化后判断是否为该路径本身或其子目录
/// 2. 否则 → 视为目录名：路径中任一层文件名与之相等即忽略（任意层级）
fn is_ignored(path: &Path, ignores: &[String]) -> bool {
    if ignores.is_empty() {
        return false;
    }
    // 目录名逐层比对（一次遍历同时覆盖「名称匹配」）
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // 解析符号链接：macOS 原生文件夹对话框返回的是链接指向的真实路径，
    // 但扫描可能从 ~/Downloads 下的符号链接入口进入。两边都 canonicalize 后比较
    // 才能让「忽略真实路径」对「链接入口」同样生效。
    let path_canon = fs::canonicalize(path).ok();
    for rule in ignores {
        let rule = rule.trim();
        if rule.is_empty() {
            continue;
        }
        if rule.contains('/') || rule.starts_with('~') {
            // 路径规则：展开 ~ 后先做字符串前缀匹配
            let expanded = expand_tilde(rule);
            let rule_path = PathBuf::from(&expanded);
            if path == rule_path || path.starts_with(&rule_path) {
                return true;
            }
            // 再做符号链接解析后的匹配（真实路径 vs 真实路径）
            if let Some(pc) = &path_canon {
                if let Ok(rc) = fs::canonicalize(&rule_path) {
                    if *pc == rc || pc.starts_with(&rc) {
                        return true;
                    }
                }
            }
        } else if name == rule {
            // 目录名规则
            return true;
        }
    }
    false
}

fn expand_tilde(s: &str) -> String {
    if s == "~" {
        return dirs::home_dir().unwrap_or_default().to_string_lossy().to_string();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    s.to_string()
}

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

async fn collect_files(dir: &Path, threshold: u64, results: &mut Vec<FileEntry>, ignores: &[String]) {
    if !dir.exists() || !dir.is_dir() { return; }
    // 入口目录本身若被忽略（例如用户把某个扫描根加入了忽略），直接跳过
    if is_ignored(dir, ignores) { return; }
    let entries = match fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if is_ignored(&path, ignores) {
                continue;
            }
            if path.components().count() - dir.components().count() < 3 {
                Box::pin(collect_files(&path, threshold, results, ignores)).await;
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
    let ignores = read_list(KEY_IGNORE_DIRS, default_ignore_dirs());
    let mut results = Vec::new();
    collect_files(&download_dir, threshold, &mut results, &ignores).await;
    results.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap());
    Ok(results)
}

/// List large files across common user directories
#[tauri::command]
pub async fn list_large_files(threshold_mb: f64, limit: usize) -> Result<Vec<FileEntry>, String> {
    let threshold = (threshold_mb * MB as f64) as u64;
    let scan_dirs = read_list(KEY_SCAN_DIRS, default_scan_dirs());
    let ignores = read_list(KEY_IGNORE_DIRS, default_ignore_dirs());
    let mut results = Vec::new();
    for dir_str in &scan_dirs {
        let dir = PathBuf::from(expand_tilde(dir_str));
        if dir.exists() { collect_files(&dir, threshold, &mut results, &ignores).await; }
    }
    results.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap());
    results.truncate(limit);
    Ok(results)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrashFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrashResult {
    pub moved: Vec<String>,
    pub failed: Vec<TrashFailure>,
}

/// 把用户勾选的大文件移到废纸篓（可恢复，绝不永久删除）。
/// 安全约束：只接受普通文件；拒绝系统关键目录、废纸篓自身内的路径；
/// 同名文件加时间戳后缀避免覆盖。跨磁盘移动失败时返回明确原因。
#[tauri::command]
pub fn trash_files(paths: Vec<String>) -> Result<TrashResult, String> {
    let home = dirs::home_dir().ok_or("无法定位用户主目录")?;
    let trash = home.join(".Trash");
    fs::create_dir_all(&trash).map_err(|e| format!("无法访问废纸篓: {}", e))?;
    Ok(trash_files_to(paths, &trash))
}

/// 核心逻辑（trash 路径可注入，便于测试）
fn trash_files_to(paths: Vec<String>, trash: &Path) -> TrashResult {
    let trash_canon = trash.canonicalize().unwrap_or_else(|_| trash.to_path_buf());

    // 系统关键目录前缀，一律拒绝。注意不能笼统拒绝 /private——
    // macOS 的临时目录 canonicalize 后正是 /private/var/folders/...
    let forbidden = [
        "/System", "/usr", "/bin", "/sbin", "/Library",
        "/private/etc", "/private/var/db", "/private/var/root",
        "/.Trash",
    ];

    let mut moved = Vec::new();
    let mut failed = Vec::new();

    for raw in paths {
        let expanded = expand_tilde(&raw);
        let path = PathBuf::from(&expanded);

        // 只处理普通文件，目录一律跳过（防止误删整个文件夹）
        if !path.is_file() {
            failed.push(TrashFailure {
                path: raw,
                reason: "不是普通文件或文件已不存在，已跳过".into(),
            });
            continue;
        }

        let canon = match path.canonicalize() {
            Ok(c) => c,
            Err(e) => {
                failed.push(TrashFailure { path: raw, reason: format!("无法解析路径: {}", e) });
                continue;
            }
        };

        let canon_str = canon.to_string_lossy().to_string();
        if forbidden.iter().any(|p| canon_str == *p || canon_str.starts_with(&format!("{}/", p))) {
            failed.push(TrashFailure {
                path: raw,
                reason: "系统关键路径，出于安全考虑拒绝删除".into(),
            });
            continue;
        }
        if canon.starts_with(&trash_canon) {
            failed.push(TrashFailure {
                path: raw,
                reason: "文件已在废纸篓中".into(),
            });
            continue;
        }

        let name = canon.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.is_empty() {
            failed.push(TrashFailure { path: raw, reason: "无法获取文件名".into() });
            continue;
        }

        let mut dest = trash.join(name);
        if dest.exists() {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            dest = trash.join(format!("{}.{}", name, stamp));
        }

        match fs::rename(&canon, &dest) {
            Ok(_) => moved.push(raw),
            Err(e) => {
                let reason = if e.raw_os_error() == Some(18) {
                    "跨磁盘移动失败，请手动处理".into()
                } else {
                    format!("移动失败: {}", e)
                };
                failed.push(TrashFailure { path: raw, reason });
            }
        }
    }

    TrashResult { moved, failed }
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
    let ignores = read_list(KEY_IGNORE_DIRS, default_ignore_dirs());
    let mut size_map: std::collections::HashMap<u64, Vec<(String, String)>> = std::collections::HashMap::new();
    for path_str in paths {
        let path = PathBuf::from(expand_tilde(&path_str));
        if path.exists() && path.is_dir() {
            let mut stack = vec![path.clone()];
            while let Some(current) = stack.pop() {
                if is_ignored(&current, &ignores) { continue; }
                if let Ok(entries) = fs::read_dir(&current) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            if !is_ignored(&p, &ignores) && current.components().count() < 5 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_by_name_matches_any_level() {
        let ignores = vec!["node_modules".to_string(), ".git".to_string()];
        assert!(is_ignored(Path::new("/Users/me/proj/node_modules"), &ignores));
        assert!(is_ignored(Path::new("/Users/me/a/b/c/.git"), &ignores));
        assert!(!is_ignored(Path::new("/Users/me/Documents/report.pdf"), &ignores));
    }

    #[test]
    fn ignore_by_absolute_path_matches_self_and_children() {
        let ignores = vec!["/Users/me/Library".to_string()];
        assert!(is_ignored(Path::new("/Users/me/Library"), &ignores));
        assert!(is_ignored(Path::new("/Users/me/Library/Caches"), &ignores));
        assert!(!is_ignored(Path::new("/Users/me/Documents"), &ignores));
        // 同为 Library 结尾但不同父目录，不应被路径规则命中
        assert!(!is_ignored(Path::new("/Users/other/Library"), &ignores));
    }

    #[test]
    fn ignore_path_rule_matches_symlink_entry() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join("gj_ignore_symlink_test");
        let _ = fs::remove_dir_all(&root);
        let real = root.join("real_target");
        let link = root.join("link_entry");
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(real.join("sub")).unwrap();
        symlink(&real, &link).unwrap();

        // 规则用真实路径（模拟原生对话框返回的值）
        let ignores = vec![real.to_string_lossy().to_string()];
        // 从符号链接入口进入时也应被忽略
        assert!(is_ignored(&link, &ignores), "符号链接入口应命中真实路径规则");
        assert!(is_ignored(&link.join("sub"), &ignores));
        // 反向：规则用链接路径，真实路径也应命中
        let link_rules = vec![link.to_string_lossy().to_string()];
        assert!(is_ignored(&real, &link_rules), "真实路径应命中链接路径规则");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_ignores_ignore_nothing() {
        assert!(!is_ignored(Path::new("/anything"), &[]));
    }

    #[test]
    fn tilde_rule_uses_home() {
        let home = dirs::home_dir().unwrap_or_default();
        let ignores = vec!["~/Pictures".to_string()];
        let pics = home.join("Pictures");
        assert!(is_ignored(&pics, &ignores));
        assert!(is_ignored(&pics.join("sub"), &ignores));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn collect_files_respects_ignore_rules() {
        let root = std::env::temp_dir().join("gj_collect_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        // 造两个超过阈值的文件（阈值设很低，1 byte 即算大文件）
        fs::write(root.join("src").join("keep.bin"), b"aaaa").unwrap();
        fs::write(root.join("node_modules").join("skip.bin"), b"bbbbbbbb").unwrap();

        // 不忽略：应拿到 2 个文件
        let mut r1 = Vec::new();
        futures_lite_block_on(collect_files(&root, 1, &mut r1, &[]));
        assert_eq!(r1.len(), 2, "未配置忽略时应扫到 2 个文件");

        // 忽略 node_modules：应只剩 1 个
        let ignores = vec!["node_modules".to_string()];
        let mut r2 = Vec::new();
        futures_lite_block_on(collect_files(&root, 1, &mut r2, &ignores));
        assert_eq!(r2.len(), 1, "忽略 node_modules 后应只剩 1 个文件");
        assert!(r2[0].name == "keep.bin");

        let _ = fs::remove_dir_all(&root);
    }

    // collect_files 是 async 但只用了 await 递归，无真正 I/O 阻塞点；
    // 这里用一个极简 block_on 驱动，避免引入 tokio 依赖到测试。
    fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker { RawWaker::new(p, &VTABLE) }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    #[test]
    fn trash_moves_file_and_skips_dirs_and_missing() {
        let root = std::env::temp_dir().join("gj_trash_test");
        let _ = fs::remove_dir_all(&root);
        let trash = root.join("trash");
        let src_dir = root.join("src");
        fs::create_dir_all(&trash).unwrap();
        fs::create_dir_all(src_dir.join("subdir")).unwrap();
        let f1 = src_dir.join("big.bin");
        fs::write(&f1, b"hello").unwrap();

        let r = trash_files_to(
            vec![
                f1.to_string_lossy().to_string(),
                src_dir.join("subdir").to_string_lossy().to_string(),
                src_dir.join("missing.bin").to_string_lossy().to_string(),
            ],
            &trash,
        );

        assert_eq!(r.moved.len(), 1, "只有普通文件应被移动");
        assert_eq!(r.failed.len(), 2, "目录和不存在的文件应被跳过");
        assert!(trash.join("big.bin").exists(), "文件应出现在废纸篓");
        assert!(!f1.exists(), "原位置文件应消失");
        assert!(src_dir.join("subdir").exists(), "目录绝不能被删除");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn trash_handles_name_collision_without_overwrite() {
        let root = std::env::temp_dir().join("gj_trash_collision");
        let _ = fs::remove_dir_all(&root);
        let trash = root.join("trash");
        let src = root.join("dup.bin");
        fs::create_dir_all(&trash).unwrap();
        fs::write(&src, b"new").unwrap();
        fs::write(trash.join("dup.bin"), b"old").unwrap();

        let r = trash_files_to(vec![src.to_string_lossy().to_string()], &trash);
        assert_eq!(r.moved.len(), 1);
        assert_eq!(fs::read(trash.join("dup.bin")).unwrap(), b"old", "已有文件不能被覆盖");
        let renamed = fs::read_dir(&trash).unwrap().map(|e| e.unwrap().path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("dup.bin."))
            .expect("应生成带后缀的副本");
        assert_eq!(fs::read(&renamed).unwrap(), b"new");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn trash_rejects_system_paths() {
        let root = std::env::temp_dir().join("gj_trash_sys");
        let trash = root.join("trash");
        fs::create_dir_all(&trash).unwrap();
        if Path::new("/etc/hosts").exists() {
            let r = trash_files_to(vec!["/etc/hosts".into()], &trash);
            assert!(r.failed.iter().any(|f| f.reason.contains("系统关键路径")));
            assert!(Path::new("/etc/hosts").exists(), "系统文件必须保持原样");
        }
        let _ = fs::remove_dir_all(&root);
    }
}

