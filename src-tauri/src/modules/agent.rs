//! M8: AI Agent 编排核心 — 意图理解 → 工具调用 → 观察 → 迭代 → 回答
//! 兼容本地小模型的三种工具调用输出：原生 tool_calls / JSON 文本 / ReAct Action。
//! 自主进化：历史执行轨迹作为 few-shot 注入 system prompt，越用越懂你。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, State};
use crate::db::with_db;
use crate::modules::provider::{self, Provider, LOCAL_PROVIDER_ID};

const MAX_STEPS: usize = 8;

/// Agent 运行时控制状态：取消标志 + 破坏性操作确认桥接。
/// 确认用 HashMap 按 id 存储：多工具并行时可能有多个待确认操作，单槽会互相覆盖。
pub struct AgentControl {
    cancelled: AtomicBool,
    confirm_tx: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
}

impl Default for AgentControl {
    fn default() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            confirm_tx: Mutex::new(HashMap::new()),
        }
    }
}

impl AgentControl {
    pub fn new() -> Self { Self::default() }
    fn reset(&self) { self.cancelled.store(false, Ordering::SeqCst); }
    fn cancel(&self) { self.cancelled.store(true, Ordering::SeqCst); }
    fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::SeqCst) }
}

static CONFIRM_SEQ: AtomicU64 = AtomicU64::new(0);
fn next_confirm_id() -> String {
    format!("cfm-{}", CONFIRM_SEQ.fetch_add(1, Ordering::SeqCst))
}

/// 前端点击「停止」：置取消标志，循环在下一个检查点退出。
#[tauri::command]
pub fn agent_cancel(ctrl: State<'_, AgentControl>) -> Result<(), String> {
    ctrl.cancel();
    Ok(())
}

/// 前端确认/拒绝破坏性操作：按 id 把结果送回对应的 execute_tool。
#[tauri::command]
pub fn agent_confirm(id: String, approved: bool, ctrl: State<'_, AgentControl>) -> Result<(), String> {
    if let Some(tx) = ctrl.confirm_tx.lock().unwrap().remove(&id) {
        let _ = tx.send(approved);
    }
    Ok(())
}

/// 请求用户确认一个破坏性操作：发事件 + 等待前端应答（60s 超时，默认拒绝）。
async fn request_confirm(app: &AppHandle, description: String) -> bool {
    let id = next_confirm_id();
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Some(ctrl) = app.try_state::<AgentControl>() {
        ctrl.confirm_tx.lock().unwrap().insert(id.clone(), tx);
    } else {
        return false; // 无 state 时安全拒绝
    }
    let _ = app.emit("agent://confirm", json!({"id": id, "description": description}));
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
        Ok(Ok(approved)) => approved,
        _ => false, // 超时或前端断开 → 默认拒绝
    }
}

fn ollama_url() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

/// Ollama 是本机服务，用禁用代理的客户端直连（见 modules::local_http_client）。
fn ollama_http_client() -> reqwest::Client {
    crate::modules::local_http_client()
}

const MODEL_PREF_KEY: &str = "agent_model";
const DEFAULT_REF: &str = "ollama::qwen2.5-coder:7b";

/// 模型引用格式：`provider_id::model_name`。无 `::` 的旧值视为本地 Ollama（向后兼容）。
fn parse_model_ref(s: &str) -> (String, String) {
    if let Some((p, m)) = s.split_once("::") {
        (p.to_string(), m.to_string())
    } else {
        (LOCAL_PROVIDER_ID.to_string(), s.to_string())
    }
}

fn make_ref(provider_id: &str, model: &str) -> String {
    format!("{}::{}", provider_id, model)
}

/// 当前 Agent 使用的完整模型引用（含 provider 前缀）
fn model_ref() -> String {
    let stored = std::thread::spawn(|| {
        with_db(|conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                rusqlite::params![MODEL_PREF_KEY],
                |r| r.get::<_, String>(0),
            )
        })
    })
    .join()
    .ok()
    .and_then(|r| r.ok());
    stored
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("GUANJI_AGENT_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_REF.to_string())
}

/// 当前 Agent 使用的模型名（不含 provider 前缀）—— 供显示与请求 body 使用
fn agent_model() -> String {
    parse_model_ref(&model_ref()).1
}

/// 读取 provider 配置（本地 Ollama 用占位，云 provider 从库里取）
fn resolve_provider(provider_id: &str) -> Result<Provider, String> {
    if provider_id == LOCAL_PROVIDER_ID {
        return Ok(Provider {
            id: LOCAL_PROVIDER_ID.to_string(),
            name: "本机 Ollama".into(),
            base_url: ollama_url(),
            api_key: String::new(),
            enabled: true,
        });
    }
    provider::provider_list_full()?
        .into_iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("服务商 {} 已不存在，请重新选择模型", provider_id))
}

// ─── 对外数据结构 ──────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolTrace {
    pub step: usize,
    /// 同一 step 内并行调用的序号，用于区分多个工具
    pub call_id: Option<usize>,
    pub tool: String,
    pub args: Value,
    pub result: String,
    pub ok: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentReply {
    pub answer: String,
    pub traces: Vec<ToolTrace>,
    pub steps: usize,
    pub model: String,
}

// ─── 工具注册表（JSON Schema，供模型选择）─────────────────────

fn tools_schema() -> Vec<Value> {
    vec![
        tool("get_system_info", "获取系统信息：CPU、内存、磁盘、系统版本", json!({})),
        tool("list_processes", "列出当前运行的进程（名称、PID、CPU、内存占用）", json!({
            "limit": {"type":"integer","description":"返回数量，默认15"}
        })),
        tool("list_large_files", "查找指定目录下的大文件", json!({
            "threshold_mb": {"type":"number","description":"大小阈值（MB），默认100"},
            "limit": {"type":"integer","description":"返回数量，默认10"}
        })),
        tool("scan_downloads", "扫描下载目录，找出占用空间大的文件", json!({
            "threshold_mb": {"type":"number","description":"大小阈值（MB），默认50"}
        })),
        tool("find_duplicates", "在指定路径查找重复文件", json!({
            "paths": {"type":"array","items":{"type":"string"},"description":"要检查的目录路径数组"}
        })),
        tool("clean_memory", "整理系统内存：回收 inactive 缓存页释放内存，不会删除任何文件，无需确认直接执行", json!({})),
        tool("clean_temp_files", "清理磁盘上的系统临时文件（会删除文件，需用户确认）", json!({})),
        tool("list_services", "列出本地服务及其健康状态（Ollama、OrcaRouter、Chroma、SearXNG）", json!({})),
        tool("probe_service", "探测单个服务的健康状态", json!({
            "service_id": {"type":"string","description":"服务ID：ollama/orcarouter/chroma/searxng"}
        })),
        tool("run_all_probes", "一次性探测所有本地服务的健康状态", json!({})),
        tool("clip_list", "查看剪贴板历史记录", json!({
            "limit": {"type":"integer","description":"返回数量，默认10"}
        })),
        tool("list_tasks", "列出所有定时任务", json!({})),
        tool("add_task", "创建一个新的定时任务", json!({
            "name": {"type":"string","description":"任务名称"},
            "command": {"type":"string","description":"要执行的shell命令"},
            "schedule": {"type":"string","description":"频率：hourly/daily/weekly"}
        })),
        tool("mem_list", "查看已记忆的信息（用户偏好、历史事实）", json!({})),
        tool("mem_add", "记住一条信息（用户偏好、事实、约定），供以后使用", json!({
            "content": {"type":"string","description":"要记住的内容"},
            "tag": {"type":"string","description":"标签：preference/fact/idea"},
            "summary": {"type":"string","description":"一句话摘要"}
        })),
        tool("mem_search", "在记忆中语义搜索相关信息", json!({
            "query": {"type":"string","description":"搜索关键词"},
            "top_n": {"type":"integer","description":"返回数量，默认5"}
        })),
        tool("notify", "向用户发送一条桌面通知（任务完成时告知用户）", json!({
            "title": {"type":"string","description":"通知标题"},
            "body": {"type":"string","description":"通知内容"}
        })),
        // ── 管家级执行工具 ──
        tool("run_command", "在电脑上执行一条 shell 命令（危险命令会被安全拦截）。用于真正做事：查系统状态、启动服务、批量文件操作等", json!({
            "command": {"type":"string","description":"要执行的 shell 命令"},
            "cwd": {"type":"string","description":"可选：工作目录"}
        })),
        tool("open_target", "用系统默认程序打开应用、文件或网址（如 open -a Safari 或打开某个 App）", json!({
            "target": {"type":"string","description":"应用名、文件路径或 URL"}
        })),
        tool("read_file", "读取一个 UTF-8 文本文件的内容（上限 100KB）", json!({
            "path": {"type":"string","description":"文件绝对路径"},
            "max_chars": {"type":"integer","description":"可选：最多返回字符数，默认8000"}
        })),
        tool("write_file", "写入文本文件（自动创建目录；系统目录受保护）。用于写笔记、配置、脚本", json!({
            "path": {"type":"string","description":"文件绝对路径"},
            "content": {"type":"string","description":"要写入的内容"},
            "append": {"type":"boolean","description":"可选：是否追加，默认覆盖"}
        })),
        tool("set_reminder", "设置一个定时提醒，到点会弹系统通知。支持「5分钟后」「2小时后」「明天10点」等说法", json!({
            "message": {"type":"string","description":"提醒内容"},
            "when": {"type":"string","description":"时间，如 5分钟后 / 2小时后 / ISO8601"}
        })),
        tool("list_reminders", "查看所有还未触发的提醒", json!({})),
        tool("delete_reminder", "删除一条提醒", json!({
            "id": {"type":"integer","description":"提醒ID"}
        })),
    ]
}

