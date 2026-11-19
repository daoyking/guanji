import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface ModelDetails {
  format?: string;
  family?: string;
  parameter_size?: string;
  quantization_level?: string;
}

interface OllamaModel {
  name: string;
  size: number;              // bytes
  digest: string;
  modified_at?: string;
  details?: ModelDetails;
}

const STATUS_IDLE = 'idle' as const;
const STATUS_PULLING = 'pulling' as const;
const STATUS_DONE = 'done' as const;
const STATUS_ERROR = 'error' as const;

type PullStatus = typeof STATUS_IDLE | typeof STATUS_PULLING | typeof STATUS_DONE | typeof STATUS_ERROR;

interface Provider {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
  enabled: boolean;
}

// 常见 OpenAI 兼容服务的预设（base_url 填到 /v1 一级）
const PROVIDER_PRESETS: { name: string; base_url: string; hint: string }[] = [
  { name: 'DeepSeek', base_url: 'https://api.deepseek.com/v1', hint: 'deepseek-chat / deepseek-reasoner' },
  { name: 'OpenAI', base_url: 'https://api.openai.com/v1', hint: 'gpt-4o-mini / gpt-4o' },
  { name: '通义千问', base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', hint: 'qwen-plus / qwen-turbo' },
  { name: 'Kimi', base_url: 'https://api.moonshot.cn/v1', hint: 'moonshot-v1-8k' },
  { name: '智谱 GLM', base_url: 'https://open.bigmodel.cn/api/paas/v4', hint: 'glm-4-flash / glm-4' },
  { name: 'OrcaRouter(本机)', base_url: 'http://localhost:7001/v1', hint: '本机聚合网关' },
];

const emptyProvider = { name: '', base_url: '', api_key: '' };

export default function Models() {
  const [models, setModels] = useState<OllamaModel[]>([]);
  const [loading, setLoading] = useState(false);
  const [ollamaStatus, setOllamaStatus] = useState<'ok' | 'offline' | 'checking'>('checking');
  const [pullInput, setPullInput] = useState('');
  // 云模型服务
  const [providers, setProviders] = useState<Provider[]>([]);
  const [providerForm, setProviderForm] = useState({ ...emptyProvider });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [providerMsg, setProviderMsg] = useState('');
  const [testingId, setTestingId] = useState<string | null>(null);
  const [providerModels, setProviderModels] = useState<Record<string, string[]>>({});
  const [pullStatus, setPullStatus] = useState<{name: string; status: PullStatus; message: string}>({
    name: '',
    status: STATUS_IDLE,
    message: '',
  });

  const loadModels = async () => {
    setLoading(true);
    try {
      const data = await invoke<OllamaModel[]>('list_ollama_models');
      setModels(data);
      setOllamaStatus('ok');
    } catch (e) {
      console.error('Failed to list models:', e);
      setOllamaStatus('offline');
    } finally {
      setLoading(false);
    }
  };

  // ── 云模型服务 ──
  const loadProviders = async () => {
    try {
      setProviders(await invoke<Provider[]>('provider_list'));
    } catch (e) {
      setProviderMsg(`加载服务商失败：${String(e)}`);
    }
  };

  const applyPreset = (preset: typeof PROVIDER_PRESETS[number]) => {
    setProviderForm({ name: preset.name, base_url: preset.base_url, api_key: '' });
    setProviderMsg(`已填入 ${preset.name} 配置，常见模型：${preset.hint}`);
    setEditingId(null);
  };

  const saveProvider = async () => {
    const { name, base_url, api_key } = providerForm;
    if (!name.trim() || !base_url.trim()) {
      setProviderMsg('✗ 名称与 Base URL 不能为空');
      return;
    }
    try {
      await invoke('provider_save', { id: editingId, name, baseUrl: base_url, apiKey: api_key, enabled: true });
      setProviderForm({ ...emptyProvider });
      setEditingId(null);
      setProviderMsg(`✓ 已保存服务商「${name}」`);
      await loadProviders();
    } catch (e) {
      setProviderMsg(`✗ 保存失败：${String(e)}`);
    }
  };

  const editProvider = (p: Provider) => {
    setProviderForm({ name: p.name, base_url: p.base_url, api_key: p.api_key });
    setEditingId(p.id);
    setProviderMsg('');
  };

  const removeProvider = async (p: Provider) => {
    if (!confirm(`确定删除云模型服务「${p.name}」吗？\n\n注意：若当前对话模型来自该服务，需重新选择模型。`)) return;
    try {
      const msg = await invoke<string>('provider_delete', { id: p.id });
      setProviderMsg(`✓ ${msg}`);
      setProviderModels((prev) => { const n = { ...prev }; delete n[p.id]; return n; });
      await loadProviders();
    } catch (e) {
      setProviderMsg(`✗ 删除失败：${String(e)}`);
    }
  };

  const testProvider = async (p: Provider) => {
    setTestingId(p.id);
    setProviderMsg(`正在测试「${p.name}」并拉取模型列表…`);
    try {
      const ids = await invoke<string[]>('provider_list_models', { id: p.id });
      setProviderModels((prev) => ({ ...prev, [p.id]: ids }));
      setProviderMsg(`✓ 「${p.name}」连接成功，发现 ${ids.length} 个模型`);
    } catch (e) {
      setProviderMsg(`✗ 「${p.name}」连接失败：${String(e)}`);
    } finally {
      setTestingId(null);
    }
  };

  useEffect(() => {
    loadModels();
    loadProviders();
  }, []);

  const handlePull = async () => {
    const name = pullInput.trim();
    if (!name) return;
    setPullStatus({ name, status: STATUS_PULLING, message: `正在拉取 ${name}…` });
    try {
      const result = await invoke<string>('pull_ollama_model', { name });
      setPullStatus({ name, status: STATUS_DONE, message: result });
      setPullInput('');
      await loadModels();
    } catch (e) {
      setPullStatus({ name, status: STATUS_ERROR, message: String(e) });
    }
  };

  const handleDelete = async (model: OllamaModel) => {
    if (!confirm(`确定要删除模型 "${model.name}" 吗？（约 ${(model.size / 1024 / 1024 / 1024).toFixed(1)} GB）`)) return;
    try {
      const result = await invoke<string>('delete_ollama_model', { name: model.name });
      alert(result);
      await loadModels();
    } catch (e) {
      alert(String(e));
    }
  };

  const formatSize = (bytes: number): string => {
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
    return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  };

  const getFamilyColor = (family?: string): string => {
    if (!family) return 'bg-gray-100 text-gray-600 dark:bg-gray-700 dark:text-gray-400';
    const map: Record<string, string> = {
      qwen2: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
      phi: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
      llama: 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400',
      bert: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400',
    };
    return map[family.toLowerCase()] || 'bg-gray-100 text-gray-600 dark:bg-gray-700 dark:text-gray-400';
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">模型管家</h2>
        <button onClick={loadModels} disabled={loading} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 flex items-center gap-2">
          <span>{loading ? '刷新中...' : '🔄 刷新'}</span>
        </button>
      </div>

      {/* Ollama Status Banner */}
      <div className={`rounded-xl p-4 card-shadow flex items-center gap-3 ${
        ollamaStatus === 'ok' ? 'bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800' :
        ollamaStatus === 'offline' ? 'bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800' :
        'bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700'
      }`}>
        <span className="text-2xl">{ollamaStatus === 'ok' ? '✅' : ollamaStatus === 'offline' ? '❌' : '⏳'}</span>
        <div className="flex-1">
          <p className={`font-medium ${
            ollamaStatus === 'ok' ? 'text-green-800 dark:text-green-300' :
            ollamaStatus === 'offline' ? 'text-red-800 dark:text-red-300' :
            'text-gray-600 dark:text-gray-400'
          }`}>
            {ollamaStatus === 'ok' ? 'Ollama 服务在线' :
             ollamaStatus === 'offline' ? 'Ollama 未连接，请确认服务已启动 (ollama serve)' :
             '检测中...'}
          </p>
          <p className="text-sm text-gray-500 dark:text-gray-400 mt-0.5">
            {ollamaStatus === 'ok' ? `${models.length} 个本地模型` : '连接地址: localhost:11434'}
          </p>
        </div>
      </div>

      {/* Cloud Model Providers */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-lg font-medium text-gray-900 dark:text-white">☁️ 云模型服务</h3>
          <span className="text-xs text-gray-400">OpenAI 兼容协议</span>
        </div>
        <p className="text-xs text-gray-500 dark:text-gray-400 mb-4">
          添加云服务商后，可在「AI 管家」的模型选择器里切换到云端模型对话。API Key 仅保存在本机数据库。
        </p>

        {/* 预设快捷 */}
        <div className="mb-4">
          <p className="text-xs text-gray-500 dark:text-gray-400 mb-2">快速填入：</p>
          <div className="flex flex-wrap gap-2">
            {PROVIDER_PRESETS.map((preset) => (
              <button key={preset.name} onClick={() => applyPreset(preset)}
                className="px-3 py-1 text-xs bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 rounded-full hover:bg-purple-100 dark:hover:bg-purple-900/30 hover:text-purple-700 dark:hover:text-purple-400 transition-colors">
                + {preset.name}
              </button>
            ))}
          </div>
        </div>

        {/* 配置表单 */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3 mb-3">
          <input value={providerForm.name} onChange={e => setProviderForm({ ...providerForm, name: e.target.value })}
            placeholder="服务商名称，如 DeepSeek"
            className="px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-sm text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-purple-500" />
          <input value={providerForm.base_url} onChange={e => setProviderForm({ ...providerForm, base_url: e.target.value })}
            placeholder="Base URL，如 https://api.deepseek.com/v1"
            className="px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-sm text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-purple-500" />
          <input value={providerForm.api_key} onChange={e => setProviderForm({ ...providerForm, api_key: e.target.value })}
            type="password" placeholder="API Key（可选，本机服务可留空）"
            className="px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-sm text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-purple-500" />
        </div>
        <div className="flex gap-2 mb-3">
          <button onClick={saveProvider} className="px-5 py-2 bg-purple-600 text-white rounded-lg hover:bg-purple-700 text-sm font-medium">
            {editingId ? '保存修改' : '添加服务'}
          </button>
          {editingId && <button onClick={() => { setEditingId(null); setProviderForm({ ...emptyProvider }); }} className="px-4 py-2 bg-gray-200 dark:bg-gray-600 text-gray-700 dark:text-gray-200 rounded-lg text-sm">取消编辑</button>}
        </div>

        {/* 已配置列表 */}
        {providers.length > 0 && (
          <div className="mt-4 space-y-2">
            <p className="text-xs text-gray-500 dark:text-gray-400">已配置 {providers.length} 个服务：</p>
            {providers.map(p => (
              <div key={p.id} className={`rounded-lg border px-3 py-2.5 ${p.enabled ? 'border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700/40' : 'border-gray-200 dark:border-gray-700 opacity-60'}`}>
                <div className="flex items-center justify-between gap-3 flex-wrap">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-gray-900 dark:text-white">{p.name}</span>
                      <span className={`text-[10px] px-1.5 py-0.5 rounded-full ${p.enabled ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400' : 'bg-gray-200 text-gray-600'}`}>
                        {p.enabled ? '已启用' : '已停用'}
                      </span>
                    </div>
                    <p className="text-[11px] text-gray-500 dark:text-gray-400 truncate mt-0.5" title={p.base_url}>{p.base_url}</p>
                    <p className="text-[11px] text-gray-400 mt-0.5">Key: {p.api_key ? '••••' + p.api_key.slice(-4) : '未设置'}</p>
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    <button onClick={() => testProvider(p)} disabled={testingId !== null}
                      className="text-xs px-2.5 py-1 rounded border border-purple-300 dark:border-purple-700 text-purple-600 dark:text-purple-400 hover:bg-purple-50 dark:hover:bg-purple-900/20 disabled:opacity-50">
                      {testingId === p.id ? '测试中…' : '测试并列出模型'}
                    </button>
                    <button onClick={() => editProvider(p)} className="text-xs px-2 py-1 text-gray-500 hover:text-blue-600">编辑</button>
                    <button onClick={() => removeProvider(p)} className="text-xs px-2 py-1 text-gray-500 hover:text-red-600">删除</button>
                  </div>
                </div>
                {providerModels[p.id] && (
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {providerModels[p.id].length === 0 ? (
                      <span className="text-[11px] text-gray-400">未返回模型</span>
                    ) : providerModels[p.id].map(id => (
                      <span key={id} className="text-[10px] px-1.5 py-0.5 bg-purple-50 dark:bg-purple-900/20 text-purple-700 dark:text-purple-400 rounded font-mono">{id}</span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}

        {providerMsg && (
          <div className={`mt-3 p-2.5 rounded-lg text-xs ${providerMsg.startsWith('✓') ? 'bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-400' : providerMsg.startsWith('✗') ? 'bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400' : 'bg-blue-50 dark:bg-blue-900/20 text-blue-700 dark:text-blue-400'} break-all`}>
            {providerMsg}
          </div>
        )}
      </section>

      {/* Pull New Model */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">拉取新模型</h3>
        <div className="flex gap-3">
          <input
            value={pullInput}
            onChange={e => setPullInput(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handlePull()}
            placeholder="输入模型名称，如：qwen2.5:7b、phi4、llama3.2"
            className="flex-1 px-4 py-2.5 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <button
            onClick={handlePull}
            disabled={!pullInput.trim() || pullStatus.status === STATUS_PULLING}
            className="px-6 py-2.5 bg-gradient-to-r from-blue-600 to-blue-700 text-white rounded-lg hover:from-blue-700 hover:to-blue-800 disabled:opacity-50 disabled:cursor-not-allowed font-medium transition-all"
          >
            {pullStatus.status === STATUS_PULLING ? '拉取中...' : '拉取'}
          </button>
        </div>

        {/* Common Models Quick Add */}
        <div className="mt-3 flex flex-wrap gap-2">
          {['qwen2.5:7b', 'qwen2.5:3b', 'phi4-mini-reasoning', 'llama3.2', 'deepseek-coder:6.7b'].map(tag => (
            <button
              key={tag}
              onClick={() => { setPullInput(tag); }}
              className="px-3 py-1 text-xs bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-400 rounded-full hover:bg-blue-100 dark:hover:bg-blue-900/30 hover:text-blue-600 dark:hover:text-blue-400 transition-colors"
            >
              + {tag}
            </button>
          ))}
        </div>

        {/* Pull Progress */}
        {pullStatus.status !== STATUS_IDLE && (
          <div className={`mt-4 p-3 rounded-lg text-sm ${
            pullStatus.status === STATUS_DONE ? 'bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-400' :
            pullStatus.status === STATUS_ERROR ? 'bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400' :
            'bg-blue-50 dark:bg-blue-900/20 text-blue-700 dark:text-blue-400'
          }`}>
            <div className="flex items-center gap-2">
              <span>{pullStatus.status === STATUS_PULLING ? '⬇️' : pullStatus.status === STATUS_DONE ? '✅' : '❌'}</span>
              <span className="font-medium">{pullStatus.message}</span>
            </div>
          </div>
        )}
      </section>

      {/* Installed Models List */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-medium text-gray-900 dark:text-white">已安装模型</h3>
          <span className="text-sm text-gray-500 dark:text-gray-400">{models.length} 个</span>
        </div>

        {models.length === 0 ? (
          <div className="text-center py-12 text-gray-400 dark:text-gray-500">
            <p className="text-4xl mb-3">🤖</p>
            <p className="text-lg font-medium">暂无已安装的模型</p>
            <p className="text-sm mt-1">使用上方输入框拉取第一个模型</p>
          </div>
        ) : (
          <div className="space-y-3">
            {models.map(model => (
              <div key={model.digest} className="flex items-center justify-between p-4 bg-gray-50 dark:bg-gray-700/50 rounded-xl hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors group">
                <div className="flex items-center gap-4 min-w-0">
                  <div className="w-10 h-10 rounded-full bg-gradient-to-br from-blue-500 to-purple-600 flex items-center justify-center text-white font-bold text-sm shrink-0">
                    {(model.details?.family ?? 'M').slice(0, 2).toUpperCase()}
                  </div>
                  <div className="min-w-0">
                    <p className="font-medium text-gray-900 dark:text-white truncate">{model.name}</p>
                    <div className="flex items-center gap-2 mt-1 flex-wrap">
                      {model.details?.parameter_size && (
                        <span className="text-xs text-gray-500 dark:text-gray-400">{model.details.parameter_size}</span>
                      )}
                      {model.details?.quantization_level && (
                        <span className="text-xs px-1.5 py-0.5 bg-gray-200 dark:bg-gray-600 rounded text-gray-600 dark:text-gray-300">{model.details.quantization_level}</span>
                      )}
                      {model.details?.format && (
                        <span className={`text-xs px-1.5 py-0.5 rounded ${getFamilyColor(model.details?.family)}`}>
                          {model.details.format.toUpperCase()}
                        </span>
                      )}
                      {model.modified_at && (
                        <span className="text-xs text-gray-400">更新于 {new Date(model.modified_at).toLocaleDateString('zh-CN')}</span>
                      )}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-4 shrink-0">
                  <span className="text-sm font-mono text-gray-500 dark:text-gray-400">
                    {formatSize(model.size)}
                  </span>
                  <button
                    onClick={() => handleDelete(model)}
                    className="opacity-0 group-hover:opacity-100 px-3 py-1.5 text-sm text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-all"
                  >
                    删除
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Tips */}
      <div className="bg-blue-50 dark:bg-blue-900/20 rounded-xl p-4 text-sm text-blue-700 dark:text-blue-400">
        <p className="font-medium mb-1">💡 提示</p>
        <ul className="list-disc list-inside space-y-0.5 text-blue-600 dark:text-blue-500">
          <li>M1/M2 Mac 建议 ≤7B 参数模型；M1 Pro 32GB 可流畅运行 7B~13B 量化版</li>
          <li>常用推荐：<code className="font-mono bg-blue-100 dark:bg-blue-800/40 px-1 rounded">qwen2.5:7b</code> 平衡性能与速度，<code className="font-mono bg-blue-100 dark:bg-blue-800/40 px-1 rounded">qwen2.5:3b</code> 快速响应</li>
          <li>拉取需要网络连接，大型模型可能需要较长时间</li>
        </ul>
      </div>
    </div>
  );
}
