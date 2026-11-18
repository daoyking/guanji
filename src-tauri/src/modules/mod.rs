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
