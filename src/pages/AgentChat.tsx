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
interface LocalModel {
  name: string;
  ref: string;       // provider::model，用于探活/切换
  source: string;    // 来源显示名：本机 Ollama / 云服务商名
  kind: 'local' | 'cloud' | 'error';
  size: number;
  current: boolean;
}
interface ModelProbe {
  model: string;
  ok: boolean;
  latency_ms: number;
  detail: string;
}
interface StoredMessage {
  id: string;
  role: string;
  content: string;
  traces?: Trace[];
  steps?: number;
  model?: string;
  time: string;
}

const SUGGESTIONS = [
  '检查所有本地服务的健康状态',
  '扫描下载目录里的大文件',
  '记住：我偏好深色主题',
  '5分钟后提醒我休息一下',
  '用命令看看磁盘剩余空间',
  '打开 Safari 浏览器',
];

const TOOL_LABELS: Record<string, string> = {
  get_system_info: '系统信息', list_processes: '进程列表', list_large_files: '大文件扫描',
  scan_downloads: '下载目录扫描', find_duplicates: '重复文件', clean_temp_files: '清理临时文件',
  list_services: '服务列表', probe_service: '服务探测', run_all_probes: '全部服务探测',
  clip_list: '剪贴板', list_tasks: '定时任务', add_task: '创建任务',
  mem_list: '记忆列表', mem_add: '记忆存储', mem_search: '记忆搜索', notify: '桌面通知',
  run_command: '执行命令', open_target: '打开应用/文件',
  read_file: '读取文件', write_file: '写入文件',
  set_reminder: '设置提醒', list_reminders: '查看提醒', delete_reminder: '删除提醒',
};

function now() {
  return new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
}

const WELCOME_CONTENT = '你好，我是管家。直接用中文下达指令——查系统、跑命令、读写文件、打开应用、设定时提醒、记事情，我来判断该调用哪些工具并一步步做完，还会从成败经验里越来越懂你。';
const welcomeMessage = (): Message => ({ id: 'welcome', role: 'assistant', content: WELCOME_CONTENT, time: now() });

