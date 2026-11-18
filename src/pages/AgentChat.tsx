import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

type MsgRole = 'user' | 'assistant' | 'system';
interface Trace {
  step: number;
  tool: string;
  args: Record<string, unknown>;
  result?: string;
  status: 'running' | 'done' | 'error';
  elapsed_ms?: number;
}
interface Message {
  id: string;
  role: MsgRole;
  content: string;
  traces?: Trace[];
  steps?: number;
  model?: string;
  time: string;
}

const SUGGESTIONS = [
  '检查所有本地服务的健康状态',
  '扫描下载目录里的大文件',
  '现在有哪些进程占用内存最高？',
  '看看我的剪贴板历史',
  '记住：我偏好深色主题',
  '列出所有定时任务',
];

const TOOL_LABELS: Record<string, string> = {
  get_system_info: '系统信息', list_processes: '进程列表', list_large_files: '大文件扫描',
  scan_downloads: '下载目录扫描', find_duplicates: '重复文件', clean_temp_files: '清理临时文件',
  list_services: '服务列表', probe_service: '服务探测', run_all_probes: '全部服务探测',
  clip_list: '剪贴板', list_tasks: '定时任务', add_task: '创建任务',
  mem_list: '记忆列表', mem_add: '记忆存储', mem_search: '记忆搜索', notify: '桌面通知',
};

function now() {
  return new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
}

