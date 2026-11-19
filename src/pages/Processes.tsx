import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface ProcessEntry {
  pid: number;
  name: string;
  memory_mb: number;
  cpu_percent: number;
  status: string;
  kill_risk: 'dangerous' | 'caution' | 'safe';
  kill_recommend: string;
  kill_reason: string;
  kill_impact: string;
}

// 风险等级 → 徽章样式（红=危险，黄=谨慎，绿=安全）
const RISK_STYLE: Record<string, { badge: string; dot: string; label: string }> = {
  dangerous: {
    badge: 'bg-red-100 dark:bg-red-900/30 text-red-700 dark:text-red-400',
    dot: 'bg-red-500',
    label: '不建议终止',
  },
  caution: {
    badge: 'bg-amber-100 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400',
    dot: 'bg-amber-500',
    label: '谨慎终止',
  },
  safe: {
    badge: 'bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400',
    dot: 'bg-green-500',
    label: '可以终止',
  },
};

export default function Processes() {
  const [processes, setProcesses] = useState<ProcessEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [sortBy, setSortBy] = useState<'memory' | 'cpu'>('memory');
  const [error, setError] = useState('');

  const loadProcesses = async () => {
    setLoading(true);
    setError('');
    try {
      const procs = await invoke<ProcessEntry[]>('get_processes', { limit: 50 });
      setProcesses(procs);
    } catch (e) {
      setError(`加载进程失败: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  const handleKill = async (p: ProcessEntry) => {
    const style = RISK_STYLE[p.kill_risk] || RISK_STYLE.caution;
    // 危险进程：拦截，给出明确说明
    if (p.kill_risk === 'dangerous') {
      alert(`已阻止终止「${p.name}」\n\n原因：${p.kill_reason}\n影响：${p.kill_impact}`);
      return;
    }
    const warn = p.kill_risk === 'caution'
      ? `⚠️ 谨慎终止\n\n「${p.name}」：${p.kill_reason}\n终止影响：${p.kill_impact}\n\n确定仍要终止吗？`
      : `确定要终止进程「${p.name}」(${p.pid}) 吗？\n\n${p.kill_reason}`;
    if (!confirm(warn)) return;
    try {
      const msg = await invoke<string>('kill_process', { pid: p.pid });
      alert(msg);
      await loadProcesses();
    } catch (e) {
      alert(`终止失败: ${String(e)}`);
    }
  };

  useEffect(() => {
    loadProcesses();
  }, []);

  const sorted = [...processes].sort((a, b) => {
    return sortBy === 'memory' ? b.memory_mb - a.memory_mb : b.cpu_percent - a.cpu_percent;
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">进程管家</h2>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
            每个进程都标注了终止建议与影响，帮你判断能不能安全关闭
          </p>
        </div>
        <button onClick={loadProcesses} disabled={loading} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50">
          {loading ? '刷新中...' : '刷新'}
        </button>
      </div>

      {/* Sort Toggle */}
      <div className="flex gap-2">
        <button onClick={() => setSortBy('memory')} className={`px-3 py-1.5 rounded-lg text-sm ${sortBy === 'memory' ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400' : 'bg-gray-100 dark:bg-gray-700'}`}>按内存排序</button>
        <button onClick={() => setSortBy('cpu')} className={`px-3 py-1.5 rounded-lg text-sm ${sortBy === 'cpu' ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400' : 'bg-gray-100 dark:bg-gray-700'}`}>按CPU排序</button>
      </div>

      {error && (
        <div className="px-4 py-3 rounded-lg bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 text-sm">{error}</div>
      )}

      {/* Process Table */}
      <div className="bg-white dark:bg-gray-800 rounded-xl card-shadow overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead className="bg-gray-50 dark:bg-gray-700">
              <tr>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400">PID</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400">名称</th>
                <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400">内存</th>
                <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400">CPU%</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400">终止建议 / 影响</th>
                <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400">操作</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
              {sorted.map((p) => {
                const style = RISK_STYLE[p.kill_risk] || RISK_STYLE.caution;
                return (
                  <tr key={p.pid} className="hover:bg-gray-50 dark:hover:bg-gray-700/50">
                    <td className="px-4 py-3 font-mono text-gray-500">{p.pid}</td>
                    <td className="px-4 py-3 font-medium text-gray-900 dark:text-white">
                      <div className="flex items-center gap-2">
                        <span className={`w-2 h-2 rounded-full flex-shrink-0 ${style.dot}`}></span>
                        {p.name}
                      </div>
                    </td>
                    <td className="px-4 py-3 text-right">{p.memory_mb} MB</td>
                    <td className="px-4 py-3 text-right">
                      <span className={`px-2 py-0.5 rounded-full text-xs ${p.cpu_percent > 50 ? 'bg-red-100 text-red-700' : 'bg-green-100 text-green-700'}`}>
                        {p.cpu_percent.toFixed(1)}%
                      </span>
                    </td>
                    <td className="px-4 py-3 max-w-md">
                      <div className="flex flex-col gap-1">
                        <span className={`inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium w-fit ${style.badge}`}>
                          {style.label}
                        </span>
                        <span className="text-xs text-gray-500 dark:text-gray-400" title={p.kill_impact}>
                          {p.kill_reason} · {p.kill_impact}
                        </span>
                      </div>
                    </td>
                    <td className="px-4 py-3 text-right">
                      <button
                        onClick={() => handleKill(p)}
                        className={`text-sm font-medium ${p.kill_risk === 'dangerous' ? 'text-gray-400 dark:text-gray-500' : p.kill_risk === 'caution' ? 'text-amber-600 hover:text-amber-700' : 'text-red-500 hover:text-red-700'}`}
                        title={p.kill_risk === 'dangerous' ? p.kill_reason : undefined}
                      >
                        {p.kill_risk === 'dangerous' ? '已阻止' : '终止'}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        {sorted.length === 0 && !loading && (
          <p className="text-center text-gray-500 py-8">无进程数据</p>
        )}
      </div>

      {/* Legend */}
      <div className="flex flex-wrap items-center gap-4 text-xs text-gray-500 dark:text-gray-400">
        <span className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-full bg-green-500"></span>可以终止：后台辅助进程，影响较小</span>
        <span className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-full bg-amber-500"></span>谨慎终止：应用/浏览器，可能有未保存数据</span>
        <span className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-full bg-red-500"></span>不建议终止：系统关键进程，已禁止</span>
      </div>
    </div>
  );
}