export default function AgentChat() {
  const [messages, setMessages] = useState<Message[]>([welcomeMessage()]);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [model, setModel] = useState('加载中…');
  const [modelOpen, setModelOpen] = useState(false);
  const [models, setModels] = useState<LocalModel[]>([]);
  const [probing, setProbing] = useState<string | null>(null);
  const [probeCache, setProbeCache] = useState<Record<string, ModelProbe>>({});
  const [modelMsg, setModelMsg] = useState('');
  const bottomRef = useRef<HTMLDivElement>(null);

  // 启动时恢复对话历史
  useEffect(() => {
    invoke<StoredMessage[]>('chat_history')
      .then((history) => {
        const restored: Message[] = history.map((h) => ({
          id: h.id,
          role: h.role as MsgRole,
          content: h.content,
          traces: h.traces || [],
          steps: h.steps,
          model: h.model,
          time: h.time,
        }));
        if (restored.length > 0) setMessages([welcomeMessage(), ...restored]);
      })
      .catch((e) => console.error('加载对话历史失败:', e));
    invoke<string>('agent_model_name').then(setModel).catch(() => setModel('未知'));
  }, []);

  // 持久化单条消息（失败只记录日志，不打断对话）
  const persist = (m: Message) => {
    invoke('chat_upsert', {
      msg: {
        id: m.id, role: m.role, content: m.content,
        traces: m.traces || [], steps: m.steps ?? null,
        model: m.model ?? null, time: m.time,
      },
    }).catch((e) => console.error('保存对话失败:', e));
  };

  // 清空对话历史
  const clearHistory = async () => {
    if (!window.confirm('确定清空所有对话历史吗？此操作不可恢复。')) return;
    try {
      await invoke('chat_clear');
      setMessages([welcomeMessage()]);
    } catch (e) {
      alert(`清空失败：${String(e)}`);
    }
  };

  // 展开下拉时加载本机模型列表
  const toggleModelPanel = async () => {
    const next = !modelOpen;
    setModelOpen(next);
    setModelMsg('');
    if (next && models.length === 0) {
      try {
        const list = await invoke<LocalModel[]>('agent_list_models');
        setModels(list);
        if (list.length === 0) setModelMsg('未检测到模型，请启动 Ollama 或在「模型管家」添加云模型服务');
      } catch (e) {
        setModelMsg(`加载模型列表失败：${String(e)}`);
      }
    }
  };

  // 探活单个模型（真发请求验证能否使用）。key 用 ref 区分同名模型
  const probe = async (ref: string): Promise<ModelProbe> => {
    setProbing(ref);
    try {
      const result = await invoke<ModelProbe>('agent_probe_model', { model: ref });
      setProbeCache((prev) => ({ ...prev, [ref]: result }));
      return result;
    } catch (e) {
      const fail: ModelProbe = { model: ref, ok: false, latency_ms: 0, detail: String(e) };
      setProbeCache((prev) => ({ ...prev, [ref]: fail }));
      return fail;
    } finally {
      setProbing(null);
    }
  };

  // 选择模型：先探活，通过才切换并持久化。后端返回 model 名供显示
  const chooseModel = async (m: LocalModel) => {
    if (m.current || m.kind === 'error') return;
    setModelMsg('正在验证模型可用性…');
    const result = await probe(m.ref);
    if (!result.ok) {
      setModelMsg(`✗ ${m.name} 不可用：${result.detail}`);
      return;
    }
    try {
      const saved = await invoke<ModelProbe>('agent_set_model', { model: m.ref });
      setModel(saved.model.includes('::') ? saved.model.split('::').slice(1).join('::') : saved.model);
      setModels((prev) => prev.map((x) => ({ ...x, current: x.ref === m.ref })));
      setModelMsg(`✓ 已切换到 ${m.name}（${result.latency_ms}ms）`);
    } catch (e) {
      setModelMsg(`切换失败：${String(e)}`);
    }
  };

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
        const updated = { ...last, traces };
        next[next.length - 1] = updated;
        persist(updated); // 工具轨迹实时落盘
        return next;
      });
    });
    return () => { unlistenStep.then((f) => f()); };
  }, []);

  // 流式增量文本：实时写入当前助手消息
  useEffect(() => {
    const unlisten = listen<{ text: string }>('agent://delta', (event) => {
      const text = event.payload.text;
      // 流式期间把文本写进最后一条 assistant 消息，实现逐字显示
      setMessages((prev) => {
        const last = prev[prev.length - 1];
        if (!last || last.role !== 'assistant') return prev;
        const next = [...prev];
        next[next.length - 1] = { ...last, content: text };
        return next;
      });
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  // 破坏性操作二次确认
  useEffect(() => {
    const unlisten = listen<{ description: string }>('agent://confirm', async (event) => {
      const approved = window.confirm(`管家请求执行以下操作：\n\n${event.payload.description}\n\n是否允许？`);
      try { await invoke('agent_confirm', { approved }); }
      catch (e) { console.error('确认回传失败:', e); }
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  // 用户停止执行
  useEffect(() => {
    const unlisten = listen('agent://cancelled', () => {
      // 取消事件已由后端返回「已停止执行」答案覆盖当前消息
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  // 停止执行
  const stop = async () => {
    try { await invoke('agent_cancel'); }
    catch (e) { console.error('停止失败:', e); }
  };

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
    const userMsg: Message = { id: `u-${Date.now()}`, role: 'user', content, time: now() };
    const assistantMsg: Message = { id: assistantId, role: 'assistant', content: '', traces: [], time: now() };
    setMessages((prev) => [...prev, userMsg, assistantMsg]);
    persist(userMsg);
    persist(assistantMsg);

    try {
      const reply = await invoke<{ answer: string; traces: Trace[]; steps: number; model: string }>('agent_chat', {
        message: content,
        history,
      });
      const finalMsg: Message = {
        ...assistantMsg,
        content: reply.answer, traces: reply.traces, steps: reply.steps, model: reply.model,
      };
      setMessages((prev) => prev.map((m) => m.id === assistantId ? finalMsg : m));
      persist(finalMsg);
    } catch (e) {
      const failMsg: Message = { ...assistantMsg, content: `⚠️ 执行失败：${String(e)}`, traces: [] };
      setMessages((prev) => prev.map((m) => m.id === assistantId ? failMsg : m));
      persist(failMsg);
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
        <div className="flex items-center gap-2">
        {messages.length > 1 && (
          <button onClick={clearHistory} disabled={busy}
            className="px-3 py-1.5 text-xs text-gray-500 dark:text-gray-400 border border-gray-200 dark:border-gray-700 rounded-full hover:border-red-400 hover:text-red-500 disabled:opacity-50 transition-colors"
            title="清空所有对话历史">
            清空对话
          </button>
        )}
        <div className="relative">
          <button onClick={toggleModelPanel} disabled={busy}
            className="flex items-center gap-2 px-3 py-1.5 bg-green-50 dark:bg-green-900/20 hover:bg-green-100 dark:hover:bg-green-900/40 rounded-full transition-colors disabled:opacity-50"
            title="点击选择模型">
            <span className="w-2 h-2 bg-green-500 rounded-full animate-pulse" />
            <span className="text-xs text-green-700 dark:text-green-400 font-mono">{model}</span>
            <span className="text-xs text-green-600 dark:text-green-400">▾</span>
          </button>

          {modelOpen && (
            <div className="absolute right-0 top-full mt-2 w-80 z-20 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-xl shadow-xl p-3">
              <div className="flex items-center justify-between mb-2">
                <span className="text-sm font-medium text-gray-800 dark:text-gray-200">选择对话模型</span>
                <button onClick={() => setModelOpen(false)} className="text-gray-400 hover:text-gray-600 dark:hover:text-gray-200 text-lg leading-none">×</button>
              </div>
              <p className="text-[11px] text-gray-400 mb-2">本机 Ollama + 已配置的云模型，选中会自动验证可用性</p>

              <div className="space-y-1.5 max-h-64 overflow-y-auto">
                {models.map((m) => {
                  const pr = probeCache[m.ref];
                  const isErr = m.kind === 'error';
                  return (
                    <div key={m.ref} className={`rounded-lg border px-2.5 py-2 ${
                      m.current ? 'border-green-400 bg-green-50 dark:bg-green-900/20' : 'border-gray-200 dark:border-gray-700'
                    }`}>
                      <div className="flex items-center justify-between gap-2">
                        <button onClick={() => chooseModel(m)} disabled={probing !== null || m.current || isErr}
                          className="flex-1 text-left min-w-0 disabled:cursor-default">
                          <div className="flex items-center gap-1.5">
                            <span className={`text-[10px] px-1.5 py-0.5 rounded-full shrink-0 ${m.kind === 'cloud' ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400' : m.kind === 'local' ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400' : 'bg-red-100 text-red-700'}`}>
                              {m.kind === 'cloud' ? '☁ 云' : m.kind === 'local' ? '本机' : '错误'}
                            </span>
                            <span className="text-xs font-mono text-gray-800 dark:text-gray-200 truncate">{m.name}</span>
                            {m.current && <span className="text-[10px] px-1.5 py-0.5 bg-green-500 text-white rounded-full shrink-0">当前</span>}
                          </div>
                          <div className="text-[10px] text-gray-400 mt-0.5">
                            {m.source}{m.size > 0 ? ` · ${(m.size / 1e9).toFixed(1)} GB` : ''}
                          </div>
                        </button>
                        {!isErr && (
                          <button onClick={() => probe(m.ref)} disabled={probing !== null}
                            className="shrink-0 text-[11px] px-2 py-1 rounded border border-gray-300 dark:border-gray-600 text-gray-600 dark:text-gray-300 hover:border-blue-400 hover:text-blue-600 disabled:opacity-50">
                            {probing === m.ref ? '检测中…' : '检测'}
                          </button>
                        )}
                      </div>
                      {pr && (
                        <div className={`mt-1.5 text-[10px] ${pr.ok ? 'text-green-600 dark:text-green-400' : 'text-red-500'} break-all`}>
                          {pr.ok ? `✓ 可用 · ${pr.latency_ms}ms` : `✗ ${pr.detail}`}
                        </div>
                      )}
                    </div>
                  );
                })}
                {models.length === 0 && <p className="text-xs text-gray-400 text-center py-4">未检测到任何模型</p>}
              </div>
              {modelMsg && (
                <div className={`mt-2 text-[11px] ${modelMsg.startsWith('✓') ? 'text-green-600 dark:text-green-400' : modelMsg.startsWith('✗') ? 'text-red-500' : 'text-gray-500'} break-all`}>
                  {modelMsg}
                </div>
              )}
            </div>
          )}
        </div>
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
          {busy ? (
            <button onClick={stop}
              className="px-6 py-3 rounded-xl bg-red-500 hover:bg-red-600 text-white font-medium transition-colors">
              停止
            </button>
          ) : (
            <button onClick={() => send()} disabled={!input.trim()}
              className="px-6 py-3 rounded-xl bg-blue-600 hover:bg-blue-700 text-white font-medium disabled:opacity-40 transition-colors">
              发送
            </button>
          )}
        </div>
        <p className="mt-2 text-[11px] text-gray-400 text-center">
          管家会自主判断并调用工具完成任务 · 所有执行过程都可见 · 经验会自动积累
        </p>
      </div>
    </div>
  );
}