fn tool(name: &str, desc: &str, props: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": desc,
            "parameters": {
                "type": "object",
                "properties": props,
                "required": []
            }
        }
    })
}

// ─── 自主进化：从历史轨迹中提取经验 ───────────────────────────

/// 中文按二元字组切分，用于无分词器场景下的轻量相关度计算
fn text_bigrams(s: &str) -> std::collections::HashSet<String> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut set = std::collections::HashSet::new();
    if chars.len() == 1 {
        set.insert(chars[0].to_string());
        return set;
    }
    for w in chars.windows(2) {
        set.insert(w.iter().collect());
    }
    set
}

fn relevance(intent: &str, candidate: &str) -> f32 {
    let a = text_bigrams(intent);
    let b = text_bigrams(candidate);
    if a.is_empty() || b.is_empty() { return 0.0; }
    let inter = a.intersection(&b).count();
    let union = a.union(&b).count();
    if union == 0 { 0.0 } else { inter as f32 / union as f32 }
}

/// 语义相似度 + 词面重合的混合打分。语义分 <0 时视为不可用，退化为纯词面。
fn hybrid_score(semantic: Option<f32>, lexical: f32) -> f32 {
    match semantic {
        Some(s) if s >= 0.0 => 0.7 * s + 0.3 * lexical,
        _ => lexical,
    }
}

/// 按与当前意图的相关度检索历史经验，分离「成功做法」与「失败教训」。
/// 语义相似度（bge-m3 批量 embedding）为主，bigram 词面重合为辅；
/// embedding 不可用时（Ollama 未启动/无 bge-m3）自动降级为纯 bigram。
async fn load_experiences(intent: &str) -> (Vec<String>, Vec<String>) {
    let rows = tokio::task::spawn_blocking(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT tag, content, COALESCE(summary,'') FROM memories \
                 WHERE tag IN ('agent_experience','agent_failure') \
                 ORDER BY created_at DESC LIMIT 80",
            )?;
            let rows = stmt.query_map([], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            )))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();

    if rows.is_empty() { return (Vec::new(), Vec::new()); }

    // 批量算语义相似度：一次请求拿到 intent + 所有候选的向量
    let mut texts: Vec<String> = vec![intent.to_string()];
    texts.extend(rows.iter().map(|(_, _, summary)| {
        if summary.is_empty() { return summary.clone(); }
        // 用 summary（原始意图）做语义匹配，比整段经验内容更聚焦
        summary.chars().take(120).collect()
    }));
    let semantic_scores: Option<Vec<f32>> = match crate::modules::memory::embed_texts_batch(&texts).await {
        Ok(vectors) if !vectors.is_empty() => Some(
            vectors[1..].iter().map(|v| crate::modules::memory::cosine_sim(&vectors[0], v)).collect()
        ),
        _ => None,
    };

    let mut successes: Vec<(f32, String)> = Vec::new();
    let mut failures: Vec<(f32, String)> = Vec::new();
    for (idx, (tag, content, summary)) in rows.into_iter().enumerate() {
        let lexical = relevance(intent, &summary);
        let score = hybrid_score(semantic_scores.as_ref().and_then(|s| s.get(idx).copied()), lexical);
        if tag == "agent_failure" {
            failures.push((score, content));
        } else {
            successes.push((score, content));
        }
    }
    let by_score = |a: &(f32, String), b: &(f32, String)| {
        b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
    };
    successes.sort_by(by_score);
    failures.sort_by(by_score);
    (
        successes.into_iter().take(4).map(|x| x.1).collect(),
        failures.into_iter().take(3).map(|x| x.1).collect(),
    )
}

fn system_prompt(successes: &[String], failures: &[String]) -> String {
    let exp_block = if successes.is_empty() && failures.is_empty() {
        "尚无历史经验，这是第一次协作。".to_string()
    } else {
        let mut block = String::new();
        if !successes.is_empty() {
            block.push_str("你过去的相关成功经验（优先复用相同工具组合）：\n");
            for e in successes { block.push_str(&format!("- {}\n", e)); }
        }
        if !failures.is_empty() {
            block.push_str("\n你过去踩过的坑（务必避免重蹈覆辙）：\n");
            for e in failures { block.push_str(&format!("- {}\n", e)); }
        }
        block
    };
    format!(r#"你是「管家」，一台 macOS 电脑的全能 AI Agent。用户用中文下达指令，你通过调用工具真正把事情做完，而不是只给建议。

## 工作方法
1. 分析用户意图，选择最合适的工具。若多个任务互不依赖，可在一次回复中输出多个工具调用并行执行。
2. 有依赖关系的工具要分步调用：拿到前一步结果再决定下一步。
3. 信息足够时，用简洁中文给出最终答案（不要复述工具原始输出）。

## 工具调用格式（严格遵守）
输出 JSON 对象（单个工具）或 JSON 数组（多个独立工具并行），不要输出其它任何文字。必须是下面这种带 name / arguments 两个键的格式：
单个：{{"name": "工具名", "arguments": {{...}}}}
多个：[{{"name": "工具A", "arguments": {{...}}}}, {{"name": "工具B", "arguments": {{...}}}}]
❌ 绝对不要输出 {{"工具名": {{...}}}} 这种把工具名当键的格式，那是错误的。
若无需调用工具、可直接回答，输出：
{{"answer": "你的最终回答"}}

## 可用工具
{tools}

## {exp_block}

## 行为准则
- 用户说「记住/别忘了」时，用 mem_add 工具记下来。
- 用户要「打开某个应用/文件/网页」时，用 open_target。
- 用户要「读某个文件/写点什么/生成脚本」时，用 read_file / write_file。
- 用户要「定闹钟/提醒我 X 分钟后」时，用 set_reminder。
- 用户要「清理内存 / 整理内存」时，用 clean_memory：它不删文件、无不可逆损失，会自动执行，直接调用即可，不要问用户。
- 用户要「清理缓存 / 清理磁盘 / 删除临时文件」时，用 clean_temp_files 或删除类命令：这会删文件，可能弹出确认框，由用户决定是否执行。
- 需要真正执行系统操作时用 run_command（危险命令会被自动拦截，可放心尝试）。
- 文件删除、清理类操作前先扫描确认，再执行，并在回答中说清楚做了什么。
- **只能报告工具返回结果里真实存在的数字。例如 clean_memory 返回的 freeing_mb/freed_mb 是多少就说多少；工具没返回内存或磁盘数字，就绝不能自己编造「释放了 XXX MB 内存 / XXX MB 磁盘」。宁可只说「已整理完成」，也不可虚构数据。**
- **工具返回「已清理/已移到废纸篓/已删除/已整理」表示操作已经完成，绝不能再让用户手动确认或重复删除。直接用一句话报告实际结果即可。**
- 工具返回「用户取消了」时，如实告知用户操作被取消，不要编造「已完成」；若某项清理需确认而用户尚未确认，如实说明「该项需要你确认后才会执行」，不要假装已经删掉。
- 任务完成后，如果结果重要，用 notify 工具告知用户。
- 回答用中文，简洁、直接、有条理。"#,
        tools = serde_json::to_string_pretty(&tools_schema()).unwrap_or_default(),
        exp_block = exp_block,
    )
}

// ─── Ollama 调用 ───────────────────────────────────────────────

