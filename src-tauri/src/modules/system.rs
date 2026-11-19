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
    /// 终止风险等级：dangerous / caution / safe
    pub kill_risk: String,
    /// 终止建议文案
    pub kill_recommend: String,
    /// 判定原因（依据进程名 / 路径 / PID）
    pub kill_reason: String,
    /// 终止后的影响说明
    pub kill_impact: String,
}

/// 终止风险评估结果
#[derive(Clone, Debug, PartialEq)]
pub struct KillAssessment {
    pub risk: &'static str,
    pub recommend: &'static str,
    pub reason: String,
    pub impact: String,
}

/// 已知的系统关键进程（macOS / 通用），终止会破坏系统稳定性。
const CRITICAL_PROCS: &[&str] = &[
    "kernel_task", "launchd", "windowsserver", "loginwindow", "coreservicesd",
    "hidd", "coreaudiod", "securityd", "distnoted", "syslogd", "configd",
    "bluetoothd", "sysmond", "powerd", "mds", "mds_stores", "spotlight",
    "usernotificationsd", "opendirectoryd", "fseventsd", "socat", "init",
    "systemd", "firewall", "syslog", "sudo",
];

/// 有未保存数据风险的应用名关键字 → caution
const DOC_APP_HINTS: &[&str] = &[
    "word", "excel", "powerpoint", "keynote", "pages", "numbers",
    "xcode", "photoshop", "figma", "sketch", "illustrator", "vim", "emacs",
    "code", "cursor", "sublime", "atom", "notion", "obsidian",
];

/// 浏览器 → 丢失标签页/表单
const BROWSER_HINTS: &[&str] = &[
    "chrome", "safari", "firefox", "edge", "brave", "arc", "opera",
];

/// 系统保护路径前缀：这些路径下的进程一律视为系统关键
const SYSTEM_PATH_PREFIXES: &[&str] = &[
    "/system/", "/usr/libexec/", "/usr/sbin/", "/usr/bin/", "/bin/",
    "/sbin/", "/library/apple/", "/private/var/db/",
];

/// 评估终止某个进程的风险。纯函数，便于单测。
///
/// - `pid` / `name`：进程标识
/// - `exe`：可执行文件路径（拿不到时可传空串，退化为按名称判断）
/// - `self_pid`：管家自身 PID（终止自己会导致应用直接退出）
pub fn assess_process(pid: u32, name: &str, exe: &str, self_pid: u32) -> KillAssessment {
    let lower_name = name.to_lowercase();
    let lower_exe = exe.to_lowercase();

    // 1) 管家自身：终止等于自杀
    if pid == self_pid {
        return KillAssessment {
            risk: "dangerous",
            recommend: "不建议终止",
            reason: "这是管家自身的进程".into(),
            impact: "终止后本应用会立即退出".into(),
        };
    }

    // 2) PID 极小（通常是内核 / 系统守护进程）
    if pid <= 100 {
        return KillAssessment {
            risk: "dangerous",
            recommend: "不建议终止",
            reason: format!("PID {} 属于系统保留进程区间", pid),
            impact: "终止可能导致系统不稳定或服务中断".into(),
        };
    }

    // 3) 已知关键进程名
    if CRITICAL_PROCS.iter().any(|p| lower_name == *p || lower_name.starts_with(&format!("{} ", p))) {
        return KillAssessment {
            risk: "dangerous",
            recommend: "不建议终止",
            reason: format!("「{}」是系统关键进程", name),
            impact: "终止可能导致系统异常、界面卡死或需要重新登录".into(),
        };
    }

    // 4) 可执行文件位于系统保护路径
    if SYSTEM_PATH_PREFIXES.iter().any(|p| lower_exe.starts_with(p)) {
        return KillAssessment {
            risk: "dangerous",
            recommend: "不建议终止",
            reason: "进程来自系统保护目录".into(),
            impact: "终止系统服务可能导致功能异常或系统不稳定".into(),
        };
    }

    // 5) 浏览器：未保存标签页/表单
    if BROWSER_HINTS.iter().any(|h| lower_name.contains(h)) {
        return KillAssessment {
            risk: "caution",
            recommend: "谨慎终止",
            reason: "这是浏览器进程".into(),
            impact: "所有标签页会关闭，未提交的表单和草稿可能丢失".into(),
        };
    }

    // 6) 文档/编辑器类应用：未保存工作
    if DOC_APP_HINTS.iter().any(|h| lower_name.contains(h)) {
        return KillAssessment {
            risk: "caution",
            recommend: "谨慎终止",
            reason: "这是文档 / 编辑类应用".into(),
            impact: "未保存的文档或代码可能丢失".into(),
        };
    }

    // 7) 普通应用（在 /Applications 下）：可能有未保存数据
    if lower_exe.contains("/applications/") || lower_exe.ends_with(".app") {
        return KillAssessment {
            risk: "caution",
            recommend: "谨慎终止",
            reason: "这是用户安装的应用程序".into(),
            impact: "应用会立即退出，未保存的数据可能丢失".into(),
        };
    }

    // 8) 后台辅助进程（helper/agent/daemon）：通常影响较小
    if ["helper", "agent", "daemon", "service"].iter().any(|h| lower_name.contains(h)) {
        return KillAssessment {
            risk: "safe",
            recommend: "可以终止",
            reason: "这是后台辅助进程".into(),
            impact: "通常影响较小，但依赖它的应用可能短暂异常（多会自动重启）".into(),
        };
    }

    // 9) 兜底：未知进程，保守处理
    KillAssessment {
        risk: "caution",
        recommend: "谨慎终止",
        reason: "未能识别该进程的具体用途".into(),
        impact: "终止可能导致相关功能中断，请先确认它不属于正在使用的程序".into(),
    }
}

