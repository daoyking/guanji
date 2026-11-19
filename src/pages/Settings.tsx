import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface MemoryItem {
  id: number;
  content: string;
  created_at: string;
}

interface SettingsProps {
  isDark: boolean;
  onToggleTheme: (dark: boolean) => void;
}

export default function Settings({ isDark, onToggleTheme }: SettingsProps) {
  const [memories, setMemories] = useState<MemoryItem[]>([]);
  const [newNote, setNewNote] = useState('');
  const [autoStart, setAutoStart] = useState(false);
  const [notifPermission, setNotifPermission] = useState<'unauthorized' | 'authorized'>('unauthorized');
  const [autoMode, setAutoMode] = useState<'ask' | 'safe' | 'all'>('safe');
  const [cleanThreshold, setCleanThreshold] = useState('0');
  const [autoMsg, setAutoMsg] = useState('');

  useEffect(() => {
    loadMemories();
    checkNotificationPermission();
    loadAutoSettings();
  }, []);

  const loadAutoSettings = async () => {
    try {
      const [mode, threshold] = await Promise.all([
        invoke<string>('get_setting', { key: 'agent_auto_mode' }),
        invoke<string>('get_setting', { key: 'agent_auto_threshold_mb' }),
      ]);
      if (mode === 'safe' || mode === 'all' || mode === 'ask') setAutoMode(mode);
      if (threshold) setCleanThreshold(threshold);
    } catch { /* 首次运行无配置，保持默认 */ }
  };

  const saveAutoMode = async (mode: 'ask' | 'safe' | 'all') => {
    setAutoMode(mode);
    await invoke('set_setting', { key: 'agent_auto_mode', value: mode });
    setAutoMsg('已保存，下次对话立即生效');
  };

  const saveThreshold = async () => {
    const v = Number(cleanThreshold);
    if (!Number.isFinite(v) || v < 0) { setAutoMsg('请输入有效的非负数字'); return; }
    await invoke('set_setting', { key: 'agent_auto_threshold_mb', value: String(v) });
    setAutoMsg(`已保存：清理释放量 ≤ ${v} MB 时自动执行（0 为不启用阈值）`);
  };

  const loadMemories = async () => {
    try {
      const list = await invoke<MemoryItem[]>('list_memories');
      setMemories(list);
    } catch (e) {
      console.error(e);
    }
  };

  const addMemory = async () => {
    if (!newNote.trim()) return;
    try {
      await invoke<number>('add_memory', { content: newNote });
      setNewNote('');
      await loadMemories();
    } catch (e) {
      alert(String(e));
    }
  };

  const deleteMemory = async (id: number) => {
    try {
      await invoke<void>('delete_memory', { id });
      await loadMemories();
    } catch (e) {
      alert(String(e));
    }
  };

  const checkNotificationPermission = async () => {
    try {
      const perm = await invoke<string>('check_notification_permission');
      setNotifPermission(perm === 'granted' ? 'authorized' : 'unauthorized');
    } catch {
      setNotifPermission('unauthorized');
    }
  };

  const requestPermission = () => {
    window.open('x-apple.systempreferences:com.apple.preference.notifications', '_blank');
  };

  const sendTestNotification = async () => {
    try {
      await invoke('send_notification', { title: '管家测试', body: '这是一条测试通知' });
    } catch (e) {
      alert(String(e));
    }
  };

  const handleToggleTheme = async (dark: boolean) => {
    onToggleTheme(dark);
    await invoke('set_setting', { key: 'theme', value: dark ? 'dark' : 'light' });
  };

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">设置</h2>

      {/* Appearance */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">外观</h3>
        <div className="flex items-center justify-between">
          <span className="text-gray-600 dark:text-gray-400">深色模式</span>
          <button
            onClick={() => handleToggleTheme(!isDark)}
            className={`relative w-12 h-6 rounded-full transition-colors ${isDark ? 'bg-blue-600' : 'bg-gray-300'}`}
          >
            <span className={`absolute top-0.5 left-0.5 w-5 h-5 bg-white rounded-full transform transition-transform ${isDark ? 'translate-x-6' : ''}`} />
          </button>
        </div>
      </section>

      {/* Notifications */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">通知</h3>
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <span className="text-gray-600 dark:text-gray-400">桌面通知权限</span>
            <span className={`px-3 py-1 rounded-full text-sm ${notifPermission === 'authorized' ? 'bg-green-100 text-green-700' : 'bg-yellow-100 text-yellow-700'}`}>
              {notifPermission === 'authorized' ? '已授权' : '未授权'}
            </span>
          </div>
          <div className="flex gap-3">
            <button onClick={requestPermission} className="px-4 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg text-sm hover:bg-gray-200 dark:hover:bg-gray-600">
              打开系统设置
            </button>
            <button onClick={sendTestNotification} className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700">
              发送测试通知
            </button>
          </div>
        </div>
      </section>

      {/* Agent 自动执行 */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-2">Agent 自动执行</h3>
        <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
          控制 Agent 调用工具时是否需要你手动确认。清理类操作都是移到废纸篓，可随时恢复。
        </p>
        <div className="space-y-3">
          {([
            { key: 'ask', label: '每次都确认', desc: '所有删除/命令/写入操作都弹窗确认，最保守' },
            { key: 'safe', label: '自动执行安全操作（默认）', desc: '清理缓存等可恢复操作直接执行；命令/写入仍需确认' },
            { key: 'all', label: '信任模式：全部自动', desc: '所有工具调用不再询问，适合完全信任本机环境' },
          ] as const).map((opt) => (
            <button
              key={opt.key}
              onClick={() => saveAutoMode(opt.key)}
              className={`w-full text-left p-3 rounded-lg border transition-colors ${
                autoMode === opt.key
                  ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20'
                  : 'border-gray-200 dark:border-gray-700 hover:border-blue-300 dark:hover:border-blue-700'
              }`}
            >
              <div className="flex items-center gap-2">
                <span className={`w-4 h-4 rounded-full border-2 ${autoMode === opt.key ? 'border-blue-600 bg-blue-600' : 'border-gray-400'}`} />
                <span className="font-medium text-gray-900 dark:text-white">{opt.label}</span>
              </div>
              <p className="mt-1 ml-6 text-sm text-gray-500 dark:text-gray-400">{opt.desc}</p>
            </button>
          ))}
          <div className="pt-2 border-t border-gray-100 dark:border-gray-700">
            <label className="block text-sm text-gray-600 dark:text-gray-300 mb-2">
              清理自动执行阈值（MB）
              <span className="text-gray-400 dark:text-gray-500"> — 仅「安全」档生效，0 表示不启用</span>
            </label>
            <div className="flex gap-2">
              <input
                type="number"
                min="0"
                value={cleanThreshold}
                onChange={e => setCleanThreshold(e.target.value)}
                className="flex-1 px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
              />
              <button onClick={saveThreshold} className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700">
                保存
              </button>
            </div>
            <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
              设置后，预估释放量不超过该值的清理自动执行；超过则仍弹窗确认。
            </p>
          </div>
          {autoMsg && <p className="text-sm text-green-600 dark:text-green-400">{autoMsg}</p>}
        </div>
      </section>

      {/* Quick Notes */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">快速笔记</h3>
        <div className="flex gap-3 mb-4">
          <input
            value={newNote}
            onChange={e => setNewNote(e.target.value)}
            placeholder="输入一条备忘..."
            onKeyDown={e => e.key === 'Enter' && addMemory()}
            className="flex-1 px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
          />
          <button onClick={addMemory} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700">
            添加
          </button>
        </div>
        <div className="space-y-2 max-h-60 overflow-y-auto">
          {memories.length === 0 ? (
            <p className="text-sm text-gray-400 dark:text-gray-500 text-center py-4">暂无笔记</p>
          ) : memories.map(m => (
            <div key={m.id} className="flex items-start justify-between p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
              <div>
                <p className="text-sm text-gray-700 dark:text-gray-300">{m.content}</p>
                <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">{m.created_at}</p>
              </div>
              <button onClick={() => deleteMemory(m.id)} className="text-red-400 hover:text-red-600 ml-2 shrink-0">✕</button>
            </div>
          ))}
        </div>
      </section>

      {/* About */}
      <section className="bg-white dark:bg-gray-800 rounded-xl p-6 card-shadow">
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">关于</h3>
        <div className="space-y-2 text-sm text-gray-600 dark:text-gray-400">
          <p><strong>版本:</strong> 0.8.0-alpha</p>
          <p><strong>作者:</strong> jindy</p>
          <p><strong>协议:</strong> MIT License</p>
          <p className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
            管家 · 你的个人电脑管家<br />
            支持系统监控、文件管理、进程控制、定时任务等能力。
          </p>
        </div>
      </section>
    </div>
  );
}
