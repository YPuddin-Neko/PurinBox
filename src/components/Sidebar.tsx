import { useState, useEffect, useLayoutEffect, useRef, type CSSProperties, type MouseEvent } from 'react';
import { NavLink, useLocation, useNavigate } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { PanelLeftClose, Search, ChevronDown, X } from 'lucide-react';
import '../styles/sidebar.css';
import { packageAppVersion, type UpdateCheckResult } from '../utils/appVersion';
import { openExternalUrl } from '../utils/tauriRuntime';
import { navSections, homePage, settingsPage, type PageDef } from '../appRegistry';
import { useAppSettings } from './ThemeProvider';
import { PurinBrandIcon } from './PurinBrandIcon';

const DOMAIN_VARS: Record<string, { color: string; bg: string }> = {
  preprocess: { color: 'var(--dom-pre)', bg: 'var(--dom-pre-bg)' },
  dataset: { color: 'var(--dom-dat)', bg: 'var(--dom-dat-bg)' },
  advanced: { color: 'var(--dom-adv)', bg: 'var(--dom-adv-bg)' },
  home: { color: 'var(--color-accent-primary)', bg: 'var(--color-accent-soft)' },
};

function SidebarLink({ page, end, domainKey, collapsed }: {
  page: PageDef;
  end?: boolean;
  domainKey: string;
  collapsed: boolean;
}) {
  const { t } = useTranslation();
  const label = t(page.i18nKey);
  const dom = DOMAIN_VARS[domainKey] || DOMAIN_VARS.home;
  return (
    <NavLink
      to={page.path}
      end={end}
      title={collapsed ? label : undefined}
      data-domain={domainKey}
      style={{ '--item-accent': dom.color } as CSSProperties}
      className={({ isActive }) => `sidebar-item ${isActive ? 'active' : ''}`}
    >
      <span className="sidebar-item-icon"><page.icon /></span>
      <span className="sidebar-item-label">
        <span className="sidebar-item-text">{label}</span>
        {page.experimental && <span className="beta-badge">Beta</span>}
      </span>
    </NavLink>
  );
}