/// 统一聊天调用：返回 {content, tool_calls}，屏蔽 Ollama / OpenAI 兼容协议的响应差异。
/// - 本地 Ollama：POST {base}/api/chat，响应 message.content + message.tool_calls
/// - 云 provider：POST {base}/chat/completions（Bearer key），响应 choices[0].message
/// `model_reference` 为完整的 provider::model 引用，由调用方在对话开始时读取一次后透传。
async fn chat_complete(messages: Vec<Value>, model_reference: &str, app: Option<&AppHandle>) -> Result<Value, String> {
    let (provider_id, model) = parse_model_ref(model_reference);
    let p = resolve_provider(&provider_id)?;
    let tools = tools_schema();

    if p.id == LOCAL_PROVIDER_ID {
        // ── 本机 Ollama 原生协议（NDJSON 流式）──
        let url = format!("{}/api/chat", provider::normalize_base_url(&p.base_url));
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
            "tools": tools,
            "options": {"temperature": 0.1, "num_predict": 512}
        });
        let resp = ollama_http_client()
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| format!("Ollama 请求失败（请确认 Ollama 已启动）: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama HTTP {}: {}", status, text.chars().take(160).collect::<String>()));
        }

        let mut resp = resp;
        let mut buf = String::new();
        let mut content = String::new();
        let mut last_tool_calls: Vec<Value> = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取 Ollama 响应失败: {}", e))? {
            // 流式输出期间也响应取消
            if let Some(a) = app {
                if a.try_state::<AgentControl>().map(|c| c.is_cancelled()).unwrap_or(false) { break; }
            }
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // NDJSON：一行一个 JSON 对象
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim();
                if line.is_empty() { continue; }
                let v: Value = match serde_json::from_str(line) { Ok(v) => v, Err(_) => continue };
                let msg = &v["message"];
                if let Some(delta) = msg["content"].as_str() {
                    if !delta.is_empty() {
                        content.push_str(delta);
                        if let Some(a) = app {
                            let _ = a.emit("agent://delta", json!({"text": content}));
                        }
                    }
                }
                if let Some(tc) = msg["tool_calls"].as_array() {
                    if !tc.is_empty() { last_tool_calls = tc.clone(); }
                }
            }
        }
        return Ok(json!({
            "content": content,
            "tool_calls": normalize_tool_calls(Some(&last_tool_calls), false),
        }));
    } else {
        // ── OpenAI 兼容协议（云模型，SSE 流式）──
        let url = provider::api_url(&p.base_url, "chat/completions");
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.1,
            "max_tokens": 1024
        });
        let bearer = p.api_key.trim().to_string();

        // 指数退避重试：仅对 429（限流）与 5xx（服务端临时故障）重试，最多 3 次。
        // 注意：重试发生在读取任何 delta 之前（连接/响应头阶段），避免重复输出。
        const MAX_ATTEMPTS: u32 = 3;
        let mut last_err = String::new();
        let mut resp: Option<reqwest::Response> = None;
        for attempt in 0..MAX_ATTEMPTS {
            let mut req = provider::cloud_http_client()
                .post(&url)
                .json(&body)
                .timeout(std::time::Duration::from_secs(120));
            if !bearer.is_empty() { req = req.bearer_auth(&bearer); }
            match req.send().await {
                Ok(r) if r.status().is_success() => { resp = Some(r); break; }
                Ok(r) => {
                    let status = r.status();
                    let code = status.as_u16();
                    let text = r.text().await.unwrap_or_default();
                    last_err = format!("{} HTTP {}: {}", p.name, status, text.chars().take(300).collect::<String>());
                    if code != 429 && !(500..600).contains(&code) { break; }
                }
                Err(e) => { last_err = format!("{} 请求失败: {}", p.name, e); break; }
            }
            if attempt + 1 >= MAX_ATTEMPTS { break; }
            let backoff_ms = 500u64 * (1u64 << attempt);
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        }
        let mut resp = resp.ok_or(last_err)?;

        let mut buf = String::new();
        let mut content = String::new();
        // 云流式 tool_calls 按 index 分片：id/name 首片给，arguments 逐片追加
        let mut tc_by_index: std::collections::BTreeMap<usize, Value> = std::collections::BTreeMap::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取 {} 响应失败: {}", p.name, e))? {
            // 流式输出期间也响应取消
            if let Some(a) = app {
                if a.try_state::<AgentControl>().map(|c| c.is_cancelled()).unwrap_or(false) { break; }
            }
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim();
                if line.is_empty() { continue; }
                let data = match line.strip_prefix("data:") { Some(d) => d.trim(), None => continue };
                if data == "[DONE]" { continue; }
                let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };
                let delta = &v["choices"][0]["delta"];
                if let Some(d) = delta["content"].as_str() {
                    if !d.is_empty() {
                        content.push_str(d);
                        if let Some(a) = app {
                            let _ = a.emit("agent://delta", json!({"text": content}));
                        }
                    }
                }
                if let Some(tcs) = delta["tool_calls"].as_array() {
                    for tc in tcs {
                        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                        let entry = tc_by_index.entry(idx).or_insert_with(|| json!({
                            "id": "", "type": "function",
                            "function": {"name": "", "arguments": ""}
                        }));
                        if let Some(id) = tc["id"].as_str() { if !id.is_empty() { entry["id"] = json!(id); } }
                        if let Some(name) = tc["function"]["name"].as_str() {
                            if !name.is_empty() { entry["function"]["name"] = json!(name); }
                        }
                        if let Some(args) = tc["function"]["arguments"].as_str() {
                            let cur = entry["function"]["arguments"].as_str().unwrap_or("").to_string();
                            entry["function"]["arguments"] = json!(format!("{}{}", cur, args));
                        }
                    }
                }
            }
        }
        // 组装成 OpenAI 兼容的 tool_calls 数组交给 normalize 解析
        let assembled: Vec<Value> = tc_by_index.into_values().collect();
        let normalized = normalize_tool_calls(Some(&assembled), true);
        return Ok(json!({"content": content, "tool_calls": normalized}));
    }
}

/// 把两种协议的 tool_calls 统一成
/// [{id, type:"function", function:{name, arguments: Value}}]
/// 保留 id 以便按 OpenAI 规范用 role:tool + tool_call_id 回填。
fn normalize_tool_calls(calls: Option<&Vec<Value>>, openai_style: bool) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(calls) = calls {
        for (idx, c) in calls.iter().enumerate() {
            let f = &c["function"];
            let name = f["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() { continue; }
            let args: Value = if openai_style {
                match f["arguments"].as_str() {
                    Some(s) if !s.trim().is_empty() => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                    _ => f["arguments"].clone(),
                }
            } else {
                f["arguments"].clone()
            };
            // Ollama 原生 tool_calls 通常无 id，生成一个稳定的占位 id
            let id = c["id"].as_str().filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("call_{}", idx));
            out.push(json!({
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": args}
            }));
        }
    }
    out
}

/// 一个被解析出的工具调用。id 仅原生调用有，用于云模型 role:tool 回填。
#[derive(Clone, Debug)]
struct ParsedCall {
    id: Option<String>,
    name: String,
    args: Value,
}

