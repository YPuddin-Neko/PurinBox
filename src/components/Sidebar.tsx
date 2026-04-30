import { useState } from 'react';
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
  Crop,
  ImagePlus,
  FolderSync,
  Settings,
  PanelLeftClose,
  Sparkles,
} from 'lucide-react';
import '../styles/sidebar.css';

interface NavItem { id: string; label: string; icon: React.ReactNode; path: string; }
interface NavSection { title: string; items: NavItem[]; }

const navSections: NavSection[] = [
  {
    title: '概览',
    items: [{ id: 'home', label: '工作台', icon: <Home />, path: '/' }],
  },
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
    ],
  },
  {
    title: '更多工具',
    items: [
      { id: 'crop', label: '图像裁切', icon: <Crop />, path: '/crop' },
      { id: 'augment', label: '数据增强', icon: <ImagePlus />, path: '/augment' },
      { id: 'organize', label: '数据集管理', icon: <FolderSync />, path: '/organize' },
    ],
  },
  {
    title: '系统',
    items: [{ id: 'settings', label: '设置', icon: <Settings />, path: '/settings' }],
  },
];

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <aside className={`sidebar ${collapsed ? 'collapsed' : ''}`}>
      <div className="sidebar-logo">
        <div className="sidebar-logo-icon"><Sparkles /></div>
        <div className="sidebar-logo-text"><h1>AI Train Tools</h1><span>AI 训练工具箱</span></div>
      </div>
      <nav className="sidebar-nav">
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
      <div className="sidebar-version"><div className="sidebar-version-dot" /><span>v0.1.0 · Preview</span></div>
      <div className="sidebar-toggle">
        <button className="sidebar-toggle-btn" onClick={() => setCollapsed(!collapsed)} title={collapsed ? '展开菜单' : '折叠菜单'}><PanelLeftClose /></button>
      </div>
    </aside>
  );
}
