import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Crop,
  Scaling,
  BarChart3,
  Tags,
  List,
  Grid3X3,
  Copy,
  Scale,
  Settings,
} from 'lucide-react';
import { useAppSettings } from '../components/ThemeProvider';
import { useTaskList } from '../components/TaskContext';
import { PurinBrandIcon, DomainArtIcon } from '../components/PurinBrandIcon';
import useSystemStats, { getUsageColor } from '../hooks/useSystemStats';
import { packageAppVersion } from '../utils/appVersion';
import { navSections, TASK_ROUTE_MAP } from '../appRegistry';
import '../styles/home.css';

export default function HomePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { updateInfo } = useAppSettings();
  const tasks = useTaskList();
  const stats = useSystemStats();
  const [jelly, setJelly] = useState(false);
  const jellyTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (jellyTimerRef.current !== null) window.clearTimeout(jellyTimerRef.current);
    };
  }, []);

  const runningTasks = tasks.filter(task => task.status === 'running');
  const sectionCounts = Object.fromEntries(navSections.map(s => [s.key, s.items.length]));

  const triggerJelly = () => {
    if (jellyTimerRef.current !== null) window.clearTimeout(jellyTimerRef.current);
    setJelly(false);
    requestAnimationFrame(() => {
      setJelly(true);
      jellyTimerRef.current = window.setTimeout(() => {
        setJelly(false);
        jellyTimerRef.current = null;
      }, 520);
    });
  };

  const domains: {
    key: 'preprocess' | 'dataset' | 'advanced';
    title: string;
    badge: string;
    desc: string;
    path: string;
    color: string;
    bg: string;
  }[] = [
    {
      key: 'preprocess',
      title: t('sidebar.sectionPreprocess'),
      badge: t('home.itemsCount', { count: sectionCounts.preprocess ?? 13 }),
      desc: t('home.preprocessDesc'),
      path: '/crop',
      color: 'var(--dom-pre)',
      bg: 'var(--dom-pre-bg)',
    },
    {
      key: 'dataset',
      title: t('sidebar.sectionDataset'),
      badge: t('home.itemsCount', { count: sectionCounts.dataset ?? 2 }),
      desc: t('home.datasetDesc'),
      path: '/tagger',
      color: 'var(--dom-dat)',
      bg: 'var(--dom-dat-bg)',
    },
    {
      key: 'advanced',
      title: t('sidebar.sectionAdvanced'),
      badge: t('home.itemsCount', { count: sectionCounts.advanced ?? 7 }),
      desc: t('home.advancedDesc'),
      path: '/bucket-preview',
      color: 'var(--dom-adv)',
      bg: 'var(--dom-adv-bg)',
    },
  ];

  const quickTools = [
    { path: '/crop', label: t('sidebar.crop'), section: t('sidebar.sectionPreprocess'), Icon: Crop, color: 'var(--dom-pre)', bg: 'var(--dom-pre-bg)' },
    { path: '/scale', label: t('sidebar.scale'), section: t('sidebar.sectionPreprocess'), Icon: Scaling, color: 'var(--dom-pre)', bg: 'var(--dom-pre-bg)' },
    { path: '/resolution-analyze', label: t('sidebar.resolutionAnalyze'), section: t('sidebar.sectionPreprocess'), Icon: BarChart3, color: 'var(--dom-pre)', bg: 'var(--dom-pre-bg)' },
    { path: '/bucket-preview', label: t('sidebar.bucketPreview'), section: t('sidebar.sectionAdvanced'), Icon: Grid3X3, color: 'var(--dom-adv)', bg: 'var(--dom-adv-bg)' },
    { path: '/image-dedup', label: t('sidebar.imageDedup'), section: t('sidebar.sectionAdvanced'), Icon: Copy, color: 'var(--dom-adv)', bg: 'var(--dom-adv-bg)' },
    { path: '/dataset-balancer', label: t('sidebar.datasetBalancer'), section: t('sidebar.sectionAdvanced'), Icon: Scale, color: 'var(--dom-adv)', bg: 'var(--dom-adv-bg)' },
  ];

  return (
    <div className="page">
      <div className="home-wrap">
        {/* Hero */}
        <section className="home-hero">
          <div className="home-hero-left">
            <button
              type="button"
              className={`home-logo-btn ${jelly ? 'is-jelly' : ''}`}
              onClick={triggerJelly}
              title="PurinBox"
              aria-label="PurinBox"
            >
              <PurinBrandIcon size={62} />
            </button>
            <div className="home-hero-meta">
              <div className="home-title-row">
                <h1 className="home-title">PurinBox</h1>
                <span className="home-version-pill">v{packageAppVersion}</span>
                {updateInfo?.has_update && (
                  <button
                    type="button"
                    className="home-update-pill"
                    title={t('sidebar.newVersion', { version: updateInfo.latest_version })}
                    onClick={() => {
                      if (updateInfo.release_url) window.open(updateInfo.release_url, '_blank');
                      else navigate('/settings');
                    }}
                  >
                    <span className="home-update-dot" />
                    <span>{t('home.updatePill', { version: updateInfo.latest_version })} ↗</span>
                  </button>
                )}
              </div>
              <p className="home-tagline">{t('home.tagline')}</p>
            </div>
          </div>

          <div className="home-hero-actions">
            <button type="button" className="btn btn-primary" onClick={() => navigate('/tagger')}>
              <Tags />
              {t('sidebar.tagger')}
            </button>
            <button type="button" className="btn btn-secondary" onClick={() => navigate('/tag-manager')}>
              <List />
              {t('sidebar.tagManager')}
            </button>
          </div>
        </section>

        {/* 3 Core Feature Domains with Bespoke Ceramic Icons */}
        <section className="home-domains">
          {domains.map((dom, idx) => (
            <button
              key={dom.key}
              type="button"
              className="home-domain-card"
              style={{ '--d': `${45 + idx * 45}ms` } as CSSProperties}
              onClick={() => navigate(dom.path)}
            >
              <div className="home-domain-top">
                <DomainArtIcon domain={dom.key} />
                <span className="home-domain-count" style={{ background: dom.bg, color: dom.color }}>
                  {dom.badge}
                </span>
              </div>
              <div>
                <div className="home-domain-title">{dom.title}</div>
                <div className="home-domain-desc">{dom.desc}</div>
              </div>
            </button>
          ))}
        </section>

        {/* Quick Tools + Live Task & System Status */}
        <section className="home-lower">
          <div className="home-card" style={{ '--d': '180ms' } as CSSProperties}>
            <div className="home-card-head">
              <span className="home-card-title">{t('home.quickTools')}</span>
              <button type="button" className="home-sys-link" onClick={() => navigate('/settings')}>
                {t('sidebar.settings')}
              </button>
            </div>
            <div className="home-quick-grid">
              {quickTools.map((tool) => (
                <button
                  key={tool.path}
                  type="button"
                  className="home-quick-item"
                  onClick={() => navigate(tool.path)}
                >
                  <span className="home-quick-icon" style={{ background: tool.bg, color: tool.color }}>
                    <tool.Icon />
                  </span>
                  <div style={{ minWidth: 0 }}>
                    <div className="home-quick-label">{tool.label}</div>
                    <div className="home-quick-section">{tool.section}</div>
                  </div>
                </button>
              ))}
            </div>
          </div>

          <div className="home-card" style={{ '--d': '220ms' } as CSSProperties}>
            <div className="home-card-head">
              <span className="home-card-title">{t('home.taskOverview')}</span>
              <span
                className="home-card-meta"
                style={{ color: runningTasks.length > 0 ? 'var(--color-success)' : 'var(--color-text-tertiary)' }}
              >
                {runningTasks.length > 0
                  ? t('home.runningSummary', { count: runningTasks.length })
                  : t('home.idleSummary')}
              </span>
            </div>

            {tasks.length > 0 ? (
              <div className="home-task-list">
                {tasks.slice(0, 3).map((task) => {
                  const pct = task.total > 0 ? Math.min(100, Math.round((task.current / task.total) * 100)) : 0;
                  const route = TASK_ROUTE_MAP[task.id];
                  return (
                    <button
                      key={task.id}
                      type="button"
                      className="home-task-row"
                      onClick={() => route && navigate(route)}
                    >
                      <div className="home-task-top">
                        <span className="home-task-name">{task.name}</span>
                        <span style={{ fontFamily: 'var(--font-mono)', fontWeight: 700, color: 'var(--color-accent-primary)' }}>
                          {task.status === 'running' && task.total > 0 ? `${pct}%` : task.status}
                        </span>
                      </div>
                      {task.status === 'running' && task.total > 0 && (
                        <div className="home-task-bar">
                          <div className="home-task-fill" style={{ width: `${pct}%` }} />
                        </div>
                      )}
                      {task.message && (
                        <span style={{ fontSize: 10.5, color: 'var(--color-text-tertiary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                          {task.message}
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="home-empty-hint">{t('home.noTasksHint')}</div>
            )}

            <div className="home-sys-strip">
              {stats ? (
                <div className="home-sys-metrics">
                  <span>CPU <strong style={{ color: getUsageColor(stats.cpu_usage) }}>{stats.cpu_usage.toFixed(0)}%</strong></span>
                  <span>RAM <strong style={{ color: getUsageColor(stats.memory_percent) }}>{stats.memory_percent.toFixed(0)}%</strong></span>
                  {stats.gpu_usage >= 0 && (
                    <span>GPU <strong style={{ color: getUsageColor(stats.gpu_usage) }}>{stats.gpu_usage.toFixed(0)}%</strong></span>
                  )}
                </div>
              ) : (
                <span>{t('home.monitorOffHint')}</span>
              )}
              <button
                type="button"
                className="home-sys-link"
                style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}
                onClick={() => navigate('/settings')}
              >
                <Settings size={12} />
                {t('home.openSettings')}
              </button>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