/// 批量解析模型输出，返回 (工具调用列表, 最终答案)。
/// 支持四种输出：原生多个 tool_calls / 文本多个 JSON 对象 / JSON 数组 / ReAct 单调用。
fn parse_model_calls(content: &str, native_calls: Option<&Vec<Value>>) -> Result<(Vec<ParsedCall>, Option<String>), String> {
    // 1) 原生 tool_calls —— 全部解析，不再只取第一个
    if let Some(calls) = native_calls {
        let parsed: Vec<ParsedCall> = calls.iter().filter_map(|c| {
            let f = &c["function"];
            let name = f["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() { return None; }
            Some(ParsedCall {
                id: c["id"].as_str().filter(|s| !s.is_empty()).map(String::from),
                name,
                args: f["arguments"].clone(),
            })
        }).collect();
        if !parsed.is_empty() {
            return Ok((parsed, None));
        }
    }

    let text = content.trim();

    // 2) ReAct 文本格式（放在 JSON 之前，避免 Action Input 里的对象被误判为工具调用）
    if let Some(name) = extract_after(text, "Action:") {
        let args = extract_after(text, "Action Input:")
            .and_then(|s| serde_json::from_str::<Value>(s.trim()).ok())
            .unwrap_or_else(|| json!({}));
        return Ok((vec![ParsedCall { id: None, name: name.trim().to_string(), args }], None));
    }

    // 3) 文本里的所有顶层 JSON 对象（自动展开顶层数组元素）
    let cleaned = text
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let mut calls: Vec<ParsedCall> = Vec::new();
    let mut answer: Option<String> = None;
    for obj_str in find_json_objects(cleaned) {
        let v: Value = match serde_json::from_str(&obj_str) { Ok(v) => v, Err(_) => continue };
        let items: Vec<Value> = if let Some(arr) = v.as_array() { arr.clone() } else { vec![v] };
        for item in items {
            if let Some(ans) = item.get("answer").and_then(|x| x.as_str()) {
                answer = Some(ans.to_string());
                continue;
            }
            if let Some(name) = item.get("name").or_else(|| item.get("tool")).and_then(|x| x.as_str()) {
                let args = item.get("arguments").or_else(|| item.get("args")).cloned().unwrap_or_else(|| json!({}));
                calls.push(ParsedCall { id: None, name: name.to_string(), args });
                continue;
            }
            // 字典式调用：{"mem_add": {"content": ...}} —— 本地小模型常见变体
            if let Some(obj) = item.as_object() {
                if obj.len() == 1 {
                    if let Some((k, val)) = obj.iter().next() {
                        if !matches!(k.as_str(), "answer" | "final" | "thought" | "result") && val.is_object() {
                            calls.push(ParsedCall { id: None, name: k.clone(), args: val.clone() });
                        }
                    }
                }
            }
        }
    }
    // 4) 都没有 → 整段文本当作最终答案
    if calls.is_empty() && answer.is_none() {
        answer = Some(text.to_string());
    }
    // 同时出现时都返回，由调用方优先消费 calls（模型通常不会混用，但保留信息更稳健）
    Ok((calls, answer))
}

/// 单调用兼容包装（供现有调用方与单测使用）。
fn parse_model_output(content: &str, native_calls: Option<&Vec<Value>>) -> Result<(Option<String>, Value, Option<String>), String> {
    let (calls, answer) = parse_model_calls(content, native_calls)?;
    match calls.into_iter().next() {
        Some(c) => Ok((Some(c.name), c.args, None)),
        None => Ok((None, json!({}), answer)),
    }
}

fn find_first_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if escape { escape = false; continue; }
        if c == '\\' && in_string { escape = true; continue; }
        if c == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { return Some(s[start..=i].to_string()); }
            }
            _ => {}
        }
    }
    None
}

/// 提取字符串中所有「顶层」JSON 对象（容忍前后解释文字、嵌套对象不重复提取）。
/// 用于一次解析模型返回的多个工具调用。
fn find_json_objects(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'{' { i += 1; continue; }
        let start = i;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        let mut end = None;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if escape { escape = false; }
            else if c == '\\' && in_string { escape = true; }
            else if c == '"' { in_string = !in_string; }
            else if !in_string {
                if c == '{' { depth += 1; }
                else if c == '}' {
                    depth -= 1;
                    if depth == 0 { end = Some(i); break; }
                }
            }
            i += 1;
        }
        match end {
            Some(e) => { out.push(s[start..=e].to_string()); i = e + 1; }
            None => break, // 未闭合，后面无需再找
        }
    }
    out
}

fn extract_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let pos = text.find(marker)? + marker.len();
    let rest = &text[pos..];
    Some(rest.lines().next().unwrap_or(rest).trim())
}

// ─── 工具执行分发 ──────────────────────────────────────────────

/// 自动执行策略：ask=全部确认，safe=内存整理自动/磁盘与危险操作确认，all=全部自动
#[derive(Clone, Copy, PartialEq, Debug)]
enum AutoMode { Ask, Safe, All }

fn auto_mode() -> AutoMode {
    // 默认 safe：内存整理无不可逆数据损失，直接执行；磁盘删除与命令/写入仍需确认
    match crate::db::get_setting_inner("agent_auto_mode").unwrap_or_default().as_str() {
        "ask" => AutoMode::Ask,
        "all" => AutoMode::All,
        _ => AutoMode::Safe,
    }
}

/// 判断一个工具在执行前是否需要弹窗确认。纯函数便于单测。
///
/// 按「资源类型」区分（取代旧的大小阈值策略）：
/// - `clean_memory`（内存整理）：只回收 inactive 缓存页，无不可逆数据损失 → safe 档直接放行
/// - `clean_temp_files`（磁盘）/ `run_command` / `write_file` / `delete_reminder`：safe 档仍需确认
/// - `ask` 档全部确认；`all` 档全部自动
fn needs_confirm(mode: AutoMode, name: &str) -> bool {
    if mode == AutoMode::All { return false; }
    if mode == AutoMode::Safe && name == "clean_memory" { return false; }
    true
}

