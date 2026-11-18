# 管家 (GuanJia) v0.6.0-alpha

一句话指令驱动的本机全能 AI Agent。
- Tech: Tauri 2 + React 19 + Rust + SQLite + Ollama, 端口 1420
- 核心：ReAct 编排循环（意图→选工具→执行→观察→迭代），23 类工具，中文对话界面
- 管家能力：系统信息、进程、文件扫描/读写、shell 命令（带安全护栏）、打开应用、定时提醒、记忆、桌面通知
- 自进化：历史经验按意图相关度检索，区分成功经验与失败复盘，越用越懂你
- 兼容 qwen2.5-coder:7b 等本地小模型的三种工具调用输出格式
- 模型：界面自选 Ollama 模型，切换前真发请求探活（拒绝不可用/embedding 模型），偏好持久化跨重启
- 状态：v0.6.0 alpha，指令驱动 Agent + 模型自选 + 文件管家自定义忽略目录与多选删除 + 应用管家 + 对话历史持久化已可用

## Git Commit
`git rev-parse --short HEAD`
