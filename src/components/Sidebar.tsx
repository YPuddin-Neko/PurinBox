import { useState, useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
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
  Scale,
} from 'lucide-react';
import '../styles/sidebar.css';
import { getVersion } from '@tauri-apps/api/app';

interface NavItem { id: string; labelKey: string; icon: React.ReactNode; path: string; }
interface NavSection { titleKey: string; items: NavItem[]; }

const homeItem: NavItem = { id: 'home', labelKey: 'sidebar.home', icon: <Home />, path: '/' };

const navSections: NavSection[] = [
  {
    titleKey: 'sidebar.sectionPreprocess',
    items: [
      { id: 'crop', labelKey: 'sidebar.crop', icon: <Crop />, path: '/crop' },
      { id: 'person-crop', labelKey: 'sidebar.personCrop', icon: <ScanFace />, path: '/person-crop' },
      { id: 'scale', labelKey: 'sidebar.scale', icon: <Scaling />, path: '/scale' },
      { id: 'flip', labelKey: 'sidebar.imageProcess', icon: <FlipHorizontal2 />, path: '/flip' },
      { id: 'filter', labelKey: 'sidebar.filter', icon: <ScanSearch />, path: '/filter' },
      { id: 'file-keeper', labelKey: 'sidebar.fileKeeper', icon: <FileCheck2 />, path: '/file-keeper' },
      { id: 'format-convert', labelKey: 'sidebar.formatConvert', icon: <FileType />, path: '/format-convert' },
      { id: 'alpha-convert', labelKey: 'sidebar.alphaConvert', icon: <Layers />, path: '/alpha-convert' },
      { id: 'batch-rename', labelKey: 'sidebar.batchRename', icon: <TextCursorInput />, path: '/batch-rename' },
      { id: 'perspective', labelKey: 'sidebar.perspective', icon: <Move3D />, path: '/perspective' },
      { id: 'blur-noise', labelKey: 'sidebar.blurNoise', icon: <Sparkles />, path: '/blur-noise' },
    ],
  },
  {
    titleKey: 'sidebar.sectionDataset',
    items: [
      { id: 'tagger', labelKey: 'sidebar.tagger', icon: <Tags />, path: '/tagger' },
      { id: 'tag-manager', labelKey: 'sidebar.tagManager', icon: <List />, path: '/tag-manager' },
    ],
  },
  {
    titleKey: 'sidebar.sectionAdvanced',
    items: [
      { id: 'tag-sort', labelKey: 'sidebar.tagSort', icon: <ArrowUpDown />, path: '/tag-sort' },
      { id: 'bucket-preview', labelKey: 'sidebar.bucketPreview', icon: <Grid3X3 />, path: '/bucket-preview' },
      { id: 'upscale', labelKey: 'sidebar.upscale', icon: <ZoomIn />, path: '/upscale' },
      { id: 'image-cluster', labelKey: 'sidebar.imageCluster', icon: <Network />, path: '/image-cluster' },
      { id: 'image-dedup', labelKey: 'sidebar.imageDedup', icon: <Copy />, path: '/image-dedup' },
      { id: 'dataset-balancer', labelKey: 'sidebar.datasetBalancer', icon: <Scale />, path: '/dataset-balancer' },
    ],
  },
  {
    titleKey: 'sidebar.sectionSystem',
    items: [{ id: 'settings', labelKey: 'sidebar.settings', icon: <Settings />, path: '/settings' }],
  },
];

export default function Sidebar() {
  const { t } = useTranslation();
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
  const dotTitle = updateStatus === 'latest' ? t('sidebar.latestVersion')
    : updateStatus === 'update' ? t('sidebar.newVersion', { version: latestVersion })
    : updateStatus === 'error' ? t('sidebar.checkFailed')
    : t('sidebar.checking');

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
            <span className="sidebar-item-label">{t(homeItem.labelKey)}</span>
          </NavLink>
        </div>
        {navSections.map((section) => (
          <div key={section.titleKey} className="sidebar-section">
            <div className="sidebar-section-title">{t(section.titleKey)}</div>
            {section.items.map((item) => (
              <NavLink key={item.id} to={item.path}
                className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
                end={item.path === '/'}>
                <span className="sidebar-item-icon">{item.icon}</span>
                <span className="sidebar-item-label">{t(item.labelKey)}</span>
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
        <button className="sidebar-toggle-btn" onClick={() => setCollapsed(!collapsed)} title={collapsed ? t('sidebar.expandMenu') : t('sidebar.collapseMenu')}><PanelLeftClose /><span className="sidebar-item-label">{collapsed ? t('sidebar.expand') : t('sidebar.collapse')}</span></button>
      </div>
    </aside>
  );
}