async fn execute_tool(app: &AppHandle, name: &str, args: &Value) -> Result<String, String> {
    // ── 破坏性操作二次确认（human-in-the-loop）──
    // 这些工具会改变用户系统状态，执行前按策略决定是否由用户明确同意。
    // 内存整理（clean_memory）无不可逆损失 → safe 档自动放行；磁盘删除/命令/写入仍需确认。
    let mode = auto_mode();
    let confirm_desc = match name {
        "run_command" => Some(format!("执行命令：{}", args["command"].as_str().unwrap_or(""))),
        "write_file" => Some(format!("写入文件：{}（{}）",
            args["path"].as_str().unwrap_or(""),
            if args["append"].as_bool().unwrap_or(false) { "追加" } else { "覆盖" })),
        "delete_reminder" => Some(format!("删除提醒 #{}", args["id"].as_i64().unwrap_or(-1))),
        "clean_temp_files" => Some("清理系统临时文件（磁盘删除）".to_string()),
        "clean_memory" => Some("整理系统内存".to_string()),
        _ => None,
    };
    if let Some(desc) = confirm_desc {
        // needs_confirm 为 false 时短路，不进入等待，避免无谓弹窗
        if needs_confirm(mode, name) && !request_confirm(app, desc).await {
            return Err("用户取消了此操作".to_string());
        }
    }

    match name {
        "get_system_info" => {
            crate::modules::system::get_system_info().await.map(|v| v.to_string())
        }
        "list_processes" => {
            let limit = args["limit"].as_u64().unwrap_or(15) as u32;
            let list = crate::modules::system::get_processes(limit)?;
            Ok(json!(list).to_string())
        }
        "list_large_files" => {
            let threshold = args["threshold_mb"].as_f64().unwrap_or(100.0);
            let limit = args["limit"].as_u64().unwrap_or(10) as usize;
            let list = crate::modules::file::list_large_files(threshold, limit).await?;
            Ok(json!(list).to_string())
        }
        "scan_downloads" => {
            let threshold = args["threshold_mb"].as_f64();
            let list = crate::modules::file::scan_downloads(threshold).await?;
            Ok(json!(list).to_string())
        }
        "find_duplicates" => {
            let paths: Vec<String> = args["paths"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_else(|| vec![dirs::download_dir().unwrap_or_default().to_string_lossy().to_string()]);
            let groups = crate::modules::file::find_duplicates(paths).await?;
            Ok(json!(groups).to_string())
        }
        "clean_memory" => {
            let res = crate::modules::system::clean_memory().await?;
            Ok(res.to_string())
        }
        "clean_temp_files" => {
            let res = crate::modules::file::clean_temp_files().await?;
            Ok(res.to_string())
        }
        "list_services" => Ok(json!(crate::modules::service::list_services().await).to_string()),
        "probe_service" => {
            let id = args["service_id"].as_str().unwrap_or("ollama").to_string();
            Ok(json!(crate::modules::service::probe_service(id).await?).to_string())
        }
        "run_all_probes" => Ok(json!(crate::modules::service::run_all_probes().await).to_string()),
        "clip_list" => {
            let list = crate::modules::clipboard::clip_list().await?;
            Ok(json!(list).to_string())
        }
        "list_tasks" => Ok(json!(crate::modules::tasks::list_tasks().await?).to_string()),
        "add_task" => {
            let name = args["name"].as_str().unwrap_or("定时任务").to_string();
            let command = args["command"].as_str().unwrap_or("").to_string();
            let schedule = args["schedule"].as_str().unwrap_or("daily").to_string();
            crate::modules::tasks::add_task(name, command, schedule).await
        }
        "mem_list" => Ok(json!(crate::modules::memory::mem_list().await?).to_string()),
        "mem_add" => {
            let content = args["content"].as_str().unwrap_or("").to_string();
            let tag = args["tag"].as_str().map(String::from);
            let summary = args["summary"].as_str().map(String::from);
            crate::modules::memory::mem_add(content, tag, summary).await.map(|id| format!("已记住（id={}）", id))
        }
        "mem_search" => {
            let query = args["query"].as_str().unwrap_or("").to_string();
            let top_n = args["top_n"].as_u64().map(|n| n as usize);
            Ok(json!(crate::modules::memory::mem_search(query, top_n).await?).to_string())
        }
        "notify" => {
            let title = args["title"].as_str().unwrap_or("管家").to_string();
            let body = args["body"].as_str().unwrap_or("").to_string();
            crate::modules::notification::notify_push(app.clone(), title, body, None, None).await
        }
        "run_command" => {
            let command = args["command"].as_str().unwrap_or("").to_string();
            let cwd = args["cwd"].as_str().map(String::from);
            crate::modules::shell::run_command(command, cwd).await
        }
        "open_target" => {
            let target = args["target"].as_str().unwrap_or("").to_string();
            crate::modules::shell::open_target(target).await
        }
        "read_file" => {
            let path = args["path"].as_str().unwrap_or("").to_string();
            let max_chars = args["max_chars"].as_u64().map(|n| n as usize);
            crate::modules::shell::read_text_file(path, max_chars).await
        }
        "write_file" => {
            let path = args["path"].as_str().unwrap_or("").to_string();
            let content = args["content"].as_str().unwrap_or("").to_string();
            let append = args["append"].as_bool();
            crate::modules::shell::write_text_file(path, content, append).await
        }
        "set_reminder" => {
            let message = args["message"].as_str().unwrap_or("提醒").to_string();
            let when = args["when"].as_str().unwrap_or("").to_string();
            crate::modules::reminder::reminder_add(message, when).await
                .map(|id| format!("已设置提醒（id={}），到点会通知你", id))
        }
        "list_reminders" => Ok(json!(crate::modules::reminder::reminder_list().await?).to_string()),
        "delete_reminder" => {
            let id = args["id"].as_i64().unwrap_or(-1);
            crate::modules::reminder::reminder_delete(id).await.map(|_| "已删除提醒".to_string())
        }
        other => Err(format!("未知工具: {}", other)),
    }
}

// ─── 经验沉淀 ──────────────────────────────────────────────────

/// 把一条工具轨迹压成紧凑单行，供经验沉淀与历史回放复用。
/// 形如：list_large_files(threshold_mb=100) ✓ → 12 个文件
fn format_trace_brief(t: &ToolTrace) -> String {
    let args = t.args.as_object().map(|obj| {
        obj.iter().take(4).map(|(k, v)| {
            let val = match v {
                Value::String(s) => crate::modules::truncate_chars(s, 40).0,
                _ => v.to_string(),
            };
            format!("{}={}", k, crate::modules::truncate_chars(&val, 50).0)
        }).collect::<Vec<_>>().join(",")
    }).unwrap_or_default();
    let status = if t.ok { "✓" } else { "✗" };
    let result = if t.result.is_empty() { String::new() } else {
        format!(" → {}", crate::modules::truncate_chars(&t.result, 80).0)
    };
    format!("{}({}) {}{}", t.tool, args, status, result)
}

fn save_experience(intent: &str, traces: &[ToolTrace], success: bool) {
    if traces.is_empty() { return; }
    // 按 step 分组：同一 step 的多个工具是并行调用
    let mut by_step: std::collections::BTreeMap<usize, Vec<&ToolTrace>> = std::collections::BTreeMap::new();
    for t in traces {
        by_step.entry(t.step).or_default().push(t);
    }
    let mut lines = Vec::new();
    for (step, group) in by_step {
        let prefix = if group.len() > 1 { format!("{}并行:", step) } else { String::new() };
        let briefs: Vec<String> = group.iter().map(|t| format_trace_brief(t)).collect();
        lines.push(format!("{}{}", prefix, briefs.join(" + ")));
    }
    let path = lines.join("\n  ");
    let (tag, content) = if success {
        ("agent_experience", format!("用户意图「{}」→ 成功路径：\n  {}", intent, path))
    } else {
        let failed: Vec<String> = traces.iter().filter(|t| !t.ok).map(|t| t.tool.clone()).collect();
        let reason = if failed.is_empty() {
            "在限定步数内未收敛".to_string()
        } else {
            format!("失败工具：{}", failed.join(", "))
        };
        ("agent_failure", format!("用户意图「{}」→ 失败路径：\n  {}\n原因：{}。下次遇到类似意图应调整策略。", intent, path, reason))
    };
    let intent_owned = intent.to_string();
    let tag_owned = tag.to_string();
    let _ = std::thread::spawn(move || {
        with_db(|conn| {
            conn.execute(
                "INSERT INTO memories (content, created_at, tag, summary) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![content, chrono::Utc::now().to_rfc3339(), tag_owned, intent_owned],
            )?;
            Ok(())
        })
    }).join();
}

// ─── 主入口 ────────────────────────────────────────────────────

/// 把一批工具结果合并成一条 user 消息文本（本地 Ollama / 文本调用模式）。
fn format_results_as_text(results: &[(String, bool, String)]) -> String {
    if results.len() == 1 {
        let (name, ok, r) = &results[0];
        return format!("[工具结果] 工具 {} 执行{}：{}", name, if *ok {"成功"} else {"失败"}, r);
    }
    let mut s = format!("[工具结果] 本次并行执行了 {} 个工具：\n", results.len());
    for (i, (name, ok, r)) in results.iter().enumerate() {
        s.push_str(&format!("{}. {} — {}：{}\n", i + 1, name, if *ok {"成功"} else {"失败"}, r));
    }
    s
}

/// 把消息对象数组里的工具轨迹压成文本注入 assistant.content。
/// 模型由此能看到自己上次是怎么用工具解决问题的（few-shot），而非只看到最终答案。
/// 兼容两种轨迹格式：step 事件（status 字段）与 agent_chat 返回（ok 字段）。
fn enrich_history_traces(messages: &mut [Value]) {
    for m in messages.iter_mut() {
        if m.get("role").and_then(|v| v.as_str()) != Some("assistant") { continue; }
        let traces = match m.get("traces").and_then(|v| v.as_array()) {
            Some(t) if !t.is_empty() => t.clone(),
            _ => { m.as_object_mut().map(|o| o.remove("traces")); continue; }
        };
        let lines: Vec<String> = traces.iter().take(8).map(|t| {
            let tool = t.get("tool").and_then(|v| v.as_str()).unwrap_or("?");
            let args_obj = t.get("args").and_then(|v| v.as_object());
            let args = args_obj.map(|obj| {
                obj.iter().take(3).map(|(k, v)| {
                    let val = match v {
                        Value::String(s) => crate::modules::truncate_chars(s, 30).0,
                        _ => v.to_string(),
                    };
                    format!("{}={}", k, crate::modules::truncate_chars(&val, 40).0)
                }).collect::<Vec<_>>().join(",")
            }).unwrap_or_default();
            let ok = t.get("ok").and_then(|v| v.as_bool())
                .or_else(|| t.get("status").and_then(|v| v.as_str()).map(|s| s == "done"))
                .unwrap_or(true);
            let result = t.get("result").and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| format!(" → {}", crate::modules::truncate_chars(s, 60).0))
                .unwrap_or_default();
            format!("{}({}) {}{}", tool, args, if ok {"✓"} else {"✗"}, result)
        }).collect();
        let mut content = m.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !lines.is_empty() {
            content.push_str("\n\n[上次执行轨迹] ");
            content.push_str(&lines.join(" → "));
        }
        m["content"] = json!(content);
        m.as_object_mut().map(|o| o.remove("traces"));
    }
}

