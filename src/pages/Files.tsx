import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export default function Files() {
  const [largeFiles, setLargeFiles] = useState<any[]>([]);
  const [tempStats, setTempStats] = useState<{freed_bytes: number; cleaned_count: number} | null>(null);
  const [loading, setLoading] = useState(false);

  const loadLargeFiles = async () => {
    setLoading(true);
    try {
      const files = await invoke<any[]>('list_large_files', { thresholdMb: 50, limit: 20 });
      setLargeFiles(files);
    } catch (e) {
      console.error('Failed to load large files:', e);
    } finally {
      setLoading(false);
    }
  };

  const handleCleanTemp = async () => {
    try {
      const result = await invoke<any>('clean_temp_files');
      setTempStats(result);
      // 刷新大文件列表
      await loadLargeFiles();
    } catch (e) {
      alert(`清理失败: ${e}`);
    }
  };

  useEffect(() => {
    loadLargeFiles();
  }, []);

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">文件管家</h2>

      {/* Quick Stats */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <StatBox title="临时文件" value={tempStats ? `${(tempStats.freed_bytes / 1_000_000).toFixed(1)} MB` : '--'} description={`${tempStats?.cleaned_count || 0} 个文件已清理`} />
        <StatBox title="大文件 (>50MB)" value={largeFiles.length.toString()} description="占用大量空间" />
        <StatBox title="Downloads" value="--" description="扫描完成" />
      </div>

      {/* Clean Temp Button */}
      <div id="files-clean" className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-3">清理临时文件</h3>
        <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">清除系统缓存、日志和临时文件</p>
        <button
          onClick={handleCleanTemp}
          disabled={loading}
          className="px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors"
        >
          {loading ? '清理中...' : '立即清理'}
        </button>
      </div>

      {/* Large Files List */}
      <div id="files-scan" className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-medium text-gray-900 dark:text-white">大文件列表</h3>
          <button onClick={loadLargeFiles} className="text-sm text-blue-600 hover:text-blue-700">刷新</button>
        </div>
        {loading ? (
          <p className="text-gray-500 text-sm text-center py-8">扫描中...</p>
        ) : largeFiles.length === 0 ? (
          <p className="text-gray-500 text-sm text-center py-8">未发现大文件</p>
        ) : (
          <div className="space-y-2">
            {largeFiles.map((file, idx) => (
              <div key={idx} className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700 rounded-lg">
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-gray-900 dark:text-white truncate">{file.name}</p>
                  <p className="text-xs text-gray-500 dark:text-gray-400">{file.path}</p>
                </div>
                <span className="ml-4 px-3 py-1 bg-yellow-100 dark:bg-yellow-900/30 text-yellow-700 dark:text-yellow-400 rounded-full text-sm font-medium">
                  {file.size_mb.toFixed(1)} MB
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function StatBox({ title, value, description }: { title: string; value: string; description: string }) {
  return (
    <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
      <p className="text-sm text-gray-500 dark:text-gray-400">{title}</p>
      <p className="text-3xl font-bold text-gray-900 dark:text-white mt-1">{value}</p>
      <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">{description}</p>
    </div>
  );
}
