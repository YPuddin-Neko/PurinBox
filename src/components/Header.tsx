import { useLocation, useNavigate } from 'react-router-dom';
import { useState, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Sun, Moon, Monitor, ListTodo, Cpu, MemoryStick, MonitorDot, Trash2 } from 'lucide-react';
import { useAppSettings } from './ThemeProvider';
import { useTaskList, useTaskQueue, type TaskStatus } from './TaskContext';
import { changeLanguage, availableLanguages } from '../i18n';
import useSystemStats, { getUsageColor } from '../hooks/useSystemStats';
import { formatBytes } from '../utils/format';
import { routeI18nMap, routeSectionMap, TASK_ROUTE_MAP } from '../appRegistry';
import '../styles/layout.css';

const MEMORY = { memory: true, compact: true } as const;

function MiniBar({ value, color, max = 100 }: { value: number; color: string; max?: number }) {
  const pct = Math.min(Math.max(value / max * 100, 0), 100);
  return (
    <div style={{ width: 32, height: 4, borderRadius: 999, background: 'var(--color-bg-hover)', overflow: 'hidden' }}>
      <div style={{ width: `${pct}%`, height: '100%', borderRadius: 999, background: color, transition: 'width 0.5s var(--ease-out-expo)' }} />
    </div>
  );
}

const TASK_STATUS_COLORS: Record<TaskStatus, { background: string; color: string }> = {
  running: { background: 'var(--color-success-bg)', color: 'var(--color-success)' },
  done: { background: 'var(--color-info-bg)', color: 'var(--color-info)' },
  warning: { background: 'var(--color-warning-bg)', color: 'var(--color-warning)' },
  cancelled: { background: 'var(--color-bg-tertiary)', color: 'var(--color-text-tertiary)' },
  error: { background: 'var(--color-error-bg)', color: 'var(--color-error)' },
};