#[tauri::command]
pub async fn agent_chat(app: AppHandle, message: String, history: Option<Vec<Value>>) -> Result<AgentReply, String> {
    // 对话开始时读取一次模型引用，整轮迭代复用，避免每步 spawn 线程读 SQLite。
    let model_ref = model_ref();
    let display_model = parse_model_ref(&model_ref).1;
    let (successes, failures) = load_experiences(&message).await;
    // 每次新对话重置取消标志
    if let Some(ctrl) = app.try_state::<AgentControl>() { ctrl.reset(); }
    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt(&successes, &failures)}),
    ];
    if let Some(h) = history {
        let mut hist: Vec<Value> = h.into_iter().take(20).collect();
        enrich_history_traces(&mut hist);
        messages.extend(hist);
    }
    messages.push(json!({"role": "user", "content": message.clone()}));

    let mut traces: Vec<ToolTrace> = Vec::new();

    for step in 1..=MAX_STEPS {
        // 取消检查点：用户点「停止」后在下一个循环边界退出
        if app.try_state::<AgentControl>().map(|c| c.is_cancelled()).unwrap_or(false) {
            let answer = "已停止执行。".to_string();
            let _ = app.emit("agent://cancelled", json!({"steps": step - 1}));
            return Ok(AgentReply { answer, traces, steps: step - 1, model: display_model });
        }
        let resp = chat_complete(messages.clone(), &model_ref, Some(&app)).await?;
        // 流式读取中可能已被取消（此时 resp 为部分内容），统一在此收口
        if app.try_state::<AgentControl>().map(|c| c.is_cancelled()).unwrap_or(false) {
            let _ = app.emit("agent://cancelled", json!({"steps": step - 1}));
            return Ok(AgentReply { answer: "已停止执行。".to_string(), traces, steps: step - 1, model: display_model });
        }
        let content = resp["content"].as_str().unwrap_or("").to_string();
        let native = resp["tool_calls"].as_array().cloned();

        let (calls, answer) = parse_model_calls(&content, native.as_ref())?;

        // 没有工具调用 → 结束（answer 缺失时用整段文本兜底）
        if calls.is_empty() {
            let answer = answer.unwrap_or_else(|| content.clone());
            // 推送完成事件
            let _ = app.emit("agent://done", json!({"answer": answer, "steps": step}));
            // 有轨迹且全部成功才算成功经验；工具曾失败但模型给了答案，记为失败教训
            let all_ok = traces.iter().all(|t| t.ok);
            save_experience(&message, &traces, all_ok);
            return Ok(AgentReply {
                answer,
                traces,
                steps: step,
                model: display_model.clone(),
            });
        }

        let batch_size = calls.len();
        let is_local = parse_model_ref(&model_ref).0 == LOCAL_PROVIDER_ID;

        // 记录模型本轮的 assistant 消息——让模型在后续迭代里能看到自己调用过什么工具。
        // 原生 tool_calls 时带上全部调用（不再只留第一个），才能与后面的 role:tool 一一配对。
        if let Some(native_calls) = native.as_ref() {
            let mut assistant_msg = json!({"role": "assistant", "content": content});
            let tc_array: Vec<Value> = native_calls.iter().map(|c| {
                if is_local {
                    c.clone()
                } else {
                    // OpenAI 协议要求 function.arguments 为 JSON 字符串
                    let name = c["function"]["name"].as_str().unwrap_or("");
                    let args_val = &c["function"]["arguments"];
                    let args_str = if args_val.is_string() {
                        args_val.as_str().unwrap_or("{}").to_string()
                    } else {
                        args_val.to_string()
                    };
                    json!({
                        "id": c["id"].as_str().unwrap_or("call_0"),
                        "type": "function",
                        "function": {"name": name, "arguments": args_str}
                    })
                }
            }).collect();
            assistant_msg["tool_calls"] = json!(tc_array);
            messages.push(assistant_msg);
        } else {
            messages.push(json!({"role": "assistant", "content": content}));
        }

        // ── 并发执行全部工具调用 ──
        // JoinSet 要求 future 为 Send + 'static，因此克隆所需数据进 task；
        // 返回 (idx, ...) 以便按原始顺序回填结果与 traces。
        let mut set = tokio::task::JoinSet::new();
        for (idx, call) in calls.iter().enumerate() {
            let _ = app.emit("agent://step", json!({
                "step": step, "call_id": idx, "batch_total": batch_size,
                "tool": call.name, "args": call.args, "status": "running"
            }));
            let app2 = app.clone();
            let name = call.name.clone();
            let args = call.args.clone();
            set.spawn(async move {
                let started = std::time::Instant::now();
                let result = execute_tool(&app2, &name, &args).await;
                (idx, name, args, result, started.elapsed().as_millis() as u64)
            });
        }

        let mut outcomes: Vec<Option<(usize, String, Value, Result<String, String>, u64)>> = vec![None; batch_size];
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(v) => {
                    let idx = v.0;
                    if idx < outcomes.len() { outcomes[idx] = Some(v); }
                }
                Err(e) => eprintln!("[agent] 工具 task 异常: {}", e),
            }
        }
        // 用户可能在工具执行期间点了停止：本批结果照常回填，下一步循环边界统一收口
        if app.try_state::<AgentControl>().map(|c| c.is_cancelled()).unwrap_or(false) {
            let _ = app.emit("agent://cancelled", json!({"steps": step - 1}));
            return Ok(AgentReply { answer: "已停止执行。".to_string(), traces, steps: step - 1, model: display_model });
        }

        // ── 按原顺序回填结果 / traces ──
        let mut local_results: Vec<(String, bool, String)> = Vec::new();
        for outcome in outcomes.into_iter().enumerate().filter_map(|(pos, o)| o.map(|v| (pos, v))) {
            let (seq, (_, name, args, result, elapsed_ms)) = outcome;
            let (result_str, ok) = match result {
                Ok(v) => (v, true),
                Err(e) => (e, false),
            };
            // 截断过长的工具输出，避免撑爆上下文（按字符，避免中文 UTF-8 越界 panic）
            let (result_short, _) = crate::modules::truncate_chars(&result_str, 3000);

            traces.push(ToolTrace {
                step,
                call_id: Some(seq),
                tool: name.clone(),
                args: args.clone(),
                result: result_short.clone(),
                ok,
            });
            let _ = app.emit("agent://step", json!({
                "step": step, "call_id": seq, "batch_total": batch_size,
                "tool": name, "args": args,
                "status": if ok { "done" } else { "error" },
                "result": result_short, "elapsed_ms": elapsed_ms
            }));
            local_results.push((name, ok, result_short));
        }

        // 云模型 + 原生 tool_calls → 每条结果一个 role:tool + tool_call_id（OpenAI 规范）
        // 本地 Ollama / 文本调用 → 合并成一条 user 文本标记（实测兼容）
        if !is_local {
            let native_ids: Vec<String> = native.as_ref().map(|cs| cs.iter().map(|c| {
                c["id"].as_str().unwrap_or("call_0").to_string()
            }).collect()).unwrap_or_default();
            let use_role_tool = !native_ids.is_empty() && native_ids.len() == local_results.len();
            if use_role_tool {
                for (cid, (name, ok, result_short)) in native_ids.iter().zip(local_results.iter()) {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": cid,
                        "content": format!("工具 {} 执行{}：{}", name, if *ok {"成功"} else {"失败"}, result_short)
                    }));
                }
            } else {
                messages.push(json!({
                    "role": "user",
                    "content": format_results_as_text(&local_results)
                }));
            }
        } else {
            messages.push(json!({
                "role": "user",
                "content": format_results_as_text(&local_results)
            }));
        }
    }

    let answer = "抱歉，我在限定步数内没能完成任务。可能是工具结果不够明确，请换个说法或拆分指令再试。".to_string();
    let _ = app.emit("agent://done", json!({"answer": answer, "steps": MAX_STEPS}));
    save_experience(&message, &traces, false);
    Ok(AgentReply { answer, traces, steps: MAX_STEPS, model: display_model })
}

#[tauri::command]
pub fn agent_model_name() -> String {
    agent_model()
}

// ─── 模型选择 + 本机可用性验证 ────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelProbe {
    pub model: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub detail: String,
}

/// 探活：真发一次极简 chat 请求验证模型能否正常工作。
/// `model_ref` 为 `provider_id::model_name` 复合引用（纯模型名视为本地 Ollama）。
async fn probe_model(model_ref: &str) -> ModelProbe {
    let (provider_id, model) = parse_model_ref(model_ref);
    let p = match resolve_provider(&provider_id) {
        Ok(p) => p,
        Err(e) => return ModelProbe {
            model: model_ref.to_string(), ok: false, latency_ms: 0, detail: e,
        },
    };
    let is_local = p.id == LOCAL_PROVIDER_ID;
    let (url, body, bearer) = if is_local {
        (
            format!("{}/api/chat", provider::normalize_base_url(&p.base_url)),
            json!({"model": model, "messages": [{"role":"user","content":"hi"}], "stream": false, "options": {"num_predict": 1, "temperature": 0}}),
            String::new(),
        )
    } else {
        (
            provider::api_url(&p.base_url, "chat/completions"),
            json!({"model": model, "messages": [{"role":"user","content":"hi"}], "stream": false, "max_tokens": 1, "temperature": 0}),
            p.api_key.trim().to_string(),
        )
    };

    let started = std::time::Instant::now();
    let latency = || started.elapsed().as_millis() as u64;
    let mut req = if is_local { ollama_http_client() } else { provider::cloud_http_client() }
        .post(&url).json(&body).timeout(std::time::Duration::from_secs(60));
    if !bearer.is_empty() { req = req.bearer_auth(&bearer); }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return ModelProbe {
            model: model_ref.to_string(), ok: false, latency_ms: latency(),
            detail: format!("无法连接 {}：{}", p.name, e),
        },
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let reason = if status.as_u16() == 404 || status.as_u16() == 400 {
            format!("模型不存在或不被接受（HTTP {}）：{}", status, text.chars().take(120).collect::<String>())
        } else {
            format!("{} 返回 HTTP {}：{}", p.name, status, text.chars().take(120).collect::<String>())
        };
        return ModelProbe { model: model_ref.to_string(), ok: false, latency_ms: latency(), detail: reason };
    }
    if is_local {
        match serde_json::from_str::<Value>(&text) {
            Ok(v) if v.get("message").is_some() || v.get("response").is_some() => ModelProbe {
                model: model_ref.to_string(), ok: true, latency_ms: latency(), detail: "可用 · 响应正常".into(),
            },
            Ok(v) => ModelProbe {
                model: model_ref.to_string(), ok: false, latency_ms: latency(),
                detail: format!("响应格式异常：{}", v.to_string().chars().take(80).collect::<String>()),
            },
            Err(_) => ModelProbe { model: model_ref.to_string(), ok: false, latency_ms: latency(), detail: "响应解析失败".into() },
        }
    } else {
        match serde_json::from_str::<Value>(&text) {
            Ok(v) if v.get("choices").and_then(|c| c.as_array()).map(|a| !a.is_empty()).unwrap_or(false) => ModelProbe {
                model: model_ref.to_string(), ok: true, latency_ms: latency(), detail: "可用 · 响应正常".into(),
            },
            Ok(v) => ModelProbe {
                model: model_ref.to_string(), ok: false, latency_ms: latency(),
                detail: format!("响应格式异常：{}", v.to_string().chars().take(120).collect::<String>()),
            },
            Err(_) => ModelProbe { model: model_ref.to_string(), ok: false, latency_ms: latency(), detail: "响应解析失败".into() },
        }
    }
}

