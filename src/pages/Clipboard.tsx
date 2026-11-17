import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface ClipEntry {
  id: number;
  content: string;
  kind: string; // "text" | "html"
  created_at: number;
  pinned: boolean;
}

export default function Clipboard() {
  const [entries, setEntries] = useState<ClipEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState<'all' | 'pinned'>('all');
  const [toastMsg, setToastMsg] = useState<string>('');

  const showToast = (msg: string) => {
    setToastMsg(msg);
    setTimeout(() => setToastMsg(''), 2000);
  };

  const loadEntries = async () => {
    setLoading(true);
    try {
      const result = await invoke<ClipEntry[]>('clip_list');
      setEntries(result);
    } catch (e) {
      console.error('Failed to load clips:', e);
      showToast('加载失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadEntries();
  }, []);

  const togglePin = async (id: number) => {
    try {
      const newState = await invoke<boolean>('clip_toggle_pin', { id });
      showToast(newState ? '已置顶' : '已取消置顶');
      await loadEntries();
    } catch (e) {
      showToast('操作失败');
    }
  };

  const deleteEntry = async (id: number) => {
    try {
      await invoke('clip_delete', { id });
      showToast('已删除');
      await loadEntries();
    } catch (e) {
      showToast('删除失败');
    }
  };

  const copyContent = async (content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      showToast('已复制');
    } catch (e) {
      showToast('复制失败');
    }
  };

  const filteredEntries = entries.filter((e) =>
    filter === 'pinned' ? e.pinned : true
  );

  const formatTime = (ts: number) => {
    const d = new Date(ts * 1000);
    return d.toLocaleString('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  };

  return (
    <div className="space-y-6">
      {/* Toast notification */}
      {toastMsg && (
        <div className="fixed top-4 right-4 z-50 px-4 py-2 bg-gray-900 dark:bg-white text-white dark:text-gray-900 rounded-lg shadow-lg text-sm font-medium animate-in fade-in slide-in-from-top-2">
          {toastMsg}
        </div>
      )}

      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">剪贴板管家</h2>
        <button
          onClick={loadEntries}
          disabled={loading}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors"
        >
          {loading ? '加载中...' : '刷新'}
        </button>
      </div>

      {/* Filter tabs */}
      <div className="flex gap-2 border-b border-gray-200 dark:border-gray-700">
        <button
          onClick={() => setFilter('all')}
          className={`px-4 py-2 text-sm font-medium ${
            filter === 'all'
              ? 'border-b-2 border-blue-600 text-blue-600 dark:text-blue-400'
              : 'text-gray-500 hover:text-gray-700 dark:hover:text-gray-300'
          }`}
        >
          全部 ({entries.length})
        </button>
        <button
          onClick={() => setFilter('pinned')}
          className={`px-4 py-2 text-sm font-medium ${
            filter === 'pinned'
              ? 'border-b-2 border-blue-600 text-blue-600 dark:text-blue-400'
              : 'text-gray-500 hover:text-gray-700 dark:hover:text-gray-300'
          }`}
        >
          已置顶 ({entries.filter(e => e.pinned).length})
        </button>
      </div>

      {/* Entry list */}
      {filteredEntries.length === 0 ? (
        <div className="text-center py-12 text-gray-400 dark:text-gray-500">
          <div className="text-4xl mb-2">📋</div>
          <p>暂无记录</p>
          <p className="text-xs mt-1">安装后将从系统剪贴板自动捕获</p>
        </div>
      ) : (
        <div className="space-y-3">
          {filteredEntries.map((entry) => (
            <div
              key={entry.id}
              className={`rounded-lg border p-4 transition-all hover:shadow-md ${
                entry.pinned
                  ? 'border-yellow-200 dark:border-yellow-800 bg-yellow-50 dark:bg-yellow-900/10'
                  : 'border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800'
              }`}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    {entry.pinned && <span className="text-yellow-500">📌</span>}
                    <span className="text-xs px-2 py-0.5 rounded-full bg-gray-100 dark:bg-gray-700 text-gray-500">
                      {entry.kind}
                    </span>
                    <span className="text-xs text-gray-400">{formatTime(entry.created_at)}</span>
                  </div>
                  <p className="text-sm text-gray-800 dark:text-gray-200 break-words whitespace-pre-wrap font-mono">
                    {entry.content.length > 200
                      ? entry.content.slice(0, 200) + '...'
                      : entry.content}
                  </p>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <button
                    onClick={() => togglePin(entry.id)}
                    className="p-1.5 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"
                    title={entry.pinned ? '取消置顶' : '置顶'}
                  >
                    {entry.pinned ? '⭐' : '☆'}
                  </button>
                  <button
                    onClick={() => copyContent(entry.content)}
                    className="p-1.5 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"
                    title="复制"
                  >
                    📋
                  </button>
                  <button
                    onClick={() => deleteEntry(entry.id)}
                    className="p-1.5 hover:bg-red-50 dark:hover:bg-red-900/20 rounded text-red-500"
                    title="删除"
                  >
                    🗑️
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Stats footer */}
      <div className="pt-4 border-t border-gray-200 dark:border-gray-700 text-xs text-gray-400 dark:text-gray-500 flex justify-between">
        <span>共 {entries.length} 条记录</span>
        <span>自动保存至 SQLite</span>
      </div>
    </div>
  );
}
