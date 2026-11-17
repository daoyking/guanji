import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface NotificationItem {
  id: string;
  title: string;
  body: string;
  priority: 'Low' | 'Normal' | 'High' | 'Critical';
  category: string;
  created_at: number;
  read: boolean;
}

interface NotifyPrefs {
  enabled: boolean;
  quiet_start: number;
  quiet_end: number;
  silent_categories: string[];
  max_per_hour: number;
}

const PRIORITY_COLORS = {
  Low: 'bg-gray-100 text-gray-600 dark:bg-gray-700 dark:text-gray-400',
  Normal: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
  High: 'bg-orange-100 text-orange-700 dark:bg-orange-900/30 dark:text-orange-400',
  Critical: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
};

const CATEGORY_ICONS: Record<string, string> = {
  system: '💻',
  task: '⏰',
  memory: '🧠',
  file: '📁',
};

export default function Notifications() {
  const [items, setItems] = useState<NotificationItem[]>([]);
  const [prefs, setPrefs] = useState<NotifyPrefs>({
    enabled: true,
    quiet_start: 22,
    quiet_end: 7,
    silent_categories: [],
    max_per_hour: 20,
  });
  const [loading, setLoading] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [newBody, setNewBody] = useState('');
  const [pushing, setPushing] = useState(false);

  const loadNotifications = async () => {
    setLoading(true);
    try {
      const data = await invoke<NotificationItem[]>('notify_list');
      setItems(data);
    } catch (e) {
      console.error('Failed to load notifications:', e);
    } finally {
      setLoading(false);
    }
  };

  const loadPrefs = async () => {
    try {
      const data = await invoke<NotifyPrefs>('notify_get_prefs');
      setPrefs(data);
    } catch (e) {
      console.error('Failed to load prefs:', e);
    }
  };

  useEffect(() => {
    loadNotifications();
    loadPrefs();
  }, []);

  const pushTest = async () => {
    if (!newTitle.trim()) return;
    setPushing(true);
    try {
      await invoke('notify_push', {
        title: newTitle,
        body: newBody || '这是一条测试通知',
        category: 'system',
        priority: 'Normal',
      });
      setNewTitle('');
      setNewBody('');
      await loadNotifications();
    } catch (e) {
      alert(`推送失败: ${String(e)}`);
    } finally {
      setPushing(false);
    }
  };

  const markRead = async (id: string) => {
    try {
      await invoke('notify_mark_read', { id });
      await loadNotifications();
    } catch (e) {
      console.error('Mark read failed:', e);
    }
  };

  const clearRead = async () => {
    try {
      await invoke<number>('notify_clear_read');
      await loadNotifications();
    } catch (e) {
      console.error('Clear failed:', e);
    }
  };

  const toggleQuietHours = async () => {
    const newEnabled = !prefs.enabled;
    try {
      await invoke('notify_set_prefs', {
        newPrefs: { ...prefs, enabled: newEnabled },
      });
      setPrefs({ ...prefs, enabled: newEnabled });
    } catch (e) {
      console.error('Toggle failed:', e);
    }
  };

  const formatTime = (ts: number) => {
    try {
      return new Date(ts * 1000).toLocaleString('zh-CN', {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      });
    } catch {
      return String(ts);
    }
  };

  const unreadCount = items.filter(i => !i.read).length;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">推送中心 📬</h2>
        <div className="flex gap-2">
          <button onClick={loadNotifications} disabled={loading}
            className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 transition-colors">
            {loading ? '加载中...' : '刷新'}
          </button>
          {unreadCount > 0 && (
            <span className="px-3 py-2 bg-red-500 text-white rounded-lg text-sm font-medium">
              {unreadCount} 未读
            </span>
          )}
        </div>
      </div>

      {/* Quiet Hours Control */}
      <div className="bg-white dark:bg-gray-800 rounded-xl p-4 border border-gray-200 dark:border-gray-700">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">勿扰模式</h3>
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
              {prefs.quiet_start}:00 - {prefs.quiet_end}:00 期间仅发送严重通知
            </p>
          </div>
          <button
            onClick={toggleQuietHours}
            className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
              prefs.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                prefs.enabled ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
        </div>
      </div>

      {/* Quick Push */}
      <div className="bg-white dark:bg-gray-800 rounded-xl p-4 border border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">快速推送</h3>
        <div className="space-y-2">
          <input
            value={newTitle}
            onChange={(e) => setNewTitle(e.target.value)}
            placeholder="通知标题..."
            className="w-full px-3 py-2 text-sm bg-gray-50 dark:bg-gray-700 border border-gray-200 dark:border-gray-600 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 text-gray-900 dark:text-white"
          />
          <textarea
            value={newBody}
            onChange={(e) => setNewBody(e.target.value)}
            placeholder="通知内容（可选）..."
            rows={2}
            className="w-full px-3 py-2 text-sm bg-gray-50 dark:bg-gray-700 border border-gray-200 dark:border-gray-600 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 text-gray-900 dark:text-white resize-none"
          />
          <button onClick={pushTest} disabled={pushing || !newTitle.trim()}
            className="px-4 py-2 text-sm bg-purple-600 text-white rounded-lg hover:bg-purple-700 disabled:opacity-50 flex items-center gap-2">
            <span>{pushing ? '📨 推送中...' : '🚀 发送测试通知'}</span>
          </button>
        </div>
      </div>

      {/* Notification List */}
      <div>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            通知历史 ({items.length})
          </h3>
          {unreadCount > 0 && (
            <button onClick={clearRead}
              className="text-xs text-gray-500 hover:text-red-500 transition-colors">
              清除已读
            </button>
          )}
        </div>
        {items.length === 0 ? (
          <div className="text-center py-12 text-gray-400 dark:text-gray-500">
            <div className="text-4xl mb-2">🔔</div>
            <p className="text-sm">暂无通知，发送第一条吧！</p>
          </div>
        ) : (
          <div className="space-y-2">
            {items.map((item) => (
              <div key={item.id}
                className={`flex items-start gap-3 p-4 rounded-xl border transition-all ${
                  item.read
                    ? 'bg-white dark:bg-gray-800 border-gray-200 dark:border-gray-700 opacity-60'
                    : 'bg-white dark:bg-gray-800 border-blue-200 dark:border-blue-800 shadow-sm'
                } hover:shadow-md`}>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-lg">{CATEGORY_ICONS[item.category] || '📌'}</span>
                    <span className={`px-2 py-0.5 text-xs rounded-full ${PRIORITY_COLORS[item.priority]}`}>
                      {item.priority}
                    </span>
                    <span className="text-xs text-gray-400">{formatTime(item.created_at)}</span>
                  </div>
                  <p className="font-medium text-gray-900 dark:text-white text-sm">{item.title}</p>
                  {item.body && <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">{item.body}</p>}
                </div>
                {!item.read && (
                  <button onClick={() => markRead(item.id)}
                    className="text-gray-400 hover:text-blue-500 transition-colors shrink-0"
                    title="标记为已读">
                    ✓
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