/// 列出所有可用模型：本机 Ollama + 所有启用的云 provider。
/// 每项含 `ref`（provider::model，用于探活/设置）、`source`（本机/云服务商名）、`size`。
/// 可用性由前端按需逐个探活，避免并发加载多个大模型或触发大量云请求。
#[tauri::command]
pub async fn agent_list_models() -> Result<Vec<Value>, String> {
    let current_ref = model_ref();
    let mut out: Vec<Value> = Vec::new();

    // 1) 本机 Ollama
    let url = format!("{}/api/tags", ollama_url());
    if let Ok(resp) = ollama_http_client().get(&url).timeout(std::time::Duration::from_secs(8)).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                let models = body.get("models").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                for m in models {
                    let name = m["name"].as_str().unwrap_or("").to_string();
                    if name.is_empty() { continue; }
                    let r = make_ref(LOCAL_PROVIDER_ID, &name);
                    out.push(json!({
                        "name": name, "ref": r, "source": "本机 Ollama", "kind": "local",
                        "size": m["size"].as_u64().unwrap_or(0), "current": r == current_ref,
                    }));
                }
            }
        }
    }

    // 2) 云 providers（顺序拉取，单个失败不影响其它；provider 数量少，无需并发）
    let providers: Vec<Provider> = provider::provider_list_full()?.into_iter().filter(|p| p.enabled).collect();
    for p in &providers {
        match provider::fetch_cloud_models(&p.base_url, &p.api_key).await {
            Ok(ids) => for name in ids {
                let r = make_ref(&p.id, &name);
                out.push(json!({
                    "name": name, "ref": r, "source": p.name.clone(), "kind": "cloud",
                    "size": 0, "current": r == current_ref,
                }));
            },
            Err(e) => out.push(json!({
                "name": format!("⚠️ 连接失败：{}", e.chars().take(50).collect::<String>()),
                "ref": make_ref(&p.id, ""), "source": p.name.clone(), "kind": "error",
                "size": 0, "current": false,
            })),
        }
    }
    Ok(out)
}

/// 探活单个模型（前端选中时调用，确认能正常使用）
#[tauri::command]
pub async fn agent_probe_model(model: String) -> Result<ModelProbe, String> {
    Ok(probe_model(&model).await)
}

