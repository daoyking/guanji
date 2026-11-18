interface SidebarProps {
  currentPage: string;
  onPageChange: (page: string) => void;
}

const navItems = [
  { id: 'agent', label: 'AI 管家', icon: '✨', featured: true },
  { id: 'dashboard', label: '概览', icon: '📊' },
  { id: 'files', label: '文件管家', icon: '📂' },
  { id: 'processes', label: '进程管家', icon: '⚙️' },
  { id: 'apps', label: '应用管家', icon: '📱' },
  { id: 'models', label: '模型管家', icon: '🤖' },
  { id: 'services', label: '服务管家', icon: '🔗' },
  { id: 'clipboard', label: '剪贴板管家', icon: '📋' },
  { id: 'memories', label: '记忆系统', icon: '🧠' },
  { id: 'notifications', label: '推送中心', icon: '📬' },
  { id: 'tasks', label: '定时任务', icon: '⏰' },
  { id: 'settings', label: '设置', icon: '🔧' },
];

export default function Sidebar({ currentPage, onPageChange }: SidebarProps) {
  return (
    <aside className="w-56 shrink-0 flex flex-col bg-white dark:bg-gray-800 border-r border-gray-200 dark:border-gray-700">
      {/* Logo */}
      <div className="px-6 py-4 border-b border-gray-200 dark:border-gray-700">
        <h1 className="text-xl font-bold text-gray-900 dark:text-white">管家</h1>
        <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">你的个人电脑管家</p>
      </div>

      {/* Navigation */}
      <nav className="flex-1 px-3 py-4 space-y-1">
        {navItems.map((item, idx) => (
          <div key={item.id}>
            {'featured' in item && item.featured && idx > 0 && (
              <div className="my-2 border-t border-gray-200 dark:border-gray-700" />
            )}
            <button
              onClick={() => onPageChange(item.id)}
              className={`flex w-full items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors ${
                currentPage === item.id
                  ? 'bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400'
                  : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-700'
              } ${'featured' in item && item.featured ? 'ring-1 ring-blue-200 dark:ring-blue-800' : ''}`}
            >
              <span className="text-lg">{item.icon}</span>
              {item.label}
              {'featured' in item && item.featured && (
                <span className="ml-auto text-[9px] px-1.5 py-0.5 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-600 dark:text-blue-400">NEW</span>
              )}
            </button>
          </div>
        ))}
      </nav>

      {/* Footer */}
      <div className="px-4 py-3 border-t border-gray-200 dark:border-gray-700">
        <p className="text-xs text-gray-400 dark:text-gray-500 text-center">v0.5.0 · α</p>
      </div>
    </aside>
  );
}
