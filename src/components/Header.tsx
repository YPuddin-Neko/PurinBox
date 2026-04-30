import { useLocation } from 'react-router-dom';
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Sun, Moon, Monitor, ListTodo, Cpu, MemoryStick, MonitorDot } from 'lucide-react';
import { useTheme } from './ThemeProvider';
import { useTaskQueue } from './TaskContext';
import '../styles/layout.css';

const routeNames: Record<string, { breadcrumb: string; title: string }> = {
  '/': { breadcrumb: '工作台', title: '工作台' },
  '/scale': { breadcrumb: '图片缩放', title: '图片缩放' },
  '/flip': { breadcrumb: '图片处理', title: '图片处理' },
  '/filter': { breadcrumb: '分辨率筛选', title: '分辨率筛选' },
  '/file-keeper': { breadcrumb: '保留指定文件', title: '保留指定文件' },
  '/format-convert': { breadcrumb: '图片格式转换', title: '图片格式转换' },
  '/alpha-convert': { breadcrumb: '转换透明通道', title: '转换透明通道' },
  '/batch-rename': { breadcrumb: '批量重命名', title: '批量重命名' },
  '/tagger': { breadcrumb: '图片打标', title: '图片打标' },
  '/labeling': { breadcrumb: '数据集打标', title: '数据集打标' },
  '/crop': { breadcrumb: '图像裁切', title: '图像裁切' },
  '/resize': { breadcrumb: '尺寸调整', title: '尺寸调整' },
  '/convert': { breadcrumb: '格式转换', title: '格式转换' },
  '/augment': { breadcrumb: '数据增强', title: '数据增强' },
  '/organize': { breadcrumb: '数据集管理', title: '数据集管理' },
  '/settings': { breadcrumb: '设置', title: '设置' },
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
  const location = useLocation();
  const currentRoute = routeNames[location.pathname] || { breadcrumb: '未知', title: '未知' };
  const { mode, setMode, monitorInterval } = useTheme();
  const { tasks: allTasks } = useTaskQueue();
  const runningTasks = allTasks.filter(t => t.status === 'running');
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

  const cycleTheme = () => {
    const next: Record<string, 'light' | 'dark' | 'system'> = { dark: 'light', light: 'system', system: 'dark' };
    setMode(next[mode]);
  };

  const themeIcon = mode === 'dark' ? <Moon style={{ width: 16, height: 16 }} />
    : mode === 'light' ? <Sun style={{ width: 16, height: 16 }} />
    : <Monitor style={{ width: 16, height: 16 }} />;

  const themeLabel = mode === 'dark' ? '深色' : mode === 'light' ? '浅色' : '跟随系统';

  return (
    <header className="main-header">
      <div className="header-left">
        <div className="header-breadcrumb">
          <span className="header-breadcrumb-item">AI Train Tools</span>
          <span className="header-breadcrumb-separator">/</span>
          <span className="header-breadcrumb-current">{currentRoute.breadcrumb}</span>
        </div>
      </div>

      {/* 中间：系统性能指标 */}
      <div className="header-stats">
        {stats && (
          <>
            <div className="header-stat-item" title={`${stats.cpu_name}\n${stats.cpu_cores} 核心`}>
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
                {stats.vram_percent >= 0 && stats.vram_total > 0 && (
                  <>
                    <div className="header-stat-divider" />
                    <div className="header-stat-item" title={`显存: ${formatBytes(stats.vram_used)} / ${formatBytes(stats.vram_total)}`}>
                      <MemoryStick style={{ width: 13, height: 13, color: getUsageColor(stats.vram_percent) }} />
                      <span className="header-stat-value" style={{ color: getUsageColor(stats.vram_percent) }}>{stats.vram_percent.toFixed(0)}%</span>
                      <MiniBar value={stats.vram_percent} color={getUsageColor(stats.vram_percent)} />
                    </div>
                  </>
                )}
              </>
            ) : stats.gpu_name && !stats.gpu_name.includes('未检测') ? (
              <>
                <div className="header-stat-divider" />
                <div className="header-stat-item" title={stats.gpu_name}>
                  <MonitorDot style={{ width: 13, height: 13, color: '#a78bfa' }} />
                  <span className="header-stat-value" style={{ color: '#a78bfa', fontSize: 10 }}>GPU</span>
                </div>
              </>
            ) : null}
          </>
        )}
      </div>

      <div className="header-right">
        {/* 任务队列 */}
        <div style={{ position: 'relative' }}>
          <button className="header-btn" title="任务队列" onClick={() => setShowTaskPanel(!showTaskPanel)} style={{ position: 'relative' }}>
            <ListTodo />
            {runningTasks.length > 0 && (
              <span style={{ position: 'absolute', top: 2, right: 2, width: 8, height: 8, borderRadius: '50%', background: '#4ade80', animation: 'pulse 2s infinite' }} />
            )}
          </button>
          {showTaskPanel && (
            <div className="header-dropdown" style={{ minWidth: 280 }}>
              <div style={{ padding: '12px 16px', fontWeight: 700, fontSize: 13, borderBottom: '1px solid var(--color-border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>任务队列</span>
                {allTasks.length > 0 && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{runningTasks.length} 运行中</span>}
              </div>
              {allTasks.length === 0 ? (
                <div style={{ padding: '20px 16px', textAlign: 'center', color: 'var(--color-text-tertiary)', fontSize: 12 }}>
                  暂无任务
                </div>
              ) : (
                <div style={{ maxHeight: 300, overflowY: 'auto' }}>
                  {allTasks.map(task => (
                    <div key={task.id} style={{ padding: '10px 16px', borderBottom: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 4 }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <span style={{ fontSize: 12, fontWeight: 600 }}>{task.name}</span>
                        <span style={{
                          fontSize: 10, padding: '1px 6px', borderRadius: 4,
                          background: task.status === 'running' ? 'rgba(74,222,128,0.1)' : task.status === 'done' ? 'rgba(96,165,250,0.1)' : 'rgba(248,113,113,0.1)',
                          color: task.status === 'running' ? '#4ade80' : task.status === 'done' ? '#60a5fa' : '#f87171',
                        }}>
                          {task.status === 'running' ? '运行中' : task.status === 'done' ? '已完成' : task.status === 'cancelled' ? '已取消' : '错误'}
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
        <button className="header-btn" onClick={cycleTheme} title={`当前: ${themeLabel}\n点击切换`}>
          {themeIcon}
        </button>
      </div>
    </header>
  );
}