export default function AgentChat() {
  const [messages, setMessages] = useState<Message[]>([{
    id: 'welcome',
    role: 'assistant',
    content: '你好，我是管家。直接用中文告诉我你想做什么——查进程、清文件、看服务、定任务、记事情，我来判断该调用哪些工具并一步步做完。',
    time: now(),
  }]);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [model, setModel] = useState('加载中…');
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<string>('agent_model_name').then(setModel).catch(() => setModel('未知'));
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  // 监听 Agent 每一步工具调用，实时更新到当前助手消息
  useEffect(() => {
    const unlistenStep = listen<Trace & { status: string }>('agent://step', (event) => {
      const p = event.payload;
      setMessages((prev) => {
        const next = [...prev];
        const last = next[next.length - 1];
        if (!last || last.role !== 'assistant') return prev;
        const traces = [...(last.traces || [])];
        const idx = traces.findIndex((t) => t.step === p.step);
        const trace: Trace = {
          step: p.step, tool: p.tool, args: p.args || {},
          status: p.status as Trace['status'], result: p.result, elapsed_ms: p.elapsed_ms,
        };
        if (idx >= 0) traces[idx] = trace; else traces.push(trace);
        next[next.length - 1] = { ...last, traces };
        return next;
      });
    });
    return () => { unlistenStep.then((f) => f()); };
  }, []);

  const send = async (text?: string) => {
    const content = (text ?? input).trim();
    if (!content || busy) return;
    setInput('');
    setBusy(true);

    // 构造历史（最近 8 轮）
    const history = messages
      .filter((m) => m.id !== 'welcome')
      .slice(-16)
      .map((m) => ({ role: m.role, content: m.content }));

    const assistantId = `a-${Date.now()}`;
    setMessages((prev) => [
      ...prev,
      { id: `u-${Date.now()}`, role: 'user', content, time: now() },
      { id: assistantId, role: 'assistant', content: '', traces: [], time: now() },
    ]);

    try {
      const reply = await invoke<{ answer: string; traces: Trace[]; steps: number; model: string }>('agent_chat', {
        message: content,
        history,
      });
      setMessages((prev) => prev.map((m) =>
        m.id === assistantId
          ? { ...m, content: reply.answer, traces: reply.traces, steps: reply.steps, model: reply.model }
          : m
      ));
    } catch (e) {
      setMessages((prev) => prev.map((m) =>
        m.id === assistantId
          ? { ...m, content: `⚠️ 执行失败：${String(e)}`, traces: [] }
          : m
      ));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col h-full max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-4 flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <span className="text-3xl">✨</span> AI 管家
          </h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
            指令驱动 · 自主决策 · 越用越懂你
          </p>
        </div>
        <div className="flex items-center gap-2 px-3 py-1.5 bg-green-50 dark:bg-green-900/20 rounded-full">
          <span className="w-2 h-2 bg-green-500 rounded-full animate-pulse" />
          <span className="text-xs text-green-700 dark:text-green-400 font-mono">{model}</span>
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto space-y-4 pr-2">
        {messages.map((m) => (
          <div key={m.id} className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div className={`max-w-[85%] rounded-2xl px-4 py-3 ${
              m.role === 'user'
                ? 'bg-blue-600 text-white rounded-br-sm'
                : 'bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-bl-sm'
            }`}>
              {/* Tool traces */}
              {m.traces && m.traces.length > 0 && (
                <div className="mb-3 space-y-1.5">
                  {m.traces.map((t) => (
                    <div key={t.step} className="flex items-start gap-2 text-xs bg-gray-50 dark:bg-gray-900/50 rounded-lg px-2.5 py-2">
                      <span className="shrink-0 w-5 h-5 rounded-full bg-blue-100 dark:bg-blue-900/40 text-blue-600 dark:text-blue-400 flex items-center justify-center font-mono">
                        {t.step}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="font-medium text-gray-700 dark:text-gray-300">
                            {TOOL_LABELS[t.tool] || t.tool}
                          </span>
                          {t.status === 'running' && <span className="text-blue-500 animate-pulse">执行中…</span>}
                          {t.status === 'done' && <span className="text-green-600 dark:text-green-400">✓ {t.elapsed_ms}ms</span>}
                          {t.status === 'error' && <span className="text-red-500">✗ 失败</span>}
                        </div>
                        {t.result && t.status !== 'running' && (
                          <div className="mt-1 text-gray-500 dark:text-gray-400 truncate font-mono text-[11px]">
                            {t.result.slice(0, 160)}
                          </div>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              )}
              {/* Content */}
              {m.content ? (
                <p className="text-sm leading-relaxed whitespace-pre-wrap">{m.content}</p>
              ) : busy ? (
                <div className="flex items-center gap-2 text-sm text-gray-400">
                  <span className="flex gap-1">
                    <span className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce" />
                    <span className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce [animation-delay:150ms]" />
                    <span className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce [animation-delay:300ms]" />
                  </span>
                  正在思考…
                </div>
              ) : null}
              <div className="mt-1.5 flex items-center gap-2 text-[10px] opacity-50">
                <span>{m.time}</span>
                {m.steps && <span>· {m.steps} 步完成</span>}
                {m.model && <span>· {m.model}</span>}
              </div>
            </div>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      {/* Suggestions */}
      {messages.length <= 1 && (
        <div className="py-3 flex flex-wrap gap-2">
          {SUGGESTIONS.map((s) => (
            <button key={s} onClick={() => send(s)} disabled={busy}
              className="text-xs px-3 py-1.5 rounded-full border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 text-gray-600 dark:text-gray-300 hover:border-blue-400 hover:text-blue-600 dark:hover:text-blue-400 transition-colors disabled:opacity-50">
              {s}
            </button>
          ))}
        </div>
      )}

      {/* Input */}
      <div className="pt-3 border-t border-gray-200 dark:border-gray-700">
        <div className="flex gap-2">
          <input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } }}
            placeholder="告诉我你想做什么，例如：检查服务状态并清理临时文件…"
            disabled={busy}
            className="flex-1 px-4 py-3 rounded-xl border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
          />
          <button onClick={() => send()} disabled={busy || !input.trim()}
            className="px-6 py-3 rounded-xl bg-blue-600 hover:bg-blue-700 text-white font-medium disabled:opacity-40 transition-colors">
            {busy ? '执行中' : '发送'}
          </button>
        </div>
        <p className="mt-2 text-[11px] text-gray-400 text-center">
          管家会自主判断并调用工具完成任务 · 所有执行过程都可见 · 经验会自动积累
        </p>
      </div>
    </div>
  );
}
