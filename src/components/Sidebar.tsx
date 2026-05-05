import { useState, useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import {
  Home,
  Scaling,
  FlipHorizontal2,
  ScanSearch,
  FileCheck2,
  FileType,
  Layers,
  TextCursorInput,
  Tags,
  List,
  ArrowUpDown,
  Settings,
  PanelLeftClose,
} from 'lucide-react';
import '../styles/sidebar.css';
import { getVersion } from '@tauri-apps/api/app';

interface NavItem { id: string; label: string; icon: React.ReactNode; path: string; }
interface NavSection { title: string; items: NavItem[]; }

const homeItem: NavItem = { id: 'home', label: '首页', icon: <Home />, path: '/' };

const navSections: NavSection[] = [
  {
    title: '数据集预处理',
    items: [
      { id: 'scale', label: '图片缩放', icon: <Scaling />, path: '/scale' },
      { id: 'flip', label: '图片处理', icon: <FlipHorizontal2 />, path: '/flip' },
      { id: 'filter', label: '分辨率筛选', icon: <ScanSearch />, path: '/filter' },
      { id: 'file-keeper', label: '保留指定文件', icon: <FileCheck2 />, path: '/file-keeper' },
      { id: 'format-convert', label: '图片格式转换', icon: <FileType />, path: '/format-convert' },
      { id: 'alpha-convert', label: '转换透明通道', icon: <Layers />, path: '/alpha-convert' },
      { id: 'batch-rename', label: '批量重命名', icon: <TextCursorInput />, path: '/batch-rename' },
    ],
  },
  {
    title: '数据集处理',
    items: [
      { id: 'tagger', label: '图片打标', icon: <Tags />, path: '/tagger' },
      { id: 'tag-manager', label: '标签管理', icon: <List />, path: '/tag-manager' },
    ],
  },
  {
    title: '高级工具',
    items: [
      { id: 'tag-sort', label: '标签排序', icon: <ArrowUpDown />, path: '/tag-sort' },
    ],
  },
  {
    title: '系统',
    items: [{ id: 'settings', label: '设置', icon: <Settings />, path: '/settings' }],
  },
];

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false);
  const [appVersion, setAppVersion] = useState('');
  useEffect(() => { getVersion().then(v => setAppVersion(v)).catch(() => {}); }, []);
  return (
    <aside className={`sidebar ${collapsed ? 'collapsed' : ''}`}>
      <nav className="sidebar-nav">
        <div className="sidebar-section">
          <NavLink to={homeItem.path}
            className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
            end>
            <span className="sidebar-item-icon">{homeItem.icon}</span>
            <span className="sidebar-item-label">{homeItem.label}</span>
          </NavLink>
        </div>
        {navSections.map((section) => (
          <div key={section.title} className="sidebar-section">
            <div className="sidebar-section-title">{section.title}</div>
            {section.items.map((item) => (
              <NavLink key={item.id} to={item.path}
                className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
                end={item.path === '/'}>
                <span className="sidebar-item-icon">{item.icon}</span>
                <span className="sidebar-item-label">{item.label}</span>
              </NavLink>
            ))}
          </div>
        ))}
      </nav>
      <div className="sidebar-version"><div className="sidebar-version-dot" /><span>v{appVersion} · Preview</span></div>
      <div className="sidebar-toggle">
        <button className="sidebar-toggle-btn" onClick={() => setCollapsed(!collapsed)} title={collapsed ? '展开菜单' : '折叠菜单'}><PanelLeftClose /><span className="sidebar-item-label">{collapsed ? '展开' : '收起'}</span></button>
      </div>
    </aside>
  );
}
