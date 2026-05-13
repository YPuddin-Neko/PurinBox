import { useState, useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import {
  Home,
  Crop,
  ScanFace,
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
  Grid3X3,
  Settings,
  PanelLeftClose,
  Move3D,
  Sparkles,
  ZoomIn,
  Network,
  Copy,
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
      { id: 'crop', label: '图片裁切', icon: <Crop />, path: '/crop' },
      { id: 'person-crop', label: '三分法裁切', icon: <ScanFace />, path: '/person-crop' },
      { id: 'scale', label: '图片缩放', icon: <Scaling />, path: '/scale' },
      { id: 'flip', label: '图片处理', icon: <FlipHorizontal2 />, path: '/flip' },
      { id: 'filter', label: '分辨率筛选', icon: <ScanSearch />, path: '/filter' },
      { id: 'file-keeper', label: '保留指定文件', icon: <FileCheck2 />, path: '/file-keeper' },
      { id: 'format-convert', label: '图片格式转换', icon: <FileType />, path: '/format-convert' },
      { id: 'alpha-convert', label: '转换透明通道', icon: <Layers />, path: '/alpha-convert' },
      { id: 'batch-rename', label: '批量重命名', icon: <TextCursorInput />, path: '/batch-rename' },
      { id: 'perspective', label: '透视变换', icon: <Move3D />, path: '/perspective' },
      { id: 'blur-noise', label: '模糊/噪点', icon: <Sparkles />, path: '/blur-noise' },
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
      { id: 'bucket-preview', label: '分桶预览', icon: <Grid3X3 />, path: '/bucket-preview' },
      { id: 'upscale', label: '图片超分', icon: <ZoomIn />, path: '/upscale' },
      { id: 'image-cluster', label: '图片聚类', icon: <Network />, path: '/image-cluster' },
      { id: 'image-dedup', label: '图片去重', icon: <Copy />, path: '/image-dedup' },
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
  // update status: 'checking' | 'latest' | 'update' | 'error'
  const [updateStatus, setUpdateStatus] = useState<'checking' | 'latest' | 'update' | 'error'>('checking');
  const [latestVersion, setLatestVersion] = useState('');
  const [releaseUrl, setReleaseUrl] = useState('');

  useEffect(() => { getVersion().then(v => setAppVersion(v)).catch(() => {}); }, []);

  useEffect(() => {
    invoke<{ has_update: boolean; latest_version: string; release_url: string }>('check_for_updates')
      .then(r => {
        setUpdateStatus(r.has_update ? 'update' : 'latest');
        setLatestVersion(r.latest_version);
        setReleaseUrl(r.release_url);
      })
      .catch(() => setUpdateStatus('error'));
  }, []);

  const dotColor = updateStatus === 'latest' ? '#4ade80' : updateStatus === 'update' ? '#ef4444' : updateStatus === 'error' ? '#fbbf24' : 'var(--color-text-tertiary)';
  const dotTitle = updateStatus === 'latest' ? '已是最新版本'
    : updateStatus === 'update' ? `发现新版本 v${latestVersion}，点击前往下载`
    : updateStatus === 'error' ? '更新检查失败，点击重试'
    : '正在检查更新...';

  const handleVersionClick = () => {
    if (updateStatus === 'update' && releaseUrl) {
      window.open(releaseUrl, '_blank');
    } else if (updateStatus === 'error') {
      setUpdateStatus('checking');
      invoke<{ has_update: boolean; latest_version: string; release_url: string }>('check_for_updates')
        .then(r => {
          setUpdateStatus(r.has_update ? 'update' : 'latest');
          setLatestVersion(r.latest_version);
          setReleaseUrl(r.release_url);
        })
        .catch(() => setUpdateStatus('error'));
    }
  };
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
      <div className="sidebar-version" title={dotTitle} onClick={handleVersionClick}
        style={{ cursor: updateStatus === 'update' || updateStatus === 'error' ? 'pointer' : 'default' }}>
        <div className="sidebar-version-dot" style={{ background: dotColor }} />
        <span>v{appVersion} · Release{updateStatus === 'update' ? ` → v${latestVersion}` : ''}</span>
      </div>
      <div className="sidebar-toggle">
        <button className="sidebar-toggle-btn" onClick={() => setCollapsed(!collapsed)} title={collapsed ? '展开菜单' : '折叠菜单'}><PanelLeftClose /><span className="sidebar-item-label">{collapsed ? '展开' : '收起'}</span></button>
      </div>
    </aside>
  );
}
