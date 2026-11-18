import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Category = 'Productivity' | 'Development' | 'Communication' | 'Entertainment' | 'Utility' | 'Other';

interface AppEntry {
  id: string;
  name: string;
  path: string;
  size_mb: number;
  category: string;
}

const categoryLabels: Record<string, string> = {
  Productivity: '生产力',
  Development: '开发工具',
  Communication: '通讯社交',
  Entertainment: '娱乐',
  Utility: '工具',
  Other: '其他',
};

export default function Apps() {
  const [apps, setApps] = useState<AppEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedCategory, setSelectedCategory] = useState<string>('All');
  const [cleanupInfo, setCleanupInfo] = useState<string>('');
  const [showConfirm, setShowConfirm] = useState<{app: AppEntry} | null>(null);

  const [loadError, setLoadError] = useState<string>('');

  const loadApps = async () => {
    setLoading(true);
    setLoadError('');
    try {
      const result = await invoke<AppEntry[]>('list_installed_apps');
      setApps(result);
    } catch (e) {
      const msg = String(e);
      setLoadError(msg);
      console.error('Failed to load apps:', e);
    } finally {
      setLoading(false);
    }
  };

  const launchApp = async (path: string) => {
    try {
      await invoke('launch_app', { path });
    } catch (e) {
      alert(String(e));
    }
  };

  const cleanupAppData = async (path: string) => {
    try {
      const result = await invoke<string>('cleanup_app_data', { path });
      setCleanupInfo(result);
      alert(result);
    } catch (e) {
      alert(String(e));
    }
  };

  const uninstallApp = async (app: AppEntry) => {
    setShowConfirm(null);
    try {
      const result = await invoke<string>('uninstall_app', { path: app.path });
      alert(result);
      await loadApps();
    } catch (e) {
      alert(String(e));
    }
  };

  useEffect(() => {
    loadApps();
  }, []);

  const categories = ['All', ...Array.from(new Set(apps.map(a => a.category)))];
  const filtered = selectedCategory === 'All' ? apps : apps.filter(a => a.category === selectedCategory);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">应用管家</h2>
        <button onClick={loadApps} disabled={loading} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50">
          {loading ? '扫描中...' : '重新扫描'}
        </button>
      </div>

      {/* Category Filter */}
      <div className="flex flex-wrap gap-2">
        {categories.map(cat => (
          <button key={cat} onClick={() => setSelectedCategory(cat)} className={`px-3 py-1.5 rounded-full text-sm ${selectedCategory === cat ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700'}`}>
            {cat === 'All' ? '全部' : categoryLabels[cat] || cat} ({cat === 'All' ? apps.length : apps.filter(a => a.category === cat).length})
          </button>
        ))}
      </div>

      {/* Load error */}
      {loadError && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-xl p-4 text-sm text-red-700 dark:text-red-300">
          读取应用列表失败：{loadError}
        </div>
      )}

      {/* Apps Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
        {!loading && !loadError && filtered.length === 0 && (
          <div className="col-span-full bg-white dark:bg-gray-800 rounded-xl p-10 text-center card-shadow">
            <p className="text-gray-500 dark:text-gray-400 text-sm">
              {apps.length === 0 ? '未扫描到应用' : '该分类下没有应用'}
            </p>
            <p className="text-xs text-gray-400 mt-2">
              {apps.length === 0 ? '扫描范围为 /Applications 与 ~/Applications' : '试试切换到「全部」'}
            </p>
          </div>
        )}
        {filtered.map(app => (
          <div key={app.id} className="bg-white dark:bg-gray-800 rounded-xl p-4 card-shadow">
            <div className="flex items-start justify-between">
              <div>
                <h3 className="font-medium text-gray-900 dark:text-white">{app.name}</h3>
                <p className="text-xs text-gray-500 mt-1">{categoryLabels[app.category] || app.category}</p>
              </div>
              <span className="text-xs text-gray-400 bg-gray-100 dark:bg-gray-700 px-2 py-1 rounded">{app.size_mb.toFixed(0)} MB</span>
            </div>
            <div className="mt-4 flex gap-2">
              <button onClick={() => launchApp(app.path)} className="flex-1 px-3 py-1.5 bg-blue-600 text-white rounded text-sm hover:bg-blue-700">打开</button>
              <button onClick={() => cleanupAppData(app.path)} className="px-3 py-1.5 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded text-sm hover:bg-gray-200">清理</button>
              <button onClick={() => setShowConfirm({app})} className="px-3 py-1.5 text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded text-sm">卸载</button>
            </div>
          </div>
        ))}
      </div>

      {/* Uninstall Confirm Dialog */}
      {showConfirm && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-xl p-6 max-w-md w-full mx-4">
            <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-2">确认卸载</h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">将删除 "{showConfirm.app.name}" 及其所有相关数据，此操作不可撤销。</p>
            <div className="flex gap-3 justify-end">
              <button onClick={() => setShowConfirm(null)} className="px-4 py-2 text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">取消</button>
              <button onClick={() => uninstallApp(showConfirm.app)} className="px-4 py-2 bg-red-600 text-white rounded-lg hover:bg-red-700">确认卸载</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
