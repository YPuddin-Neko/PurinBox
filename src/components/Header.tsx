import { useLocation, useNavigate } from 'react-router-dom';
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { Sun, Moon, Monitor, ListTodo, Cpu, MemoryStick, MonitorDot, Trash2 } from 'lucide-react';
import { useTheme } from './ThemeProvider';
import { useTaskQueue } from './TaskContext';
import { changeLanguage, availableLanguages } from '../i18n';
import '../styles/layout.css';

// 路由路径 → 侧边栏翻译 key 映射（复用 sidebar 的翻译 key）
const routeI18nMap: Record<string, string> = {
  '/': 'sidebar.home',
  '/scale': 'sidebar.scale',
  '/flip': 'sidebar.imageProcess',
  '/filter': 'sidebar.filter',
  '/file-keeper': 'sidebar.fileKeeper',
  '/format-convert': 'sidebar.formatConvert',
  '/alpha-convert': 'sidebar.alphaConvert',
  '/batch-rename': 'sidebar.batchRename',
  '/crop': 'sidebar.crop',
  '/person-crop': 'sidebar.personCrop',
  '/perspective': 'sidebar.perspective',
  '/blur-noise': 'sidebar.blurNoise',
  '/tagger': 'sidebar.tagger',
  '/tag-manager': 'sidebar.tagManager',
  '/tag-sort': 'sidebar.tagSort',
  '/bucket-preview': 'sidebar.bucketPreview',
  '/upscale': 'sidebar.upscale',
  '/image-cluster': 'sidebar.imageCluster',
  '/image-dedup': 'sidebar.imageDedup',
  '/dataset-balancer': 'sidebar.datasetBalancer',
  '/settings': 'sidebar.settings',
};

// 任务 ID → 路由路径映射
const TASK_ROUTE_MAP: Record<string, string> = {
  scale: '/scale',
  flip: '/flip',
  filter: '/filter',
  keeper: '/file-keeper',
  convert: '/format-convert',
  alpha: '/alpha-convert',
  rename: '/batch-rename',
  crop: '/crop',
  'person-crop': '/person-crop',
  perspective: '/perspective',
  'blur-noise': '/blur-noise',
  tagger: '/tagger',
  'llm-tagger': '/tagger',
  'tag-sort': '/tag-sort',
  upscale: '/upscale',
};

interface SystemStats {
  cpu_usage: number;
  cpu_name: string;
  cpu_cores: number;
  memory_used: number;
  memory_total: number;
  memory_percent: number;
  gpu_name: string;
  gpu_usage: number;
  vram_used: number;
  vram_total: number;
  vram_percent: number;
}

