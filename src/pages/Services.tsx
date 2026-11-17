import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface ServiceStatus {
  id: string;
  name: string;
  url: string;
  healthy: boolean;
  mode: string;
  icon: string;
  detail?: string;
}

export default function Services() {
  const [services, setServices] = useState<ServiceStatus[]>([]);
  const [probing, setProbing] = useState(false);

  const loadAll = async () => {
    setProbing(true);
    try {
      const results = await invoke<ServiceStatus[]>('run_all_probes');
      setServices(results);
    } catch (e) {
      console.error('Failed to probe services:', e);
    } finally {
      setProbing(false);
    }
  };

  useEffect(() => {
    loadAll();
  }, []);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">服务管家</h2>
        <button
          onClick={loadAll}
          disabled={probing}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors"
        >
          {probing ? '检测中...' : '刷新'}
        </button>
      </div>

      {/* Quick Actions */}
      <div className="bg-gradient-to-r from-blue-50 to-purple-50 dark:from-blue-900/20 dark:to-purple-900/20 rounded-xl p-4 border border-blue-100 dark:border-blue-800">
        <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">快速操作</h3>
        <div className="flex gap-3 text-xs text-gray-500 dark:text-gray-400">
          <span>🟢 Ollama → :11434</span>
          <span>🐋 OrcaRouter → :7001</span>
          <span>🧬 ChromaDB → :8100</span>
          <span>🔍 SearXNG → :8081</span>
        </div>
      </div>

      {/* Service Cards */}
      <div className="grid gap-4 md:grid-cols-2">
        {services.map((svc) => (
          <div
            key={svc.id}
            className={`rounded-xl border-2 p-5 transition-all hover:shadow-md ${
              svc.healthy
                ? 'border-green-200 dark:border-green-800 bg-green-50 dark:bg-green-900/10'
                : 'border-red-200 dark:border-red-800 bg-red-50 dark:bg-red-900/10'
            }`}
          >
            <div className="flex items-start justify-between">
              <div className="flex items-center gap-3">
                <span className="text-3xl">{svc.icon}</span>
                <div>
                  <h3 className="font-semibold text-gray-900 dark:text-white">{svc.name}</h3>
                  <p className="text-sm text-gray-500 dark:text-gray-400 font-mono">{svc.url}</p>
                </div>
              </div>
              <span className={`px-2 py-1 rounded-full text-xs font-medium ${
                svc.healthy
                  ? 'bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400'
                  : 'bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400'
              }`}>
                {svc.healthy ? '在线' : '离线'}
              </span>
            </div>
            {svc.detail && (
              <p className="mt-3 text-xs text-gray-500 dark:text-gray-400 break-all font-mono">
                {svc.detail.length > 100 ? svc.detail.slice(0, 100) + '...' : svc.detail}
              </p>
            )}
            <div className="mt-3 flex items-center gap-2 text-xs text-gray-400 dark:text-gray-500">
              <span className="px-2 py-0.5 bg-gray-100 dark:bg-gray-700 rounded">{svc.mode.replace('probe_level_', '')}</span>
              <span>HTTP Body 检测</span>
            </div>
          </div>
        ))}
      </div>

      {/* Manual Probe Buttons */}
      <div className="pt-4 border-t border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">手动检测</h3>
        <div className="flex flex-wrap gap-2">
          {services.map((svc) => (
            <button
              key={svc.id}
              onClick={() => {
                invoke('probe_service', { serviceId: svc.id }).then(loadAll).catch(console.error);
              }}
              className="px-3 py-1.5 text-sm bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg hover:border-blue-400 dark:hover:border-blue-500 transition-colors"
            >
              {svc.icon} {svc.name}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
