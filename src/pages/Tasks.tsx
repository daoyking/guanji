import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Task {
  id: string;
  name: string;
  command: string;
  schedule: string;
  enabled: boolean;
  last_run?: string;
  next_run?: string;
}

export default function Tasks() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [newName, setNewName] = useState('');
  const [newCommand, setNewCommand] = useState('');
  const [newSchedule, setNewSchedule] = useState('daily');

  const loadTasks = async () => {
    setLoading(true);
    try {
      const result = await invoke<Task[]>('list_tasks');
      setTasks(result);
    } catch (e) {
      console.error('Failed to load tasks:', e);
    } finally {
      setLoading(false);
    }
  };

  const addTask = async () => {
    if (!newName.trim() || !newCommand.trim()) return;
    try {
      await invoke<string>('add_task', { name: newName, command: newCommand, schedule: newSchedule });
      setNewName('');
      setNewCommand('');
      setNewSchedule('daily');
      setShowAdd(false);
      await loadTasks();
    } catch (e) {
      alert(String(e));
    }
  };

  const deleteTask = async (id: string) => {
    if (!confirm('确定删除此任务？')) return;
    try {
      await invoke<number>('delete_task', { id });
      await loadTasks();
    } catch (e) {
      alert(String(e));
    }
  };

  const toggleTask = async (id: string) => {
    try {
      const newState = await invoke<boolean>('toggle_task', { id });
      setTasks(tasks.map(t => t.id === id ? { ...t, enabled: newState } : t));
    } catch (e) {
      alert(String(e));
    }
  };

  useEffect(() => {
    loadTasks();
  }, []);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-gray-900 dark:text-white">定时任务</h2>
        <button onClick={() => setShowAdd(true)} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700">
          + 新建任务
        </button>
      </div>

      {/* Add Task Dialog */}
      {showAdd && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-xl p-6 max-w-md w-full mx-4">
            <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-4">新建定时任务</h3>
            <div className="space-y-3">
              <input placeholder="任务名称" value={newName} onChange={e => setNewName(e.target.value)} className="w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700" />
              <input placeholder="执行命令（Shell）" value={newCommand} onChange={e => setNewCommand(e.target.value)} className="w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700" />
              <select value={newSchedule} onChange={e => setNewSchedule(e.target.value)} className="w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700">
                <option value="daily">每天</option>
                <option value="weekly">每周</option>
                <option value="hourly">每小时</option>
              </select>
            </div>
            <div className="flex gap-3 justify-end mt-4">
              <button onClick={() => setShowAdd(false)} className="px-4 py-2 text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">取消</button>
              <button onClick={addTask} className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700">创建</button>
            </div>
          </div>
        </div>
      )}

      {/* Task List */}
      {loading ? (
        <p className="text-gray-500 text-center py-8">加载中...</p>
      ) : tasks.length === 0 ? (
        <div className="bg-white dark:bg-gray-800 rounded-xl p-12 card-shadow text-center">
          <p className="text-gray-500 dark:text-gray-400">暂无定时任务</p>
          <p className="text-sm text-gray-400 dark:text-gray-500 mt-2">点击「新建任务」开始添加</p>
        </div>
      ) : (
        <div className="space-y-3">
          {tasks.map(task => (
            <div key={task.id} className="bg-white dark:bg-gray-800 rounded-xl p-4 card-shadow flex items-center justify-between">
              <div>
                <h3 className="font-medium text-gray-900 dark:text-white">{task.name}</h3>
                <p className="text-sm text-gray-500 dark:text-gray-400 font-mono">{task.command}</p>
                <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">周期: {task.schedule}</p>
              </div>
              <div className="flex items-center gap-3">
                <span className={`px-2 py-1 rounded-full text-xs ${task.enabled ? 'bg-green-100 text-green-700' : 'bg-gray-100 text-gray-500'}`}>
                  {task.enabled ? '运行中' : '已禁用'}
                </span>
                <button onClick={() => toggleTask(task.id)} className={`w-12 h-6 rounded-full transition-colors ${task.enabled ? 'bg-blue-600' : 'bg-gray-300'}`}>
                  <span className={`block w-5 h-5 bg-white rounded-full transform transition-transform ${task.enabled ? 'translate-x-6' : 'translate-x-1'}`} />
                </button>
                <button onClick={() => deleteTask(task.id)} className="text-red-500 hover:text-red-700 text-sm">删除</button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
