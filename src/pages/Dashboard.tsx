import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { checkForUpdate } from '@/lib/versionCheck';

interface SystemInfo {
  cpu?: { usage_percent: number; core_count: number };
  memory?: { used_percent: number; total_mb: number; used_mb: number };
  disks?: { name: string; used_percent: number; total_gb: number; available_gb: number }[];
}

interface ProcessItem {
  pid: number;
  name: string;
  memory_mb: number;
  cpu_percent: number;
}

interface UpdateInfo {
  hasUpdate: boolean;
  version?: string;
  downloadUrl?: string;
  note?: string;
  error?: string;
}

export default function Dashboard() {
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [topProcesses, setTopProcesses] = useState<ProcessItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [lastUpdated, setLastUpdated] = useState<string>('');
  const [pageLoaded, setPageLoaded] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await invoke<SystemInfo>('get_system_info');
      setInfo(data);
      setPageLoaded(true);
      
      const procs = await invoke<ProcessItem[]>('get_processes', { limit: 5 });
      setTopProcesses(procs);
      
      setLastUpdated(new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }));
    } catch (e) {
      console.error('Failed to load system info:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    const interval = setInterval(load, 8000);
    return () => clearInterval(interval);
  }, [load]);

  // Check for updates on mount (non-blocking)
  useEffect(() => {
    checkForUpdate().then(setUpdateInfo).catch(console.error);
  }, []);

  if (loading && !info) {
    return (
      <div className="space-y-6 animate-pulse">
        <div className="h-8 w-32 bg-gray-200 dark:bg-gray-700 rounded-lg" />
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {[1, 2, 3].map(i => (
            <div key={i} className="h-40 bg-white dark:bg-gray-800 rounded-xl card-shadow" />
          ))}
        </div>
      </div>
    );
  }

  const cpu = info?.cpu;
  const mem = info?.memory;
  const disk = info?.disks?.[0];

  const getStatusColor = (percent: number) => {
    if (percent < 70) return 'from-green-400 to-green-500';
    if (percent < 90) return 'from-yellow-400 to-yellow-500';
    return 'from-red-400 to-red-500';
  };

  const handleCleanTemp = async () => {
    try {
      const result = await invoke<string>('clean_temp_files');
      alert(result || '清理完成');
      await load();
    } catch (e) {
      alert(`清理失败: ${String(e)}`);
    }
  };

  const navigateTo = (hash: string) => {
    window.location.hash = hash;
  };

  return (
    <div className={`space-y-6 transition-all duration-500 ${pageLoaded ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-2'}`}>
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">系统概览</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">最后更新: {lastUpdated}</p>
        </div>
        <div className="flex items-center gap-3">
          {updateInfo?.hasUpdate && (
            <a
              href={updateInfo.downloadUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="px-3 py-1.5 text-xs bg-orange-500 text-white rounded-lg hover:bg-orange-600 transition-colors flex items-center gap-1"
            >
              <span>🆕</span>
              <span>新版 {updateInfo.version}</span>
            </a>
          )}
          <button
            onClick={load}
            disabled={loading}
            className="px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-all disabled:opacity-50 flex items-center gap-2"
          >
            <span>{loading ? '刷新中...' : '🔄 刷新'}</span>
          </button>
        </div>
      </div>

      {/* System Status Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {/* CPU Card */}
        <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow hover:shadow-lg transition-shadow">
          <div className="flex items-start justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-full bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center text-xl">
                🧠
              </div>
              <div>
                <p className="text-sm text-gray-500 dark:text-gray-400">CPU</p>
                <p className="text-2xl font-bold text-gray-900 dark:text-white">
                  {cpu?.usage_percent.toFixed(1)}%
                </p>
              </div>
            </div>
            <span className="text-xs text-gray-400">{cpu?.core_count} 核</span>
          </div>
          <div className="relative pt-1">
            <div className="flex mb-2 items-center justify-between">
              <span className="text-xs font-semibold inline-block py-1 px-2 uppercase rounded-full text-blue-600 bg-blue-200 dark:bg-blue-900 dark:text-blue-400">
                {(cpu?.usage_percent ?? 0) < 70 ? '正常' : (cpu?.usage_percent ?? 0) < 90 ? '负载较高' : '高负载'}
              </span>
            </div>
            <div className="overflow-hidden h-2 mb-4 text-xs flex rounded bg-gray-200 dark:bg-gray-700">
              <div 
                style={{ width: `${Math.min(cpu?.usage_percent || 0, 100)}%` }}
                className={`shadow-none flex flex-col text-center whitespace-nowrap text-white justify-center bg-gradient-to-r ${getStatusColor(cpu?.usage_percent || 0)} transition-all duration-700 ease-out`}
              />
            </div>
          </div>
        </div>

        {/* Memory Card */}
        <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow hover:shadow-lg transition-shadow">
          <div className="flex items-start justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-full bg-purple-100 dark:bg-purple-900/30 flex items-center justify-center text-xl">
                💾
              </div>
              <div>
                <p className="text-sm text-gray-500 dark:text-gray-400">内存</p>
                <p className="text-2xl font-bold text-gray-900 dark:text-white">
                  {mem?.used_percent.toFixed(1)}%
                </p>
              </div>
            </div>
            <span className="text-xs text-gray-400">{((mem?.total_mb || 0) / 1024).toFixed(1)} GB</span>
          </div>
          <div className="relative pt-1">
            <div className="flex mb-2 items-center justify-between">
              <span className="text-xs font-semibold inline-block py-1 px-2 uppercase rounded-full text-purple-600 bg-purple-200 dark:bg-purple-900 dark:text-purple-400">
                {(mem?.used_percent ?? 0) < 70 ? '充足' : (mem?.used_percent ?? 0) < 90 ? '建议关闭应用' : '紧张'}
              </span>
            </div>
            <div className="overflow-hidden h-2 mb-4 text-xs flex rounded bg-gray-200 dark:bg-gray-700">
              <div 
                style={{ width: `${Math.min(mem?.used_percent || 0, 100)}%` }}
                className={`shadow-none flex flex-col text-center whitespace-nowrap text-white justify-center bg-gradient-to-r ${getStatusColor(mem?.used_percent || 0)} transition-all duration-700 ease-out`}
              />
            </div>
            <p className="text-xs text-gray-500 dark:text-gray-400">{mem?.used_mb} MB / {mem?.total_mb} MB</p>
          </div>
        </div>

        {/* Disk Card */}
        <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow hover:shadow-lg transition-shadow">
          <div className="flex items-start justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-full bg-orange-100 dark:bg-orange-900/30 flex items-center justify-center text-xl">
                💿
              </div>
              <div>
                <p className="text-sm text-gray-500 dark:text-gray-400">磁盘</p>
                <p className="text-2xl font-bold text-gray-900 dark:text-white">
                  {disk?.used_percent.toFixed(1)}%
                </p>
              </div>
            </div>
            <span className="text-xs text-gray-400">{disk?.name.replace(/^.*(macintosh|Macintosh)/i, '')}</span>
          </div>
          <div className="relative pt-1">
            <div className="flex mb-2 items-center justify-between">
              <span className="text-xs font-semibold inline-block py-1 px-2 uppercase rounded-full text-orange-600 bg-orange-200 dark:bg-orange-900 dark:text-orange-400">
                {(disk?.used_percent ?? 0) < 70 ? '空间充足' : (disk?.used_percent ?? 0) < 90 ? '空间不足' : '空间危险'}
              </span>
            </div>
            <div className="overflow-hidden h-2 mb-4 text-xs flex rounded bg-gray-200 dark:bg-gray-700">
              <div 
                style={{ width: `${Math.min(disk?.used_percent || 0, 100)}%` }}
                className={`shadow-none flex flex-col text-center whitespace-nowrap text-white justify-center bg-gradient-to-r ${getStatusColor(disk?.used_percent || 0)} transition-all duration-700 ease-out`}
              />
            </div>
            <p className="text-xs text-gray-500 dark:text-gray-400">
              {((disk?.total_gb ?? 0) - (disk?.available_gb ?? 0)).toFixed(1)} GB / {(disk?.total_gb ?? 0).toFixed(1)} GB
            </p>
          </div>
        </div>
      </div>

      {/* Top Processes */}
      <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-medium text-gray-900 dark:text-white">内存占用 TOP5</h3>
          <button onClick={() => navigateTo('#processes')} className="text-sm text-blue-600 hover:text-blue-700">查看全部 →</button>
        </div>
        <div className="space-y-2">
          {topProcesses.map((proc, idx) => (
            <div key={proc.pid} className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors">
              <div className="flex items-center gap-3">
                <span className="w-6 h-6 rounded-full bg-gray-200 dark:bg-gray-600 flex items-center justify-center text-xs font-medium text-gray-600 dark:text-gray-300">
                  {idx + 1}
                </span>
                <div>
                  <p className="font-medium text-gray-900 dark:text-white text-sm">{proc.name}</p>
                  <p className="text-xs text-gray-500 dark:text-gray-400">PID: {proc.pid}</p>
                </div>
              </div>
              <div className="flex items-center gap-4">
                <div className="w-24">
                  <div className="h-1.5 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
                    <div 
                      className="h-full bg-gradient-to-r from-blue-400 to-blue-600 rounded-full transition-all duration-500"
                      style={{ width: `${Math.min(proc.memory_mb / ((mem?.total_mb || 1) * 0.05) * 100, 100)}%` }}
                    />
                  </div>
                </div>
                <span className="text-sm font-medium text-gray-700 dark:text-gray-300 w-16 text-right">{proc.memory_mb} MB</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Quick Actions */}
      <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">快速操作</h3>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <button
            onClick={handleCleanTemp}
            className="flex flex-col items-center gap-2 p-4 bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-700 dark:to-gray-800 rounded-xl hover:from-blue-50 hover:to-blue-100 dark:hover:from-blue-900/20 dark:hover:to-blue-800/20 transition-all hover:shadow-md group"
          >
            <span className="text-2xl group-hover:scale-110 transition-transform">🧹</span>
            <span className="text-sm text-gray-700 dark:text-gray-300 font-medium">清理缓存</span>
          </button>
          <button
            onClick={() => navigateTo('#files')}
            className="flex flex-col items-center gap-2 p-4 bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-700 dark:to-gray-800 rounded-xl hover:from-purple-50 hover:to-purple-100 dark:hover:from-purple-900/20 dark:hover:to-purple-800/20 transition-all hover:shadow-md group"
          >
            <span className="text-2xl group-hover:scale-110 transition-transform">🔍</span>
            <span className="text-sm text-gray-700 dark:text-gray-300 font-medium">大文件扫描</span>
          </button>
          <button
            onClick={() => navigateTo('#processes')}
            className="flex flex-col items-center gap-2 p-4 bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-700 dark:to-gray-800 rounded-xl hover:from-green-50 hover:to-green-100 dark:hover:from-green-900/20 dark:hover:to-green-800/20 transition-all hover:shadow-md group"
          >
            <span className="text-2xl group-hover:scale-110 transition-transform">📊</span>
            <span className="text-sm text-gray-700 dark:text-gray-300 font-medium">进程查看</span>
          </button>
          <button
            onClick={() => navigateTo('#tasks')}
            className="flex flex-col items-center gap-2 p-4 bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-700 dark:to-gray-800 rounded-xl hover:from-orange-50 hover:to-orange-100 dark:hover:from-orange-900/20 dark:hover:to-orange-800/20 transition-all hover:shadow-md group"
          >
            <span className="text-2xl group-hover:scale-110 transition-transform">⏰</span>
            <span className="text-sm text-gray-700 dark:text-gray-300 font-medium">定时任务</span>
          </button>
        </div>
      </div>
    </div>
  );
}
