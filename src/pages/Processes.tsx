import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export default function Processes() {
  const [processes, setProcesses] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [sortBy, setSortBy] = useState<'memory' | 'cpu'>('memory');

  const loadProcesses = async () => {
    setLoading(true);
    try {
      const procs = await invoke<any[]>('get_processes', { limit: 50 });
      setProcesses(procs);
    } catch (e) {
      console.error('Failed to load processes:', e);
    } finally {
      setLoading(false);
    }
  };

  const handleKill = async (pid: number, name: string) => {
    if (!confirm(`确定要终止进程 "${name}" (${pid}) 吗？`)) return;
    try {
      await invoke('kill_process', { pid });
      await loadProcesses();
    } catch (e) {
      alert(String(e));
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
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">进程管家</h2>
        <button onClick={loadProcesses} disabled={loading} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50">
          {loading ? '刷新中...' : '刷新'}
        </button>
      </div>

      {/* Sort Toggle */}
      <div className="flex gap-2">
        <button onClick={() => setSortBy('memory')} className={`px-3 py-1.5 rounded-lg text-sm ${sortBy === 'memory' ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400' : 'bg-gray-100 dark:bg-gray-700'}`}>按内存排序</button>
        <button onClick={() => setSortBy('cpu')} className={`px-3 py-1.5 rounded-lg text-sm ${sortBy === 'cpu' ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400' : 'bg-gray-100 dark:bg-gray-700'}`}>按CPU排序</button>
      </div>

      {/* Process Table */}
      <div className="bg-white dark:bg-gray-800 rounded-xl card-shadow overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-gray-50 dark:bg-gray-700">
            <tr>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400">PID</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400">名称</th>
              <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400">内存</th>
              <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400">CPU%</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400">状态</th>
              <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
            {sorted.map((p) => (
              <tr key={p.pid} className="hover:bg-gray-50 dark:hover:bg-gray-700/50">
                <td className="px-4 py-3 font-mono text-gray-500">{p.pid}</td>
                <td className="px-4 py-3 font-medium text-gray-900 dark:text-white">{p.name}</td>
                <td className="px-4 py-3 text-right">{p.memory_mb} MB</td>
                <td className="px-4 py-3 text-right">
                  <span className={`px-2 py-0.5 rounded-full text-xs ${p.cpu_percent > 50 ? 'bg-red-100 text-red-700' : 'bg-green-100 text-green-700'}`}>
                    {p.cpu_percent.toFixed(1)}%
                  </span>
                </td>
                <td className="px-4 py-3 text-gray-500">{p.status}</td>
                <td className="px-4 py-3 text-right">
                  <button onClick={() => handleKill(p.pid, p.name)} className="text-red-500 hover:text-red-700 text-sm">终止</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {sorted.length === 0 && !loading && (
          <p className="text-center text-gray-500 py-8">无进程数据</p>
        )}
      </div>
    </div>
  );
}