export default function Header() {
  const { t, i18n } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const i18nKey = routeI18nMap[location.pathname];
  const sectionKey = routeSectionMap[location.pathname];
  const currentBreadcrumb = i18nKey ? t(i18nKey) : t('header.unknown');
  const { mode, cycleThemeWithRipple } = useAppSettings();
  const allTasks = useTaskList();
  const { clearCompleted } = useTaskQueue();
  const runningTasks = allTasks.filter(t => t.status === 'running');
  const completedTasks = allTasks.filter(t => t.status !== 'running');
  const stats = useSystemStats();
  const [showTaskPanel, setShowTaskPanel] = useState(false);

  const themeBtnRef = useRef<HTMLButtonElement>(null);
  const taskAnchorRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showTaskPanel) return;
    const onDown = (e: MouseEvent) => {
      if (taskAnchorRef.current && !taskAnchorRef.current.contains(e.target as Node)) {
        setShowTaskPanel(false);
      }
    };
    window.addEventListener('mousedown', onDown);
    return () => window.removeEventListener('mousedown', onDown);
  }, [showTaskPanel]);

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

  const taskStatusLabel = (status: TaskStatus) => {
    switch (status) {
      case 'running': return t('header.running');
      case 'done': return t('header.done');
      case 'warning': return t('common.warning');
      case 'cancelled': return t('header.cancelled');
      default: return t('header.errorStatus');
    }
  };

  return (
    <header className="main-header">
      <div className="header-left">
        <div className="header-breadcrumb">
          <span className="header-breadcrumb-item">PurinBox</span>
          {sectionKey && (
            <>
              <span className="header-breadcrumb-separator">/</span>
              <span className="header-breadcrumb-item">{t(sectionKey)}</span>
            </>
          )}
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
          <div className="header-stat-item" title={`${formatBytes(stats.memory_used, MEMORY)} / ${formatBytes(stats.memory_total, MEMORY)}`}>
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
                  <div className="header-stat-item" title={`VRAM: ${formatBytes(stats.vram_used, MEMORY)} / ${formatBytes(stats.vram_total, MEMORY)}`}>
                    <MemoryStick style={{ width: 13, height: 13, color: getUsageColor(stats.vram_percent) }} />
                    <span className="header-stat-value" style={{ color: getUsageColor(stats.vram_percent) }}>{stats.vram_percent.toFixed(0)}%</span>
                    <MiniBar value={stats.vram_percent} color={getUsageColor(stats.vram_percent)} />
                  </div>
                </>
              )}
            </>
          ) : stats.gpu_name ? (
            <>
              <div className="header-stat-divider" />
              <div className="header-stat-item" title={stats.gpu_name}>
                <MonitorDot style={{ width: 13, height: 13, color: 'var(--color-accent-primary)' }} />
                <span className="header-stat-value" style={{ color: 'var(--color-accent-primary)', fontSize: 10 }}>GPU</span>
              </div>
            </>
          ) : null}
        </div>
      ) : <div />}

      <div className="header-right">
        {/* 任务队列 */}
        <div ref={taskAnchorRef} style={{ position: 'relative' }}>
          <button type="button" className="header-btn" title={t('header.taskQueue')} onClick={() => setShowTaskPanel(!showTaskPanel)}>
            <ListTodo />
            {runningTasks.length > 0 && (
              <span style={{ position: 'absolute', top: 4, right: 4, width: 7, height: 7, borderRadius: '50%', background: 'var(--color-success)', animation: 'pulse 2s infinite' }} />
            )}
          </button>
          {showTaskPanel && (
            <div className="header-dropdown">
              <div style={{ padding: '11px 14px', fontWeight: 700, fontSize: 12.5, borderBottom: '1px solid var(--color-border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>{t('header.taskQueue')}</span>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  {allTasks.length > 0 && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{runningTasks.length} {t('header.running')}</span>}
                  {completedTasks.length > 0 && (
                    <button type="button" className="task-clear-button" onClick={(e) => { e.stopPropagation(); clearCompleted(); }} title={t('header.clearDoneTitle')}>
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
                    <div key={task.id} className={TASK_ROUTE_MAP[task.id] ? 'task-queue-item' : undefined}
                      onClick={() => { const route = TASK_ROUTE_MAP[task.id]; if (route) { navigate(route); setShowTaskPanel(false); } }}
                      style={{ padding: '10px 14px', borderBottom: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 4, cursor: TASK_ROUTE_MAP[task.id] ? 'pointer' : 'default', transition: 'background 0.15s' }}
                    >
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8 }}>
                        <span style={{ fontSize: 12, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>{task.name}</span>
                        <span style={{
                          fontSize: 10, padding: '1px 6px', borderRadius: 5, fontWeight: 600, whiteSpace: 'nowrap', flexShrink: 0,
                          ...TASK_STATUS_COLORS[task.status],
                        }}>
                          {taskStatusLabel(task.status)}
                        </span>
                      </div>
                      {task.status === 'running' && task.total > 0 && (
                        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                          <div style={{ flex: 1, height: 4, borderRadius: 999, background: 'var(--color-bg-hover)', overflow: 'hidden' }}>
                            <div style={{ width: `${Math.min(100, (task.current / task.total) * 100)}%`, height: '100%', borderRadius: 999, background: 'var(--color-success)', transition: 'width 0.3s' }} />
                          </div>
                          <span style={{ fontSize: 10, fontFamily: 'var(--font-mono)', color: 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{task.current}/{task.total}</span>
                        </div>
                      )}
                      <div style={{ fontSize: 10.5, color: 'var(--color-text-tertiary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{task.message}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* 主题切换 */}
        <button type="button" ref={themeBtnRef} className="header-btn" onClick={cycleTheme} title={themeLabel}>
          {themeIcon}
        </button>

        {/* 语言切换 */}
        <button type="button" className="header-btn" title={t('settings.language')}
          onClick={() => {
            const idx = availableLanguages.indexOf(i18n.language);
            changeLanguage(availableLanguages[(idx + 1) % availableLanguages.length]);
          }}
          style={{ fontSize: 12, fontWeight: 700 }}
        >
          {{ 'zh-CN': '中', en: 'EN', ja: '日' }[i18n.language] || '中'}
        </button>
      </div>
    </header>
  );
}