function MiniBar({ value, color, max = 100 }: { value: number; color: string; max?: number }) {
  const pct = Math.min(Math.max(value / max * 100, 0), 100);
  return (
    <div style={{ width: 36, height: 4, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}>
      <div style={{ width: `${pct}%`, height: '100%', borderRadius: 2, background: color, transition: 'width 0.6s ease' }} />
    </div>
  );
}

function formatBytes(bytes: number) {
  const gb = bytes / (1024 * 1024 * 1024);
  return gb >= 1 ? `${gb.toFixed(1)}G` : `${(bytes / (1024 * 1024)).toFixed(0)}M`;
}

function getUsageColor(pct: number) {
  if (pct < 50) return '#4ade80';
  if (pct < 80) return '#fbbf24';
  return '#f87171';
}

export default function Header() {
  const { t, i18n } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const i18nKey = routeI18nMap[location.pathname];
  const currentBreadcrumb = i18nKey ? t(i18nKey) : t('header.unknown');
  const { mode, monitorInterval, cycleThemeWithRipple } = useTheme();
  const { tasks: allTasks, clearCompleted } = useTaskQueue();
  const runningTasks = allTasks.filter(t => t.status === 'running');
  const completedTasks = allTasks.filter(t => t.status !== 'running');
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [showTaskPanel, setShowTaskPanel] = useState(false);

  // 轮询系统状态
  useEffect(() => {
    if (monitorInterval <= 0) { setStats(null); return; }
    let alive = true;
    const poll = async () => {
      try {
        const s = await invoke<SystemStats>('get_system_stats');
        if (alive) setStats(s);
      } catch {}
    };
    poll();
    const timer = setInterval(poll, monitorInterval);
    return () => { alive = false; clearInterval(timer); };
  }, [monitorInterval]);

  const themeBtnRef = useRef<HTMLButtonElement>(null);

  const cycleTheme = (e: React.MouseEvent) => {
    const rect = themeBtnRef.current?.getBoundingClientRect();
    const x = rect ? rect.left + rect.width / 2 : e.clientX;
    const y = rect ? rect.top + rect.height / 2 : e.clientY;
    cycleThemeWithRipple(x, y);
  };

  const themeIcon = mode === 'dark' ? <Moon style={{ width: 16, height: 16 }} />
    : mode === 'light' ? <Sun style={{ width: 16, height: 16 }} />
    : <Monitor style={{ width: 16, height: 16 }} />;

  const themeLabel = mode === 'dark' ? t('settings.themeDark') : mode === 'light' ? t('settings.themeLight') : t('settings.themeSystem');

  const taskStatusLabel = (status: string) => {
    switch (status) {
      case 'running': return t('header.running');
      case 'done': return t('header.done');
      case 'cancelled': return t('header.cancelled');
      default: return t('header.errorStatus');
    }
  };

  return (
    <header className="main-header">
      <div className="header-left">
        <div className="header-breadcrumb">
          <span className="header-breadcrumb-item">PurinBox</span>
          <span className="header-breadcrumb-separator">/</span>
          <span className="header-breadcrumb-current">{currentBreadcrumb}</span>
        </div>
      </div>

      {/* 中间：系统性能指标 */}
      {stats ? (
        <div className="header-stats">
          <div className="header-stat-item" title={`${stats.cpu_name}\n${stats.cpu_cores} ${t('header.cores')}`}>
            <Cpu style={{ width: 13, height: 13, color: getUsageColor(stats.cpu_usage) }} />
            <span className="header-stat-value" style={{ color: getUsageColor(stats.cpu_usage) }}>{stats.cpu_usage.toFixed(0)}%</span>
            <MiniBar value={stats.cpu_usage} color={getUsageColor(stats.cpu_usage)} />
          </div>
          <div className="header-stat-divider" />
          <div className="header-stat-item" title={`${formatBytes(stats.memory_used)} / ${formatBytes(stats.memory_total)}`}>
            <MemoryStick style={{ width: 13, height: 13, color: getUsageColor(stats.memory_percent) }} />
            <span className="header-stat-value" style={{ color: getUsageColor(stats.memory_percent) }}>{stats.memory_percent.toFixed(0)}%</span>
            <MiniBar value={stats.memory_percent} color={getUsageColor(stats.memory_percent)} />
          </div>
          {stats.gpu_usage >= 0 ? (
            <>
              <div className="header-stat-divider" />
              <div className="header-stat-item" title={`${stats.gpu_name}\nGPU: ${stats.gpu_usage.toFixed(0)}%`}>
                <MonitorDot style={{ width: 13, height: 13, color: getUsageColor(stats.gpu_usage) }} />
                <span className="header-stat-value" style={{ color: getUsageColor(stats.gpu_usage) }}>{stats.gpu_usage.toFixed(0)}%</span>
                <MiniBar value={stats.gpu_usage} color={getUsageColor(stats.gpu_usage)} />
              </div>
              {stats.vram_percent >= 0 && stats.vram_total > 0 && !stats.gpu_name.includes('Apple') && (
                <>
                  <div className="header-stat-divider" />
                  <div className="header-stat-item" title={`VRAM: ${formatBytes(stats.vram_used)} / ${formatBytes(stats.vram_total)}`}>
                    <MemoryStick style={{ width: 13, height: 13, color: getUsageColor(stats.vram_percent) }} />
                    <span className="header-stat-value" style={{ color: getUsageColor(stats.vram_percent) }}>{stats.vram_percent.toFixed(0)}%</span>
                    <MiniBar value={stats.vram_percent} color={getUsageColor(stats.vram_percent)} />
                  </div>
                </>
              )}
            </>
          ) : stats.gpu_name && !stats.gpu_name.includes('未检测') && !stats.gpu_name.includes('Not detected') ? (
            <>
              <div className="header-stat-divider" />
              <div className="header-stat-item" title={stats.gpu_name}>
                <MonitorDot style={{ width: 13, height: 13, color: '#a78bfa' }} />
                <span className="header-stat-value" style={{ color: '#a78bfa', fontSize: 10 }}>GPU</span>
              </div>
            </>
          ) : null}
        </div>
      ) : <div />}

      <div className="header-right">
        {/* 任务队列 */}
        <div style={{ position: 'relative' }}>
          <button className="header-btn" title={t('header.taskQueue')} onClick={() => setShowTaskPanel(!showTaskPanel)} style={{ position: 'relative' }}>
            <ListTodo />
            {runningTasks.length > 0 && (
              <span style={{ position: 'absolute', top: 2, right: 2, width: 8, height: 8, borderRadius: '50%', background: '#4ade80', animation: 'pulse 2s infinite' }} />
            )}
          </button>
          {showTaskPanel && (
            <div className="header-dropdown" style={{ minWidth: 280 }}>
              <div style={{ padding: '12px 16px', fontWeight: 700, fontSize: 13, borderBottom: '1px solid var(--color-border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>{t('header.taskQueue')}</span>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  {allTasks.length > 0 && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{runningTasks.length} {t('header.running')}</span>}
                  {completedTasks.length > 0 && (
                    <button onClick={(e) => { e.stopPropagation(); clearCompleted(); }} title={t('header.clearDoneTitle')}
                      style={{ display: 'flex', alignItems: 'center', gap: 3, padding: '2px 6px', borderRadius: 4, border: '1px solid var(--color-border)', background: 'transparent', color: 'var(--color-text-tertiary)', fontSize: 10, cursor: 'pointer', transition: 'all 0.15s' }}
                      onMouseEnter={e => { (e.currentTarget).style.color = '#f87171'; (e.currentTarget).style.borderColor = 'rgba(248,113,113,0.3)'; }}
                      onMouseLeave={e => { (e.currentTarget).style.color = 'var(--color-text-tertiary)'; (e.currentTarget).style.borderColor = 'var(--color-border)'; }}
                    >
                      <Trash2 style={{ width: 10, height: 10 }} /> {t('header.clearDone')}
                    </button>
                  )}
                </div>
              </div>
              {allTasks.length === 0 ? (
                <div style={{ padding: '20px 16px', textAlign: 'center', color: 'var(--color-text-tertiary)', fontSize: 12 }}>
                  {t('header.noTasks')}
                </div>
              ) : (
                <div style={{ maxHeight: 300, overflowY: 'auto' }}>
                  {allTasks.map(task => (
                    <div key={task.id}
                      onClick={() => { const route = TASK_ROUTE_MAP[task.id]; if (route) { navigate(route); setShowTaskPanel(false); } }}
                      style={{ padding: '10px 16px', borderBottom: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 4, cursor: TASK_ROUTE_MAP[task.id] ? 'pointer' : 'default', transition: 'background 0.15s' }}
                      onMouseEnter={e => { if (TASK_ROUTE_MAP[task.id]) (e.currentTarget as HTMLDivElement).style.background = 'var(--color-bg-hover)'; }}
                      onMouseLeave={e => { (e.currentTarget as HTMLDivElement).style.background = ''; }}
                    >
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8 }}>
                        <span style={{ fontSize: 12, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>{task.name}</span>
                        <span style={{
                          fontSize: 10, padding: '1px 6px', borderRadius: 4, whiteSpace: 'nowrap', flexShrink: 0,
                          background: task.status === 'running' ? 'rgba(74,222,128,0.1)' : task.status === 'done' ? 'rgba(96,165,250,0.1)' : 'rgba(248,113,113,0.1)',
                          color: task.status === 'running' ? '#4ade80' : task.status === 'done' ? '#60a5fa' : '#f87171',
                        }}>
                          {taskStatusLabel(task.status)}
                        </span>
                      </div>
                      {task.status === 'running' && task.total > 0 && (
                        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                          <div style={{ flex: 1, height: 3, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}>
                            <div style={{ width: `${task.progress}%`, height: '100%', borderRadius: 2, background: '#4ade80', transition: 'width 0.3s' }} />
                          </div>
                          <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{task.current}/{task.total}</span>
                        </div>
                      )}
                      <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{task.message}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* 主题切换 */}
        <button ref={themeBtnRef} className="header-btn" onClick={cycleTheme} title={themeLabel}>
          {themeIcon}
        </button>

        {/* 语言切换 */}
        <button className="header-btn" title={t('settings.language')}
          onClick={() => {
            const langs = availableLanguages.map(l => l.value);
            const idx = langs.indexOf(i18n.language);
            changeLanguage(langs[(idx + 1) % langs.length]);
          }}
          style={{ fontSize: 13, fontWeight: 800, letterSpacing: '-0.02em' }}
        >
          {{ 'zh-CN': '中', en: 'EN', ja: '日' }[i18n.language] || '中'}
        </button>
      </div>
    </header>
  );
}
