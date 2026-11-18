import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface MemoryItem {
  id: number;
  content: string;
  tag?: string;
  summary?: string;
  created_at: string;
}

interface SimilarityResult {
  id: number;
  content: string;
  score: number;
}

const TAG_COLORS: Record<string, string> = {
  meeting: 'bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300',
  idea: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/40 dark:text-yellow-300',
  code: 'bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-300',
  personal: 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300',
  todo: 'bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300',
};

export default function Memories() {
  const [memories, setMemories] = useState<MemoryItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [newContent, setNewContent] = useState('');
  const [analyzing, setAnalyzing] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchResults, setSearchResults] = useState<SimilarityResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [aiSummary, setAiSummary] = useState<string | null>(null);

  const loadMemories = async () => {
    setLoading(true);
    try {
      const list = await invoke<MemoryItem[]>('mem_list');
      setMemories(list);
    } catch (e) {
      console.error('Failed to load memories:', e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadMemories();
  }, []);

  const addMemory = async () => {
    if (!newContent.trim()) return;
    try {
      await invoke<number>('mem_add', { content: newContent });
      setNewContent('');
      await loadMemories();
    } catch (e) {
      alert(String(e));
    }
  };

  const deleteMemory = async (id: number) => {
    try {
      await invoke('mem_delete', { id });
      await loadMemories();
    } catch (e) {
      alert(String(e));
    }
  };

  const analyzeWithAI = async () => {
    if (!newContent.trim()) return;
    setAnalyzing(true);
    try {
      const result = await invoke<any>('mem_analyze', { text: newContent });
      // Create memory with AI-assisted data persisted to SQLite
      const id = await invoke<number>('mem_add', { content: newContent, tag: result.tag, summary: result.summary });
      await loadMemories();
    } catch (e) {
      alert(`分析失败: ${String(e)}`);
    } finally {
      setAnalyzing(false);
    }
  };

  const searchMemories = async () => {
    if (!searchQuery.trim()) return;
    setSearching(true);
    try {
      const results = await invoke<SimilarityResult[]>('mem_search', {
        query: searchQuery,
        top_n: 5
      });
      setSearchResults(results);
    } catch (e) {
      console.error('Search failed:', e);
      setSearchResults([]);
    } finally {
      setSearching(false);
    }
  };

  const formatTime = (iso: string) => {
    try {
      return new Date(iso).toLocaleString('zh-CN', {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      });
    } catch {
      return iso;
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">记忆系统 🧠</h2>
        <button onClick={loadMemories} disabled={loading}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors">
          {loading ? '加载中...' : '刷新'}
        </button>
      </div>

      {/* Search Box */}
      <div className="bg-white dark:bg-gray-800 rounded-xl p-4 border border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">语义搜索</h3>
        <div className="flex gap-2">
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && searchMemories()}
            placeholder="输入关键词搜索相关记忆..."
            className="flex-1 px-3 py-2 text-sm bg-gray-50 dark:bg-gray-700 border border-gray-200 dark:border-gray-600 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 text-gray-900 dark:text-white"
          />
          <button onClick={searchMemories} disabled={searching}
            className="px-4 py-2 text-sm bg-purple-600 text-white rounded-lg hover:bg-purple-700 disabled:opacity-50">
            {searching ? '搜索中...' : '搜索'}
          </button>
        </div>
        {searchResults.length > 0 && (
          <div className="mt-3 space-y-2">
            {searchResults.map((r) => (
              <div key={r.id} className="p-3 bg-purple-50 dark:bg-purple-900/20 rounded-lg text-sm">
                <div className="flex justify-between items-start">
                  <span className="text-gray-700 dark:text-gray-300">{r.content.slice(0, 100)}{r.content.length > 100 ? '...' : ''}</span>
                  <span className="text-xs text-purple-600 dark:text-purple-400 ml-2">相似度 {r.score.toFixed(2)}</span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Add Memory */}
      <div className="bg-white dark:bg-gray-800 rounded-xl p-4 border border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">添加记忆</h3>
        <textarea
          value={newContent}
          onChange={(e) => setNewContent(e.target.value)}
          placeholder="输入笔记或记录内容..."
          rows={3}
          className="w-full px-3 py-2 text-sm bg-gray-50 dark:bg-gray-700 border border-gray-200 dark:border-gray-600 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 text-gray-900 dark:text-white resize-none"
        />
        <div className="flex gap-2 mt-3">
          <button onClick={addMemory} disabled={!newContent.trim()}
            className="px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50">
            直接保存
          </button>
          <button onClick={analyzeWithAI} disabled={analyzing || !newContent.trim()}
            className="px-4 py-2 text-sm bg-purple-600 text-white rounded-lg hover:bg-purple-700 disabled:opacity-50 flex items-center gap-1">
            <span>{analyzing ? '🤖 分析中...' : '✨ AI分析'}</span>
          </button>
        </div>
        {aiSummary && (
          <p className="mt-2 text-xs text-purple-600 dark:text-purple-400">{aiSummary}</p>
        )}
      </div>

      {/* Memory List */}
      <div>
        <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">
          已保存 ({memories.length})
        </h3>
        {memories.length === 0 ? (
          <div className="text-center py-12 text-gray-400 dark:text-gray-500">
            <div className="text-4xl mb-2">📝</div>
            <p className="text-sm">暂无记忆，点击上方按钮添加</p>
          </div>
        ) : (
          <div className="space-y-2">
            {memories.map((m) => (
              <div key={m.id}
                className="flex items-start justify-between p-4 bg-white dark:bg-gray-800 rounded-xl border border-gray-200 dark:border-gray-700 hover:shadow-md transition-shadow">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    {m.tag && (
                      <span className={`px-2 py-0.5 text-xs rounded-full ${TAG_COLORS[m.tag] || 'bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300'}`}>
                        {m.tag}
                      </span>
                    )}
                    <span className="text-xs text-gray-400">{formatTime(m.created_at)}</span>
                  </div>
                  {m.summary && (
                    <p className="text-xs text-purple-600 dark:text-purple-400 mb-1 italic">
                      💡 {m.summary}
                    </p>
                  )}
                  <p className="text-sm text-gray-700 dark:text-gray-300 break-words">
                    {m.content.slice(0, m.content.length > 200 ? 200 : undefined)}
                    {m.content.length > 200 && '...'}
                  </p>
                </div>
                <button onClick={() => deleteMemory(m.id)}
                  className="ml-3 text-gray-400 hover:text-red-500 transition-colors shrink-0">
                  🗑️
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