/// 设置 Agent 模型偏好：先探活，通过才写入 SQLite
#[tauri::command]
pub async fn agent_set_model(model: String) -> Result<ModelProbe, String> {
    let result = probe_model(&model).await;
    if !result.ok {
        return Ok(result); // 返回失败原因，前端据此提示且不切换
    }
    let pref = model.clone();
    let saved = std::thread::spawn(move || {
        with_db(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![MODEL_PREF_KEY, pref],
            )?;
            Ok(())
        })
    })
    .join()
    .map_err(|_| "保存偏好失败".to_string())?;
    saved.map_err(|e| format!("保存偏好失败：{}", e))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_multiple_results_includes_all_tools() {
        let results = vec![
            ("get_system_info".to_string(), true, "ok1".to_string()),
            ("list_processes".to_string(), false, "err2".to_string()),
            ("mem_list".to_string(), true, "ok3".to_string()),
        ];
        let text = format_results_as_text(&results);
        assert!(text.contains("3 个工具"));
        assert!(text.contains("1. get_system_info — 成功：ok1"));
        assert!(text.contains("2. list_processes — 失败：err2"));
        assert!(text.contains("3. mem_list — 成功：ok3"));
    }

    #[test]
    fn parse_multiple_native_tool_calls() {
        let calls = vec![
            json!({"id": "call_1", "function": {"name": "get_system_info", "arguments": {}}}),
            json!({"id": "call_2", "function": {"name": "list_processes", "arguments": {"limit": 5}}}),
            json!({"id": "call_3", "function": {"name": "mem_search", "arguments": {"query": "x"}}}),
        ];
        let (parsed, answer) = parse_model_calls("", Some(&calls)).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name, "get_system_info");
        assert_eq!(parsed[0].id.as_deref(), Some("call_1"));
        assert_eq!(parsed[1].name, "list_processes");
        assert_eq!(parsed[1].args["limit"], 5);
        assert_eq!(parsed[2].id.as_deref(), Some("call_3"));
        assert!(answer.is_none());
    }

    #[test]
    fn parse_multiple_text_tool_calls() {
        let text = r#"我先并行查一下系统和进程。
{"name": "get_system_info", "arguments": {}}
然后查进程：
{"name": "list_processes", "arguments": {"limit": 8}}
"#;
        let (parsed, answer) = parse_model_calls(text, None).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "get_system_info");
        assert_eq!(parsed[1].name, "list_processes");
        assert_eq!(parsed[1].args["limit"], 8);
        assert!(answer.is_none());
    }

    #[test]
    fn parse_json_array_tool_calls() {
        let text = r#"[{"name":"mem_list","arguments":{}},{"name":"list_tasks","arguments":{}}]"#;
        let (parsed, answer) = parse_model_calls(text, None).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "mem_list");
        assert_eq!(parsed[1].name, "list_tasks");
        assert!(answer.is_none());
    }

    #[test]
    fn find_json_objects_skips_nested_and_keeps_top_level() {
        let text = r#"{"name":"mem_add","arguments":{"content":"嵌套{\"a\":1}","tag":"t"}} 后面 {"answer":"完成"}"#;
        let objs = find_json_objects(text);
        assert_eq!(objs.len(), 2, "只取两个顶层对象，嵌套的 arguments 不应被拆出");
        let (calls, answer) = parse_model_calls(text, None).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "mem_add");
        assert_eq!(answer.as_deref(), Some("完成"));
    }

    #[test]
    fn parse_native_tool_call() {
        let calls = vec![json!({"function": {"name": "list_processes", "arguments": {"limit": 10}}})];
        let (name, args, answer) = parse_model_output("", Some(&calls)).unwrap();
        assert_eq!(name.as_deref(), Some("list_processes"));
        assert_eq!(args["limit"], 10);
        assert!(answer.is_none());
    }

    #[test]
    fn parse_json_in_content() {
        let text = r#"{"name": "list_large_files", "arguments": {"threshold_mb": 100, "limit": 10}}"#;
        let (name, args, answer) = parse_model_output(text, None).unwrap();
        assert_eq!(name.as_deref(), Some("list_large_files"));
        assert_eq!(args["threshold_mb"], 100);
        assert!(answer.is_none());
    }

    #[test]
    fn parse_json_with_surrounding_text() {
        let text = "好的，我来查一下。\n{\"name\": \"list_services\", \"arguments\": {}}\n希望对你有帮助。";
        let (name, _args, _) = parse_model_output(text, None).unwrap();
        assert_eq!(name.as_deref(), Some("list_services"));
    }

    #[test]
    fn parse_markdown_wrapped_json() {
        let text = "```json\n{\"name\": \"mem_add\", \"arguments\": {\"content\": \"test\"}}\n```";
        let (name, args, _) = parse_model_output(text, None).unwrap();
        assert_eq!(name.as_deref(), Some("mem_add"));
        assert_eq!(args["content"], "test");
    }

    #[test]
    fn parse_direct_answer() {
        let text = "{\"answer\": \"你好，我可以帮你管理这台电脑。\"}";
        let (name, _args, answer) = parse_model_output(text, None).unwrap();
        assert!(name.is_none());
        assert_eq!(answer.unwrap(), "你好，我可以帮你管理这台电脑。");
    }

    #[test]
    fn parse_plain_text_as_answer() {
        let text = "这是一段普通回答，没有工具调用。";
        let (name, _args, answer) = parse_model_output(text, None).unwrap();
        assert!(name.is_none());
        assert_eq!(answer.unwrap(), text);
    }

    #[test]
    fn parse_react_action() {
        let text = "Thought: 需要查看进程\nAction: list_processes\nAction Input: {\"limit\": 20}";
        let (name, args, _) = parse_model_output(text, None).unwrap();
        assert_eq!(name.as_deref(), Some("list_processes"));
        assert_eq!(args["limit"], 20);
    }

    #[test]
    fn parse_dict_style_tool_call() {
        // qwen2.5-coder 常见变体：把工具名直接当 key
        let text = r#"{"mem_add": {"content": "我喜欢深色主题", "tag": "preference", "summary": "主题偏好"}}"#;
        let (name, args, answer) = parse_model_output(text, None).unwrap();
        assert_eq!(name.as_deref(), Some("mem_add"));
        assert_eq!(args["content"], "我喜欢深色主题");
        assert_eq!(args["tag"], "preference");
        assert!(answer.is_none());
    }

    #[test]
    fn parse_dict_style_with_markdown() {
        let text = "```json\n{\"notify\": {\"title\": \"完成\", \"body\": \"检查完毕\"}}\n```";
        let (name, args, _) = parse_model_output(text, None).unwrap();
        assert_eq!(name.as_deref(), Some("notify"));
        assert_eq!(args["title"], "完成");
    }

    #[test]
    fn enrich_history_injects_traces_and_removes_raw_field() {
        let mut msgs = vec![
            json!({"role": "user", "content": "查一下进程"}),
            json!({
                "role": "assistant", "content": "有 48 个进程",
                "traces": [
                    {"step": 1, "tool": "list_processes", "args": {"limit": 5}, "status": "done", "result": "48 个进程"},
                    {"step": 2, "tool": "run_command", "args": {"command": "uptime"}, "status": "error", "result": "失败"}
                ]
            }),
        ];
        enrich_history_traces(&mut msgs);
        let c = msgs[1]["content"].as_str().unwrap();
        assert!(c.contains("有 48 个进程"));
        assert!(c.contains("[上次执行轨迹]"));
        assert!(c.contains("list_processes(limit=5) ✓"));
        assert!(c.contains("run_command(command=uptime) ✗"));
        assert!(msgs[1].get("traces").is_none(), "原始 traces 字段应被移除");
        assert!(msgs[0].get("traces").is_none());
    }

    #[test]
    fn enrich_history_accepts_ok_field_from_agent_reply() {
        let mut msgs = vec![json!({
            "role": "assistant", "content": "ok",
            "traces": [{"step": 1, "tool": "mem_list", "args": {}, "ok": false, "result": "err"}]
        })];
        enrich_history_traces(&mut msgs);
        let c = msgs[0]["content"].as_str().unwrap();
        assert!(c.contains("mem_list() ✗"));
    }

    #[test]
    fn auto_mode_respects_confirmation_policies() {
        // ask：全部确认，包括内存整理
        assert!(needs_confirm(AutoMode::Ask, "clean_memory"));
        assert!(needs_confirm(AutoMode::Ask, "clean_temp_files"));
        assert!(needs_confirm(AutoMode::Ask, "run_command"));
        // all：全部自动
        assert!(!needs_confirm(AutoMode::All, "run_command"));
        assert!(!needs_confirm(AutoMode::All, "clean_temp_files"));
        assert!(!needs_confirm(AutoMode::All, "clean_memory"));
        // safe（默认）：内存整理自动放行，磁盘删除与危险操作仍需确认
        assert!(!needs_confirm(AutoMode::Safe, "clean_memory"));
        assert!(needs_confirm(AutoMode::Safe, "clean_temp_files"));
        assert!(needs_confirm(AutoMode::Safe, "run_command"));
        assert!(needs_confirm(AutoMode::Safe, "write_file"));
        assert!(needs_confirm(AutoMode::Safe, "delete_reminder"));
    }

    #[test]
    fn hybrid_score_uses_semantic_when_available() {
        // 语义高、词面 0：应接近 0.7
        let s = hybrid_score(Some(0.95), 0.0);
        assert!((s - 0.665).abs() < 0.001, "实际 {}", s);
        // 语义不可用：退化为纯词面
        assert_eq!(hybrid_score(None, 0.4), 0.4);
        assert_eq!(hybrid_score(Some(-1.0), 0.25), 0.25);
    }

    #[test]
    fn format_trace_brief_is_compact_and_keeps_status() {
        let t = ToolTrace {
            step: 1,
            call_id: Some(0),
            tool: "list_large_files".to_string(),
            args: json!({"threshold_mb": 100, "limit": 10, "extra": "x"}),
            result: "找到了 12 个大文件".to_string(),
            ok: true,
        };
        let s = format_trace_brief(&t);
        assert!(s.starts_with("list_large_files("));
        assert!(s.contains("threshold_mb=100"));
        assert!(s.contains("✓"));
        assert!(s.contains("找到了 12 个大文件"));
        assert!(s.len() < 180, "过长：{}", s);
    }

    #[test]
    fn relevance_ranks_related_intent_higher() {
        let intent = "帮我看看下载目录里的大文件";
        let related = "用户意图「扫描下载目录大文件」→ 使用工具 [scan_downloads] 成功完成";
        let unrelated = "用户意图「记住我喜欢深色主题」→ 使用工具 [mem_add] 成功完成";
        let r1 = relevance(intent, related);
        let r2 = relevance(intent, unrelated);
        assert!(r1 > r2, "related={} unrelated={}", r1, r2);
        assert!(r1 > 0.1, "related too low: {}", r1);
    }

    #[test]
    fn bigrams_handle_chinese() {
        let set = text_bigrams("记忆");
        assert!(set.contains("记忆"));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn find_nested_json() {
        let text = "前缀 {\"name\":\"x\",\"arguments\":{\"a\":{\"b\":1}}} 后缀";
        let obj = find_first_json_object(text).unwrap();
        let v: Value = serde_json::from_str(&obj).unwrap();
        assert_eq!(v["arguments"]["a"]["b"], 1);
    }

    #[test]
    fn model_ref_parse_roundtrip() {
        assert_eq!(parse_model_ref("ollama::qwen2.5:3b"), ("ollama".into(), "qwen2.5:3b".into()));
        assert_eq!(parse_model_ref("p123_456::deepseek-chat"), ("p123_456".into(), "deepseek-chat".into()));
        // 旧值（纯模型名）向后兼容为本地
        assert_eq!(parse_model_ref("qwen2.5:3b"), ("ollama".into(), "qwen2.5:3b".into()));
    }

    #[test]
    fn normalize_openai_tool_calls_parses_arguments_string() {
        // OpenAI 的 arguments 是 JSON 字符串
        let calls = vec![json!({"id":"call_1","function":{"name":"mem_add","arguments":"{\"content\":\"记住我\"}"}})];
        let out = normalize_tool_calls(Some(&calls), true);
        assert_eq!(out[0]["function"]["name"], "mem_add");
        assert_eq!(out[0]["function"]["arguments"]["content"], "记住我");
    }

    #[test]
    fn normalize_ollama_tool_calls_keeps_object_arguments() {
        // Ollama 的 arguments 已是对象
        let calls = vec![json!({"function":{"name":"list_tasks","arguments":{"limit":5}}})];
        let out = normalize_tool_calls(Some(&calls), false);
        assert_eq!(out[0]["function"]["arguments"]["limit"], 5);
    }

    #[test]
    fn normalize_tool_calls_skips_invalid() {
        let calls = vec![json!({"function":{"name":"","arguments":{}}}), json!({"function":{"name":"ok","arguments":"bad-json"}})];
        let out = normalize_tool_calls(Some(&calls), true);
        assert_eq!(out.len(), 1); // 空名跳过；坏 JSON 退化为 {} 但保留
        assert_eq!(out[0]["function"]["arguments"], json!({}));
    }
}

