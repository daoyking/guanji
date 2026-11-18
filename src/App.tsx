import { useState } from 'react';
import Sidebar from './components/Sidebar';
import Dashboard from './pages/Dashboard';
import Files from './pages/Files';
import Processes from './pages/Processes';
import Apps from './pages/Apps';
import Models from './pages/Models';
import Tasks from './pages/Tasks';
import Services from './pages/Services';
import Clipboard from './pages/Clipboard';
import Memories from './pages/Memories';
import Notifications from './pages/Notifications';
import Settings from './pages/Settings';
import AgentChat from './pages/AgentChat';
import { useEffect } from 'react';

type Page = 'agent' | 'dashboard' | 'files' | 'processes' | 'apps' | 'models' | 'tasks' | 'services' | 'clipboard' | 'memories' | 'notifications' | 'settings';

export default function App() {
  const [currentPage, setCurrentPage] = useState<Page>('agent');
  const [isDark, setIsDark] = useState<boolean>(() => {
    // Read from localStorage (persisted by Settings) or system preference
    return localStorage.getItem('guanji-theme') === 'dark' || 
      (!localStorage.getItem('guanji-theme') && window.matchMedia('(prefers-color-scheme: dark)').matches);
  });

  // On mount: restore theme to <html> so no flash occurs
  useEffect(() => {
    document.documentElement.classList.toggle('dark', isDark);
  }, []);

  // Sync localStorage when user toggles
  useEffect(() => {
    localStorage.setItem('guanji-theme', isDark ? 'dark' : 'light');
    document.documentElement.classList.toggle('dark', isDark);
  }, [isDark]);

  const renderPage = () => {
    switch (currentPage) {
      case 'agent': return <AgentChat />;
      case 'dashboard': return <Dashboard />;
      case 'files': return <Files />;
      case 'processes': return <Processes />;
      case 'apps': return <Apps />;
      case 'models': return <Models />;
      case 'tasks': return <Tasks />;
      case 'services': return <Services />;
      case 'clipboard': return <Clipboard />;
      case 'memories': return <Memories />;
      case 'notifications': return <Notifications />;
      case 'settings': return <Settings isDark={isDark} onToggleTheme={setIsDark} />;
      default: return <Dashboard />;
    }
  };

  return (
    <div className={`flex h-screen bg-gray-50 dark:bg-gray-900 overflow-hidden transition-colors duration-300 ${isDark ? 'dark' : ''}`}>
      <Sidebar currentPage={currentPage} onPageChange={(page) => setCurrentPage(page as Page)} />
      <main className="flex-1 overflow-y-auto p-6">
        {renderPage()}
      </main>
    </div>
  );
}
