//! M8: AI Agent 编排核心 — 意图理解 → 工具调用 → 观察 → 迭代 → 回答
//! 兼容本地小模型的三种工具调用输出：原生 tool_calls / JSON 文本 / ReAct Action。
//! 自主进化：历史执行轨迹作为 few-shot 注入 system prompt，越用越懂你。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use crate::db::with_db;

const MAX_STEPS: usize = 8;

fn ollama_url() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
}
fn agent_model() -> String {
    std::env::var("GUANJI_AGENT_MODEL").unwrap_or_else(|_| "qwen2.5-coder:7b".to_string())
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

fn load_experiences() -> Vec<String> {
    std::thread::spawn(|| {
        with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT content FROM memories WHERE tag='agent_experience' \
                 ORDER BY created_at DESC LIMIT 6",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    })
    .join()
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default()
}

fn system_prompt(experiences: &[String]) -> String {
    let exp_block = if experiences.is_empty() {
        "尚无历史经验，这是第一次协作。".to_string()
    } else {
        format!("你过去的成功经验（供参考，可直接复用相同工具组合）：\n{}",
            experiences.iter().map(|e| format!("- {}", e)).collect::<Vec<_>>().join("\n"))
    };
    format!(r#"你是「管家」，一台 macOS 电脑的全能 AI Agent。用户用中文下达指令，你通过调用工具真正把事情做完，而不是只给建议。

## 工作方法
1. 分析用户意图，选择最合适的工具（可连续调用多个工具完成复杂任务）。
2. 每次只输出一个工具调用；拿到结果后再决定下一步。
3. 信息足够时，用简洁中文给出最终答案（不要复述工具原始输出）。

## 工具调用格式（严格遵守）
只输出一个 JSON 对象，不要输出其它任何文字：
{{"name": "工具名", "arguments": {{...}}}}
若无需调用工具、可直接回答，输出：
{{"answer": "你的最终回答"}}

## 可用工具
{tools}

## {exp_block}

## 行为准则
- 用户说「记住/别忘了」时，用 mem_add 工具记下来。
- 文件删除、清理类操作前先扫描确认，再执行，并在回答中说清楚做了什么。
- 任务完成后，如果结果重要，用 notify 工具告知用户。
- 回答用中文，简洁、直接、有条理。"#,
        tools = serde_json::to_string_pretty(&tools_schema()).unwrap_or_default(),
        exp_block = exp_block,
    )
}

// ─── Ollama 调用 ───────────────────────────────────────────────

async fn chat_with_ollama(messages: Vec<Value>) -> Result<Value, String> {
    let url = format!("{}/api/chat", ollama_url());
    let body = json!({
        "model": agent_model(),
        "messages": messages,
        "stream": false,
        "options": {"temperature": 0.1, "num_predict": 512}
    });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("Ollama 请求失败（请确认 Ollama 已启动）: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Ollama HTTP {}", resp.status()));
    }
    resp.json::<Value>().await.map_err(|e| format!("解析响应失败: {}", e))
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
        other => Err(format!("未知工具: {}", other)),
    }
}

// ─── 经验沉淀 ──────────────────────────────────────────────────

fn save_experience(intent: &str, traces: &[ToolTrace]) {
    let tools: Vec<String> = traces.iter().map(|t| t.tool.clone()).collect();
    if tools.is_empty() { return; }
    let content = format!("用户意图「{}」→ 使用工具 [{}] 完成", intent,
        tools.join(" → "));
    let intent_owned = intent.to_string();
    let _ = std::thread::spawn(move || {
        with_db(|conn| {
            conn.execute(
                "INSERT INTO memories (content, created_at, tag, summary) VALUES (?1, ?2, 'agent_experience', ?3)",
                rusqlite::params![content, chrono::Utc::now().to_rfc3339(), intent_owned],
            )?;
            Ok(())
        })
    }).join();
}

// ─── 主入口 ────────────────────────────────────────────────────

#[tauri::command]
pub async fn agent_chat(app: AppHandle, message: String, history: Option<Vec<Value>>) -> Result<AgentReply, String> {
    let experiences = load_experiences();
    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt(&experiences)}),
    ];
    if let Some(h) = history {
        for m in h.into_iter().take(20) { messages.push(m); }
    }
    messages.push(json!({"role": "user", "content": message.clone()}));

    let mut traces: Vec<ToolTrace> = Vec::new();

    for step in 1..=MAX_STEPS {
        let resp = chat_with_ollama(messages.clone()).await?;
        let msg = resp["message"].clone();
        let content = msg["content"].as_str().unwrap_or("").to_string();
        let native = msg["tool_calls"].as_array().cloned();

        let (tool_name, args, answer) = parse_model_output(&content, native.as_ref())?;

        // 模型直接给出答案 → 结束
        if tool_name.is_none() {
            let answer = answer.unwrap_or_else(|| content.clone());
            // 推送完成事件
            let _ = app.emit("agent://done", json!({"answer": answer, "steps": step}));
            save_experience(&message, &traces);
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

        // 把工具结果回填给模型，继续迭代
        messages.push(json!({"role": "assistant", "content": content}));
        messages.push(json!({
            "role": "tool",
            "content": format!("工具 {} 执行{}: {}", name, if ok {"成功"} else {"失败"}, result_short)
        }));
    }

    let answer = "抱歉，我在限定步数内没能完成任务。可能是工具结果不够明确，请换个说法或拆分指令再试。".to_string();
    let _ = app.emit("agent://done", json!({"answer": answer, "steps": MAX_STEPS}));
    save_experience(&message, &traces);
    Ok(AgentReply { answer, traces, steps: MAX_STEPS, model: agent_model() })
}

#[tauri::command]
pub fn agent_model_name() -> String {
    agent_model()
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
    fn find_nested_json() {
        let text = "前缀 {\"name\":\"x\",\"arguments\":{\"a\":{\"b\":1}}} 后缀";
        let obj = find_first_json_object(text).unwrap();
        let v: Value = serde_json::from_str(&obj).unwrap();
        assert_eq!(v["arguments"]["a"]["b"], 1);
    }
}
