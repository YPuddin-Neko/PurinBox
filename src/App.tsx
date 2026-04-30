import { BrowserRouter, Routes, Route, useLocation } from 'react-router-dom';
import { ThemeProvider } from './components/ThemeProvider';
import Sidebar from './components/Sidebar';
import Header from './components/Header';
import HomePage from './pages/HomePage';
import ScalePage from './pages/ScalePage';
import FlipPage from './pages/FlipPage';
import FilterPage from './pages/FilterPage';
import FileKeeperPage from './pages/FileKeeperPage';
import FormatConvertPage from './pages/FormatConvertPage';
import AlphaConvertPage from './pages/AlphaConvertPage';
import BatchRenamePage from './pages/BatchRenamePage';
import TaggerPage from './pages/TaggerPage';
import LabelingPage from './pages/LabelingPage';
import CropPage from './pages/CropPage';
import ConvertPage from './pages/ConvertPage';
import AugmentPage from './pages/AugmentPage';
import OrganizePage from './pages/OrganizePage';
import SettingsPage from './pages/SettingsPage';
import './styles/global.css';
import './styles/sidebar.css';
import './styles/layout.css';
import './styles/progress.css';
import { TaskProvider } from './components/TaskContext';

// 需要保持状态的页面（重型页面）用 display:none 隐藏而非卸载
const persistentPages = [
  { path: '/tagger', component: TaggerPage },
];

// 普通页面走 Routes
const routePages = [
  { path: '/', component: HomePage },
  { path: '/scale', component: ScalePage },
  { path: '/flip', component: FlipPage },
  { path: '/filter', component: FilterPage },
  { path: '/file-keeper', component: FileKeeperPage },
  { path: '/format-convert', component: FormatConvertPage },
  { path: '/alpha-convert', component: AlphaConvertPage },
  { path: '/batch-rename', component: BatchRenamePage },
  { path: '/labeling', component: LabelingPage },
  { path: '/crop', component: CropPage },
  { path: '/convert', component: ConvertPage },
  { path: '/augment', component: AugmentPage },
  { path: '/organize', component: OrganizePage },
  { path: '/settings', component: SettingsPage },
];

function AppContent() {
  const location = useLocation();
  const currentPath = location.pathname;

  return (
    <div className="main-layout">
      <Header />
      <main className="main-content">
        {/* 持久化页面 - 始终挂载，通过 display 控制显示 */}
        {persistentPages.map(({ path, component: Component }) => (
          <div key={path} style={{ display: currentPath === path ? 'block' : 'none', height: '100%' }}>
            <Component />
          </div>
        ))}

        {/* 非持久化页面 - 正常路由 */}
        {!persistentPages.some(p => p.path === currentPath) && (
          <Routes>
            {routePages.map(({ path, component: Component }) => (
              <Route key={path} path={path} element={<Component />} />
            ))}
          </Routes>
        )}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <TaskProvider>
        <BrowserRouter>
          <Sidebar />
          <AppContent />
        </BrowserRouter>
      </TaskProvider>
    </ThemeProvider>
  );
}
