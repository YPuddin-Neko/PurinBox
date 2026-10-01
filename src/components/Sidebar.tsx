import { useState, useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { PanelLeftClose } from 'lucide-react';
import '../styles/sidebar.css';
import { packageAppVersion, type UpdateCheckResult } from '../utils/appVersion';
import { navSections, homePage, settingsPage } from '../appRegistry';
import { useAppSettings } from './ThemeProvider';

export default function Sidebar() {
  const { t } = useTranslation();
  const { workflowEnabled } = useAppSettings();
  const [collapsed, setCollapsed] = useState(false);
  const appVersion = packageAppVersion;
  /** 仅在检查到新版本时有值；检查失败按"已是最新"显示 */
  const [update, setUpdate] = useState<UpdateCheckResult | null>(null);


  // Delay version check so startup isn't affected
  useEffect(() => {
    const timer = setTimeout(() => {
      invoke<UpdateCheckResult>('check_for_updates')
        .then(r => { if (r.has_update) setUpdate(r); })
        .catch(() => {});
    }, 2000);
    return () => clearTimeout(timer);
  }, []);

  const dotColor = update ? '#ef4444' : '#4ade80';
  const dotTitle = update ? t('sidebar.newVersion', { version: update.latest_version }) : t('sidebar.latestVersion');

  const handleVersionClick = () => {
    if (update?.release_url) window.open(update.release_url, '_blank');
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
                className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}>
                <span className="sidebar-item-icon"><item.icon /></span>
                <span className="sidebar-item-label">
                  {t(item.i18nKey)}
                  {item.experimental && (
                    <span className="beta-badge">
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
        style={{ cursor: update ? 'pointer' : 'default' }}>
        <div className="sidebar-version-dot" style={{ background: dotColor }} />
        <span>v{appVersion} · Release{update ? ` → v${update.latest_version}` : ''}</span>
      </div>
    </aside>
  );
}
