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

export default function Models() {
  const [models, setModels] = useState<OllamaModel[]>([]);
  const [loading, setLoading] = useState(false);
  const [ollamaStatus, setOllamaStatus] = useState<'ok' | 'offline' | 'checking'>('checking');
  const [pullInput, setPullInput] = useState('');
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

  useEffect(() => {
    loadModels();
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
