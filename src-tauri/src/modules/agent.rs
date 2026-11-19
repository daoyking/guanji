//! M8: AI Agent 编排核心 — 意图理解 → 工具调用 → 观察 → 迭代 → 回答
//! 兼容本地小模型的三种工具调用输出：原生 tool_calls / JSON 文本 / ReAct Action。
//! 自主进化：历史执行轨迹作为 few-shot 注入 system prompt，越用越懂你。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use crate::db::with_db;
use crate::modules::provider::{self, Provider, LOCAL_PROVIDER_ID};

const MAX_STEPS: usize = 8;

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
    provider::provider_list()?
        .into_iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("服务商 {} 已不存在，请重新选择模型", provider_id))
}

// ─── 对外数据结构 ──────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolTrace {
    pub step: usize,
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
        tool("clean_temp_files", "清理系统临时文件（谨慎使用，会删除临时文件）", json!({})),
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

/// 按与当前意图的相关度检索历史经验，分离「成功做法」与「失败教训」
fn load_experiences(intent: &str) -> (Vec<String>, Vec<String>) {
    let rows = std::thread::spawn(|| {
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
    .join()
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();

    let mut successes: Vec<(f32, String)> = Vec::new();
    let mut failures: Vec<(f32, String)> = Vec::new();
    for (tag, content, summary) in rows {
        let score = relevance(intent, &summary);
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
1. 分析用户意图，选择最合适的工具（可连续调用多个工具完成复杂任务）。
2. 每次只输出一个工具调用；拿到结果后再决定下一步。
3. 信息足够时，用简洁中文给出最终答案（不要复述工具原始输出）。

## 工具调用格式（严格遵守）
只输出一个 JSON 对象，不要输出其它任何文字。必须是下面这种带 name / arguments 两个键的格式：
{{"name": "工具名", "arguments": {{...}}}}
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
- 需要真正执行系统操作时用 run_command（危险命令会被自动拦截，可放心尝试）。
- 文件删除、清理类操作前先扫描确认，再执行，并在回答中说清楚做了什么。
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
async fn chat_complete(messages: Vec<Value>) -> Result<Value, String> {
    let (provider_id, model) = parse_model_ref(&model_ref());
    let p = resolve_provider(&provider_id)?;
    let tools = tools_schema();

    if p.id == LOCAL_PROVIDER_ID {
        // ── 本机 Ollama 原生协议 ──
        let url = format!("{}/api/chat", provider::normalize_base_url(&p.base_url));
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
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
        let body: Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
        let msg = body["message"].clone();
        Ok(json!({
            "content": msg["content"].as_str().unwrap_or(""),
            "tool_calls": normalize_tool_calls(msg["tool_calls"].as_array(), false),
        }))
    } else {
        // ── OpenAI 兼容协议（云模型）──
        let url = provider::api_url(&p.base_url, "chat/completions");
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.1,
            "max_tokens": 1024
        });
        let mut req = provider::cloud_http_client()
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(120));
        if !p.api_key.trim().is_empty() {
            req = req.bearer_auth(p.api_key.trim());
        }
        let resp = req.send().await.map_err(|e| format!("{} 请求失败: {}", p.name, e))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{} HTTP {}: {}", p.name, status, text.chars().take(300).collect::<String>()));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("响应解析失败：{}", e))?;
        let msg = &v["choices"][0]["message"];
        if msg.is_null() {
            return Err(format!("响应缺少 choices[0].message：{}", text.chars().take(200).collect::<String>()));
        }
        Ok(json!({
            "content": msg["content"].as_str().unwrap_or(""),
            // OpenAI 的 arguments 是 JSON 字符串，需解析为对象
            "tool_calls": normalize_tool_calls(msg["tool_calls"].as_array(), true),
        }))
    }
}

/// 把两种协议的 tool_calls 统一成 [{function:{name, arguments: Value}}]
fn normalize_tool_calls(calls: Option<&Vec<Value>>, openai_style: bool) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(calls) = calls {
        for c in calls {
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
            out.push(json!({"function": {"name": name, "arguments": args}}));
        }
    }
    out
}

/// 兼容三种工具调用输出，统一解析为 (tool_name, args) 或最终答案
fn parse_model_output(content: &str, native_calls: Option<&Vec<Value>>) -> Result<(Option<String>, Value, Option<String>), String> {
    // 1) 原生 tool_calls
    if let Some(calls) = native_calls {
        if let Some(first) = calls.first() {
            let f = &first["function"];
            let name = f["name"].as_str().unwrap_or("").to_string();
            let args = f["arguments"].clone();
            if !name.is_empty() {
                return Ok((Some(name), args, None));
            }
        }
    }

    let text = content.trim();
    // 2) 剥离 markdown 代码块
    let cleaned = text
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // 3) 从文本中提取第一个 JSON 对象（容忍模型前后加解释文字）
    let json_candidate = if cleaned.starts_with('{') {
        find_first_json_object(cleaned)
    } else {
        find_first_json_object(text)
    };

    if let Some(obj_str) = json_candidate {
        if let Ok(v) = serde_json::from_str::<Value>(&obj_str) {
            if let Some(ans) = v.get("answer").and_then(|x| x.as_str()) {
                return Ok((None, json!({}), Some(ans.to_string())));
            }
            if let Some(name) = v.get("name").or_else(|| v.get("tool")).and_then(|x| x.as_str()) {
                let args = v.get("arguments").or_else(|| v.get("args")).cloned().unwrap_or_else(|| json!({}));
                return Ok((Some(name.to_string()), args, None));
            }
            // 3b) 字典式调用：{"mem_add": {"content": ...}} —— 本地小模型常见变体
            if let Some(obj) = v.as_object() {
                if obj.len() == 1 {
                    if let Some((k, val)) = obj.iter().next() {
                        // 排除 answer/final/thought 等语义键，只把「值为对象」的单键当工具名
                        if !matches!(k.as_str(), "answer" | "final" | "thought" | "result") && val.is_object() {
                            return Ok((Some(k.clone()), val.clone(), None));
                        }
                    }
                }
            }
        }
    }

    // 4) ReAct 文本格式：Action: xxx / Action Input: {...}
    if let Some(name) = extract_after(text, "Action:") {
        let args = extract_after(text, "Action Input:")
            .and_then(|s| serde_json::from_str::<Value>(s.trim()).ok())
            .unwrap_or_else(|| json!({}));
        return Ok((Some(name.trim().to_string()), args, None));
    }

    // 5) 都不匹配 → 整段文本当作最终答案
    Ok((None, json!({}), Some(text.to_string())))
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

fn extract_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let pos = text.find(marker)? + marker.len();
    let rest = &text[pos..];
    Some(rest.lines().next().unwrap_or(rest).trim())
}