export default function Sidebar() {
  const { t } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const { updateInfo, setUpdateInfo } = useAppSettings();

  const [collapsed, setCollapsed] = useState(false);
  const [search, setSearch] = useState('');
  const [folded, setFolded] = useState<Record<string, boolean>>({});
  const [jelly, setJelly] = useState(false);

  const navRef = useRef<HTMLElement>(null);
  const pillRef = useRef<HTMLDivElement>(null);
  const pillMountedRef = useRef(false);

  const appVersion = packageAppVersion;

  // 推迟检查更新，不占用启动时间；结果存入共享 context 供侧栏/首页/设置同步展示
  useEffect(() => {
    const timer = setTimeout(() => {
      if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
        invoke<UpdateCheckResult>('check_for_updates')
          .then(r => { if (r.has_update) setUpdateInfo(r); })
          .catch(() => {});
      }
    }, 2000);
    return () => clearTimeout(timer);
  }, [setUpdateInfo]);

  const query = search.trim().toLowerCase();

  const visibleSections = navSections
    .map(section => ({
      ...section,
      items: section.items
        .filter(item => !query || t(item.i18nKey).toLowerCase().includes(query)),
    }))
    .filter(section => section.items.length > 0);

  // FLIP 弹簧滑移高亮条：跟随当前激活导航项平滑移动并切换域色
  useLayoutEffect(() => {
    const syncPill = (instant = false) => {
      const nav = navRef.current;
      const pill = pillRef.current;
      if (!nav || !pill) return;
      const activeEl = nav.querySelector<HTMLElement>('.sidebar-item.active');
      const hiddenInFold = activeEl?.closest('.sidebar-section-acc.is-folded');
      if (!activeEl || hiddenInFold) {
        pill.style.opacity = '0';
        return;
      }
      const nr = nav.getBoundingClientRect();
      const er = activeEl.getBoundingClientRect();
      const x = er.left - nr.left + nav.scrollLeft;
      const y = er.top - nr.top + nav.scrollTop;
      const domKey = activeEl.dataset.domain || 'home';
      const dom = DOMAIN_VARS[domKey] || DOMAIN_VARS.home;

      if (instant) pill.style.transition = 'none';
      pill.style.setProperty('--pill-bg', dom.bg);
      pill.style.setProperty('--pill-accent', dom.color);
      pill.style.width = `${er.width}px`;
      pill.style.height = `${er.height}px`;
      pill.style.transform = `translate3d(${x}px, ${y}px, 0)`;
      pill.style.opacity = '1';
      if (instant) {
        pill.getBoundingClientRect();
        pill.style.transition = '';
      }
    };

    syncPill(!pillMountedRef.current);
    pillMountedRef.current = true;

    // 侧栏宽度或分组折叠过渡结束后再精准校准一次
    const settleTimer = setTimeout(() => syncPill(false), 300);
    return () => clearTimeout(settleTimer);
  }, [location.pathname, collapsed, query, folded]);

  const hasUpdate = Boolean(updateInfo?.has_update);

  const handleVersionClick = (e: MouseEvent) => {
    e.stopPropagation();
    if (updateInfo?.release_url) {
      void openExternalUrl(updateInfo.release_url);
    } else {
      navigate('/settings');
    }
  };

  const triggerJelly = () => {
    setJelly(true);
    setTimeout(() => setJelly(false), 500);
    if (location.pathname !== '/') navigate('/');
  };

  return (
    <aside className={`sidebar ${collapsed ? 'collapsed' : ''}`}>
      {/* 品牌与工具筛选 */}
      <div className="sidebar-header">
        <div className="sidebar-brand-row">
          <div
            className="sidebar-brand"
            onClick={triggerJelly}
            title={
              collapsed && hasUpdate && updateInfo
                ? `PurinBox · ${t('sidebar.newVersion', { version: updateInfo.latest_version })}`
                : 'PurinBox'
            }
          >
            <span className="sidebar-brand-icon-wrap">
              <PurinBrandIcon
                size={30}
                className={`sidebar-brand-icon ${jelly ? 'is-jelly' : ''}`}
              />
              {collapsed && hasUpdate && <span className="sidebar-brand-update-dot" />}
            </span>
            {!collapsed && (
              <div className="sidebar-brand-meta">
                <span className="sidebar-brand-name">PurinBox</span>
                {hasUpdate && updateInfo ? (
                  <button
                    type="button"
                    className="sidebar-brand-ver has-update"
                    title={t('sidebar.newVersion', { version: updateInfo.latest_version })}
                    onClick={handleVersionClick}
                  >
                    <span className="sidebar-version-dot" />
                    <span className="sidebar-brand-ver-text">
                      v{appVersion} → {updateInfo.latest_version} ↗
                    </span>
                  </button>
                ) : (
                  <span className="sidebar-brand-ver">v{appVersion}</span>
                )}
              </div>
            )}
          </div>
          {!collapsed && (
            <button
              type="button"
              className="sidebar-collapse-icon-btn"
              onClick={() => setCollapsed(true)}
              title={t('sidebar.collapseMenu')}
            >
              <PanelLeftClose />
            </button>
          )}
        </div>

        {!collapsed && (
          <div className="sidebar-search-wrap">
            <Search className="sidebar-search-icon" />
            <input
              type="text"
              className="sidebar-search-input"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t('sidebar.filterPlaceholder')}
            />
            {search && (
              <button
                type="button"
                className="sidebar-search-clear"
                onClick={() => setSearch('')}
                aria-label={t('common.close')}
              >
                <X size={11} />
              </button>
            )}
          </div>
        )}
      </div>

      {/* 主导航 */}
      <nav ref={navRef} className="sidebar-nav">
        <div ref={pillRef} className="sidebar-active-pill" aria-hidden="true" />

        <div className="sidebar-section sidebar-section-home">
          <SidebarLink page={homePage} end domainKey="home" collapsed={collapsed} />
        </div>

        {visibleSections.map((section) => {
          const dom = DOMAIN_VARS[section.key] || DOMAIN_VARS.preprocess;
          const isFolded = !collapsed && !query && Boolean(folded[section.key]);
          return (
            <div key={section.titleKey} className="sidebar-section">
              {collapsed ? (
                <div className="sidebar-rail-divider" />
              ) : (
                <button
                  type="button"
                  className="sidebar-section-head"
                  onClick={() => setFolded(prev => ({ ...prev, [section.key]: !prev[section.key] }))}
                  aria-expanded={!isFolded}
                >
                  <span className="sidebar-section-title-left">
                    <span className="sidebar-section-dot" style={{ background: dom.color }} />
                    <span>{t(section.titleKey)} ({section.items.length})</span>
                  </span>
                  <ChevronDown className={`sidebar-section-chev ${isFolded ? 'is-folded' : ''}`} />
                </button>
              )}

              <div className={`sidebar-section-acc ${isFolded ? 'is-folded' : ''}`}>
                <div className="sidebar-section-acc-inner">
                  {section.items.map((item) => (
                    <SidebarLink
                      key={item.path}
                      page={item}
                      domainKey={section.key}
                      collapsed={collapsed}
                    />
                  ))}
                </div>
              </div>
            </div>
          );
        })}
      </nav>

      {/* 底部：展开按钮 + 设置 */}
      <div className="sidebar-footer">
        {collapsed && (
          <button
            type="button"
            className="sidebar-expand-btn"
            onClick={() => setCollapsed(false)}
            title={t('sidebar.expandMenu')}
          >
            <PanelLeftClose />
          </button>
        )}

        <SidebarLink page={settingsPage} domainKey="home" collapsed={collapsed} />
      </div>
    </aside>
  );
}