/// 终止进程（发 SIGKILL）。危险进程与管家自身会被拒绝，防止误杀。
#[tauri::command]
pub fn kill_process(pid: u32) -> Result<String, String> {
    let self_pid = std::process::id();
    if pid == self_pid {
        return Err("不能终止管家自身进程".into());
    }
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
    let target = sys
        .process(sysinfo::Pid::from_u32(pid))
        .ok_or_else(|| format!("进程 {} 不存在或已退出", pid))?;
    let name = target.name().to_string_lossy().to_string();
    let exe = target.exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let assessment = assess_process(pid, &name, &exe, self_pid);
    if assessment.risk == "dangerous" {
        return Err(format!(
            "已阻止终止系统关键进程「{}」：{}",
            name, assessment.impact
        ));
    }
    target
        .kill()
        .then(|| format!("已终止进程「{}」({})", name, pid))
        .ok_or_else(|| format!("终止进程「{}」失败，可能权限不足", name))
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
    let self_pid = std::process::id();
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .filter(|p| !p.name().is_empty())
        .map(|p| {
            let pid = p.pid().as_u32();
            let name = p.name().to_string_lossy().to_string();
            let exe = p.exe().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            let a = assess_process(pid, &name, &exe, self_pid);
            ProcessInfo {
                pid,
                name,
                cpu_percent: p.cpu_usage(),
                memory_mb: p.memory() / (1024 * 1024),
                status: format!("{:?}", p.status()),
                kill_risk: a.risk.to_string(),
                kill_recommend: a.recommend.to_string(),
                kill_reason: a.reason,
                kill_impact: a.impact,
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SELF: u32 = 99999;

    #[test]
    fn self_process_is_dangerous() {
        let a = assess_process(SELF, "guanji", "/Applications/管家.app", SELF);
        assert_eq!(a.risk, "dangerous");
        assert!(a.reason.contains("管家"));
    }

    #[test]
    fn low_pid_is_dangerous() {
        let a = assess_process(1, "launchd", "", SELF);
        assert_eq!(a.risk, "dangerous");
        let a2 = assess_process(42, "some_daemon", "/Applications/X.app", SELF);
        assert_eq!(a2.risk, "dangerous");
    }

    #[test]
    fn critical_names_are_dangerous() {
        for name in ["kernel_task", "WindowServer", "loginwindow", "mds_stores", "coreaudiod"] {
            let a = assess_process(500, name, "/usr/libexec/x", SELF);
            assert_eq!(a.risk, "dangerous", "{} 应为危险", name);
        }
    }

    #[test]
    fn system_path_is_dangerous() {
        let a = assess_process(600, "unknown", "/System/Library/PrivateFrameworks/x", SELF);
        assert_eq!(a.risk, "dangerous");
        let a2 = assess_process(601, "unknown", "/usr/libexec/foo", SELF);
        assert_eq!(a2.risk, "dangerous");
    }

    #[test]
    fn browsers_are_caution() {
        let a = assess_process(1000, "Google Chrome", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", SELF);
        assert_eq!(a.risk, "caution");
        assert!(a.impact.contains("标签页"));
    }

    #[test]
    fn doc_apps_are_caution() {
        let a = assess_process(1001, "Code", "/Applications/Visual Studio Code.app/Contents/MacOS/Electron", SELF);
        assert_eq!(a.risk, "caution");
        assert!(a.impact.contains("未保存"));
    }

    #[test]
    fn helper_processes_are_safe() {
        let a = assess_process(2000, "foo Helper", "/Users/me/Library/foo Helper.app/x", SELF);
        assert_eq!(a.risk, "safe");
        let a2 = assess_process(2001, "bar_agent", "/Users/me/bin/bar_agent", SELF);
        assert_eq!(a2.risk, "safe");
    }

    #[test]
    fn unknown_is_caution_by_default() {
        let a = assess_process(3000, "mystery", "", SELF);
        assert_eq!(a.risk, "caution");
        assert!(a.reason.contains("未能识别"));
    }

    #[test]
    fn case_insensitive_name_match() {
        let a = assess_process(500, "KERNEL_TASK", "", SELF);
        assert_eq!(a.risk, "dangerous");
    }
}

