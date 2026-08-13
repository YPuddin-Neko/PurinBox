import { useState, useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { PanelLeftClose } from 'lucide-react';
import '../styles/sidebar.css';
import { getAppVersion, packageAppVersion } from '../utils/appVersion';
import { navSections, homePage, settingsPage } from '../appRegistry';
import { useAppSettings } from './ThemeProvider';

export default function Sidebar() {
  const { t } = useTranslation();
  const { workflowEnabled } = useAppSettings();
  const [collapsed, setCollapsed] = useState(false);
  const [appVersion, setAppVersion] = useState(packageAppVersion);
  // update status: 'checking' | 'latest' | 'update' | 'error'
  const [updateStatus, setUpdateStatus] = useState<'checking' | 'latest' | 'update' | 'error'>('latest');
  const [latestVersion, setLatestVersion] = useState('');
  const [releaseUrl, setReleaseUrl] = useState('');

  useEffect(() => { getAppVersion().then(setAppVersion); }, []);

  // Delay version check so startup isn't affected
  useEffect(() => {
    const timer = setTimeout(() => {
      invoke<{ has_update: boolean; latest_version: string; release_url: string }>('check_for_updates')
        .then(r => {
          setUpdateStatus(r.has_update ? 'update' : 'latest');
          setLatestVersion(r.latest_version);
          setReleaseUrl(r.release_url);
        })
        .catch(() => setUpdateStatus('latest'));
    }, 2000);
    return () => clearTimeout(timer);
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
          <NavLink to={homePage.path}
            className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
            end>
            <span className="sidebar-item-icon"><homePage.icon /></span>
            <span className="sidebar-item-label">{t(homePage.i18nKey)}</span>
          </NavLink>
        </div>
        {navSections
          .map((section) => ({
            ...section,
            // 实验性功能（测试版）由设置中的开关控制是否显示
            items: section.items.filter(item => !item.experimental || workflowEnabled),
          }))
          .filter((section) => section.items.length > 0)
          .map((section) => (
          <div key={section.titleKey} className="sidebar-section">
            <div className="sidebar-section-title">{t(section.titleKey)}</div>
            {section.items.map((item) => (
              <NavLink key={item.path} to={item.path}
                className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
                end={item.path === '/'}>
                <span className="sidebar-item-icon"><item.icon /></span>
                <span className="sidebar-item-label">
                  {t(item.i18nKey)}
                  {item.experimental && (
                    <span style={{
                      marginLeft: 6, padding: '0 4px', borderRadius: 3,
                      fontSize: 9, fontWeight: 700, letterSpacing: '0.02em',
                      color: '#fbbf24', border: '1px solid rgba(251, 191, 36, 0.45)',
                      verticalAlign: 'middle',
                    }}>
                      Beta
                    </span>
                  )}
                </span>
              </NavLink>
            ))}
          </div>
        ))}
      </nav>
      <div className="sidebar-toggle">
        <button className="sidebar-toggle-btn" onClick={() => setCollapsed(!collapsed)} title={collapsed ? t('sidebar.expandMenu') : t('sidebar.collapseMenu')}><PanelLeftClose /><span className="sidebar-item-label">{collapsed ? t('sidebar.expand') : t('sidebar.collapse')}</span></button>
      </div>
      <div className="sidebar-toggle" style={{borderTop: 'none', paddingTop: 0}}>
        <NavLink to={settingsPage.path}
          className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
          style={{margin: 0, width: '100%'}}>
          <span className="sidebar-item-icon"><settingsPage.icon /></span>
          <span className="sidebar-item-label">{t(settingsPage.i18nKey)}</span>
        </NavLink>
      </div>
      <div className="sidebar-version" title={dotTitle} onClick={handleVersionClick}
        style={{ cursor: updateStatus === 'update' || updateStatus === 'error' ? 'pointer' : 'default', background: 'none', border: 'none' }}>
        <div className="sidebar-version-dot" style={{ background: dotColor }} />
        <span>v{appVersion} · Release{updateStatus === 'update' ? ` → v${latestVersion}` : ''}</span>
      </div>
    </aside>
  );
}