// ─── 工具执行分发 ──────────────────────────────────────────────

async fn execute_tool(app: &AppHandle, name: &str, args: &Value) -> Result<String, String> {
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

fn save_experience(intent: &str, traces: &[ToolTrace], success: bool) {
    let tools: Vec<String> = traces.iter().map(|t| t.tool.clone()).collect();
    if tools.is_empty() { return; }
    let (tag, content) = if success {
        (
            "agent_experience",
            format!("用户意图「{}」→ 使用工具 [{}] 成功完成", intent, tools.join(" → ")),
        )
    } else {
        // 失败复盘：记录踩坑点，供后续同类意图规避
        let failed_tools: Vec<String> = traces.iter()
            .filter(|t| !t.ok)
            .map(|t| format!("{}（{}）", t.tool,
                t.result.chars().take(60).collect::<String>()))
            .collect();
        let reason = if failed_tools.is_empty() {
            "在限定步数内未收敛".to_string()
        } else {
            format!("工具执行失败：{}", failed_tools.join("; "))
        };
        (
            "agent_failure",
            format!("用户意图「{}」→ 工具链 [{}] 失败，原因：{}。下次遇到类似意图应调整策略。",
                intent, tools.join(" → "), reason),
        )
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

#[tauri::command]
pub async fn agent_chat(app: AppHandle, message: String, history: Option<Vec<Value>>) -> Result<AgentReply, String> {
    let (successes, failures) = load_experiences(&message);
    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt(&successes, &failures)}),
    ];
    if let Some(h) = history {
        for m in h.into_iter().take(20) { messages.push(m); }
    }
    messages.push(json!({"role": "user", "content": message.clone()}));

    let mut traces: Vec<ToolTrace> = Vec::new();

    for step in 1..=MAX_STEPS {
        let resp = chat_complete(messages.clone()).await?;
        let content = resp["content"].as_str().unwrap_or("").to_string();
        let native = resp["tool_calls"].as_array().cloned();

        let (tool_name, args, answer) = parse_model_output(&content, native.as_ref())?;

        // 模型直接给出答案 → 结束
        if tool_name.is_none() {
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
                model: agent_model(),
            });
        }

        let name = tool_name.unwrap();
        let _ = app.emit("agent://step", json!({
            "step": step, "tool": name, "args": args, "status": "running"
        }));

        let started = std::time::Instant::now();
        let result = execute_tool(&app, &name, &args).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let (result_str, ok) = match result {
            Ok(v) => (v, true),
            Err(e) => (e, false),
        };
        // 截断过长的工具输出，避免撑爆上下文
        let result_short = if result_str.len() > 3000 {
            format!("{}…（已截断，共{}字符）", &result_str[..3000], result_str.len())
        } else {
            result_str.clone()
        };

        traces.push(ToolTrace {
            step,
            tool: name.clone(),
            args: args.clone(),
            result: result_short.clone(),
            ok,
        });
        let _ = app.emit("agent://step", json!({
            "step": step, "tool": name, "args": args,
            "status": if ok { "done" } else { "error" },
            "result": result_short, "elapsed_ms": elapsed_ms
        }));

        // 把工具结果回填给模型，继续迭代。
        // 用 user 角色 + 明确标记，而非 role:tool——后者在 OpenAI 协议下要求与上一条
        // assistant.tool_calls 用 tool_call_id 严格配对，云模型若以文本输出工具调用会 400。
        // 文本回填对本地 Ollama 与所有 OpenAI 兼容端点都安全。
        messages.push(json!({
            "role": "user",
            "content": format!("[工具结果] 工具 {} 执行{}：{}", name, if ok {"成功"} else {"失败"}, result_short)
        }));
    }

    let answer = "抱歉，我在限定步数内没能完成任务。可能是工具结果不够明确，请换个说法或拆分指令再试。".to_string();
    let _ = app.emit("agent://done", json!({"answer": answer, "steps": MAX_STEPS}));
    save_experience(&message, &traces, false);
    Ok(AgentReply { answer, traces, steps: MAX_STEPS, model: agent_model() })
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
    let providers: Vec<Provider> = provider::provider_list()?.into_iter().filter(|p| p.enabled).collect();
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

