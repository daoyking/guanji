import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

interface FileScanConfig {
  scan_dirs: string[];
  ignore_dirs: string[];
}

export default function Files() {
  const [largeFiles, setLargeFiles] = useState<any[]>([]);
  const [tempStats, setTempStats] = useState<{freed_bytes: number; cleaned_count: number} | null>(null);
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<FileScanConfig>({ scan_dirs: [], ignore_dirs: [] });
  const [configOpen, setConfigOpen] = useState(false);
  const [nameInput, setNameInput] = useState('');
  const [saving, setSaving] = useState(false);

  const loadConfig = async () => {
    try {
      const cfg = await invoke<FileScanConfig>('file_get_config');
      setConfig(cfg);
    } catch (e) {
      console.error('Failed to load file config:', e);
    }
  };

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

  const saveScanDirs = async (dirs: string[]) => {
    setSaving(true);
    try {
      await invoke('file_set_scan_dirs', { dirs });
      setConfig(prev => ({ ...prev, scan_dirs: dirs }));
      await loadLargeFiles();
    } catch (e) {
      alert(`保存扫描目录失败: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const saveIgnoreDirs = async (dirs: string[]) => {
    setSaving(true);
    try {
      await invoke('file_set_ignore_dirs', { dirs });
      setConfig(prev => ({ ...prev, ignore_dirs: dirs }));
      await loadLargeFiles();
    } catch (e) {
      alert(`保存忽略目录失败: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const addScanDir = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择要扫描的文件夹' });
    if (!selected || typeof selected !== 'string') return;
    if (config.scan_dirs.includes(selected)) return;
    await saveScanDirs([...config.scan_dirs, selected]);
  };

  const removeScanDir = async (dir: string) => {
    await saveScanDirs(config.scan_dirs.filter(d => d !== dir));
  };

  const addIgnoreDir = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择要忽略的文件夹' });
    if (!selected || typeof selected !== 'string') return;
    if (config.ignore_dirs.includes(selected)) return;
    await saveIgnoreDirs([...config.ignore_dirs, selected]);
  };

  const addIgnoreName = async () => {
    const name = nameInput.trim().replace(/^[/\\]+|[/\\]+$/g, '');
    if (!name) return;
    if (name.includes('/') || name.includes('\\')) {
      alert('这里只填文件夹名称（不含路径）。要忽略具体路径请用「选择文件夹」。');
      return;
    }
    if (config.ignore_dirs.includes(name)) {
      setNameInput('');
      return;
    }
    await saveIgnoreDirs([...config.ignore_dirs, name]);
    setNameInput('');
  };

  const removeIgnoreDir = async (dir: string) => {
    await saveIgnoreDirs(config.ignore_dirs.filter(d => d !== dir));
  };

  const handleCleanTemp = async () => {
    try {
      const result = await invoke<any>('clean_temp_files');
      setTempStats(result);
      await loadLargeFiles();
    } catch (e) {
      alert(`清理失败: ${e}`);
    }
  };

  useEffect(() => {
    loadConfig();
    loadLargeFiles();
  }, []);

  const ruleKind = (rule: string) => (rule.includes('/') || rule.startsWith('~') ? '路径' : '名称');

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">文件管家</h2>
        <button
          onClick={() => setConfigOpen(!configOpen)}
          className="px-4 py-2 text-sm bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-200 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
        >
          {configOpen ? '收起设置' : '⚙ 扫描与忽略设置'}
        </button>
      </div>

      {configOpen && (
        <div className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow space-y-6">
          <p className="text-sm text-gray-500 dark:text-gray-400">
            选择要扫描的文件夹，以及扫描时忽略的文件夹。修改后立即生效。
          </p>

          <div>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-base font-medium text-gray-900 dark:text-white">
                扫描目录 <span className="text-sm font-normal text-gray-400">({config.scan_dirs.length})</span>
              </h3>
              <button
                onClick={addScanDir}
                disabled={saving}
                className="px-3 py-1.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors"
              >
                + 选择文件夹
              </button>
            </div>
            {config.scan_dirs.length === 0 ? (
              <p className="text-sm text-gray-400 py-2">未配置，使用默认目录</p>
            ) : (
              <div className="space-y-2">
                {config.scan_dirs.map((dir) => (
                  <div key={dir} className="flex items-center gap-2 p-2.5 bg-gray-50 dark:bg-gray-700 rounded-lg">
                    <span className="text-blue-500 text-xs">📁</span>
                    <span className="flex-1 text-sm text-gray-700 dark:text-gray-200 truncate" title={dir}>{dir}</span>
                    <button
                      onClick={() => removeScanDir(dir)}
                      disabled={saving}
                      className="px-2 py-0.5 text-xs text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded disabled:opacity-50"
                    >
                      移除
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div className="border-t border-gray-200 dark:border-gray-700 pt-6">
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-base font-medium text-gray-900 dark:text-white">
                忽略的文件夹 <span className="text-sm font-normal text-gray-400">({config.ignore_dirs.length})</span>
              </h3>
              <button
                onClick={addIgnoreDir}
                disabled={saving}
                className="px-3 py-1.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors"
              >
                + 选择文件夹
              </button>
            </div>
            <div className="flex gap-2 mb-3">
              <input
                value={nameInput}
                onChange={(e) => setNameInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') addIgnoreName(); }}
                placeholder="输入文件夹名称，如 node_modules（任意层级都忽略）"
                className="flex-1 px-3 py-2 text-sm border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
              />
              <button
                onClick={addIgnoreName}
                disabled={saving || !nameInput.trim()}
                className="px-4 py-2 text-sm bg-gray-600 text-white rounded-lg hover:bg-gray-700 disabled:opacity-50 transition-colors"
              >
                添加名称
              </button>
            </div>
            {config.ignore_dirs.length === 0 ? (
              <p className="text-sm text-gray-400 py-2">未配置忽略规则</p>
            ) : (
              <div className="space-y-2">
                {config.ignore_dirs.map((dir) => (
                  <div key={dir} className="flex items-center gap-2 p-2.5 bg-gray-50 dark:bg-gray-700 rounded-lg">
                    <span className="px-1.5 py-0.5 text-[10px] bg-purple-100 dark:bg-purple-900/30 text-purple-600 dark:text-purple-300 rounded">
                      {ruleKind(dir)}
                    </span>
                    <span className="flex-1 text-sm text-gray-700 dark:text-gray-200 truncate" title={dir}>{dir}</span>
                    <button
                      onClick={() => removeIgnoreDir(dir)}
                      disabled={saving}
                      className="px-2 py-0.5 text-xs text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded disabled:opacity-50"
                    >
                      移除
                    </button>
                  </div>
                ))}
              </div>
            )}
            <p className="mt-3 text-xs text-gray-400">
              提示：「名称」规则会忽略任意层级下同名的文件夹（如所有 node_modules）；「路径」规则只忽略指定文件夹本身及其子目录。
            </p>
          </div>
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <StatBox title="临时文件" value={tempStats ? `${(tempStats.freed_bytes / 1_000_000).toFixed(1)} MB` : '--'} description={`${tempStats?.cleaned_count || 0} 个文件已清理`} />
        <StatBox title="大文件 (>50MB)" value={largeFiles.length.toString()} description="占用大量空间" />
        <StatBox title="忽略规则" value={config.ignore_dirs.length.toString()} description="已配置的忽略项" />
      </div>

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
