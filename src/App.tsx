import { BrowserRouter, Routes, Route, useLocation } from 'react-router-dom';
import { useEffect, useRef, type ComponentType } from 'react';
import { ThemeProvider } from './components/ThemeProvider';
import Sidebar from './components/Sidebar';
import Header from './components/Header';
import HomePage from './pages/HomePage';
import ScalePage from './pages/ScalePage';
import CropPage from './pages/CropPage';
import PersonCropPage from './pages/PersonCropPage';
import FlipPage from './pages/FlipPage';
import FilterPage from './pages/FilterPage';
import ResolutionAnalyzePage from './pages/ResolutionAnalyzePage';
import FileKeeperPage from './pages/FileKeeperPage';
import FormatConvertPage from './pages/FormatConvertPage';
import AlphaConvertPage from './pages/AlphaConvertPage';
import BatchRenamePage from './pages/BatchRenamePage';
import TaggerPage from './pages/TaggerPage';
import TagManagerPage from './pages/TagManagerPage';
import TagSortPage from './pages/TagSortPage';
import BucketPreviewPage from './pages/BucketPreviewPage';
import PerspectivePage from './pages/PerspectivePage';
import BlurNoisePage from './pages/BlurNoisePage';
import UpscalePage from './pages/UpscalePage';
import ImageClusterPage from './pages/ImageClusterPage';
import ImageDedupPage from './pages/ImageDedupPage';
import DatasetBalancerPage from './pages/DatasetBalancerPage';
import SdMetadataPage from './pages/SdMetadataPage';
import AestheticPage from './pages/AestheticPage';
import SettingsPage from './pages/SettingsPage';
import './assets/fonts/inter.css';
import './styles/global.css';
import { TaskProvider } from './components/TaskContext';
import { PAGES, persistentPages, routePages } from './appRegistry';

// 路由路径 → 页面组件。页面清单本身在 appRegistry.ts（单一事实来源），
// 这里只做组件绑定；注册表里有路径但此处缺组件会在启动时立刻报错。
const PAGE_COMPONENTS: Record<string, ComponentType> = {
  '/': HomePage,
  '/aesthetic': AestheticPage,
  '/crop': CropPage,
  '/person-crop': PersonCropPage,
  '/scale': ScalePage,
  '/flip': FlipPage,
  '/filter': FilterPage,
  '/resolution-analyze': ResolutionAnalyzePage,
  '/file-keeper': FileKeeperPage,
  '/format-convert': FormatConvertPage,
  '/alpha-convert': AlphaConvertPage,
  '/batch-rename': BatchRenamePage,
  '/perspective': PerspectivePage,
  '/blur-noise': BlurNoisePage,
  '/tagger': TaggerPage,
  '/tag-manager': TagManagerPage,
  '/tag-sort': TagSortPage,
  '/bucket-preview': BucketPreviewPage,
  '/upscale': UpscalePage,
  '/image-cluster': ImageClusterPage,
  '/image-dedup': ImageDedupPage,
  '/dataset-balancer': DatasetBalancerPage,
  '/sd-metadata': SdMetadataPage,
  '/settings': SettingsPage,
};

for (const page of PAGES) {
  if (!PAGE_COMPONENTS[page.path]) {
    throw new Error(`appRegistry 中的页面 ${page.path} 缺少组件映射，请在 App.tsx 的 PAGE_COMPONENTS 中补充`);
  }
}

function AppContent() {
  const location = useLocation();
  const currentPath = location.pathname;
  // 懒加载：仅在首次访问时挂载页面，之后保持 mounted（display 切换，切页不丢状态）
  const visitedRef = useRef<Set<string>>(new Set());
  const prevPathRef = useRef(currentPath);
  const directionRef = useRef<'down' | 'up'>('down');
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (mainRef.current) mainRef.current.scrollTop = 0;
  }, [currentPath]);

  if (prevPathRef.current !== currentPath) {
    const prevIdx = PAGES.findIndex(p => p.path === prevPathRef.current);
    const nextIdx = PAGES.findIndex(p => p.path === currentPath);
    directionRef.current = nextIdx >= prevIdx ? 'down' : 'up';
    prevPathRef.current = currentPath;
  }

  const isPersistentRoute = persistentPages.some(p => p.path === currentPath);
  if (isPersistentRoute) {
    visitedRef.current.add(currentPath);
  }

  const enterClass = directionRef.current === 'down' ? 'page-enter-down' : 'page-enter-up';

  return (
    <div className="main-layout">
      <Header />
      <main ref={mainRef} className="main-content">
        {/* 持久化页面 - 首次访问时挂载，之后通过 display 控制显示，切回时触发方向感微滑入 */}
        {persistentPages.map(({ path }) => {
          if (!visitedRef.current.has(path)) return null;
          const Component = PAGE_COMPONENTS[path];
          const isActive = currentPath === path;
          return (
            <div
              key={path}
              className={isActive ? `page-slot ${enterClass}` : 'page-slot'}
              style={{ display: isActive ? 'block' : 'none', height: '100%' }}
            >
              <Component />
            </div>
          );
        })}

        {/* 非持久化页面 - 正常路由 */}
        {!isPersistentRoute && (
          <div key={currentPath} className={`page-slot ${enterClass}`}>
            <Routes>
              {routePages.map(({ path }) => {
                const Component = PAGE_COMPONENTS[path];
                return <Route key={path} path={path} element={<Component />} />;
              })}
            </Routes>
          </div>
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
