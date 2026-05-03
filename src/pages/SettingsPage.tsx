import { Settings, Palette, Info, Sun, Moon, Monitor, Check, Activity } from 'lucide-react';
import { useTheme } from '../components/ThemeProvider';

const intervalOptions = [
  { value: 1000, label: '1 秒', desc: '实时' },
  { value: 2000, label: '2 秒', desc: '较快' },
  { value: 3000, label: '3 秒', desc: '默认' },
  { value: 5000, label: '5 秒', desc: '节能' },
  { value: 10000, label: '10 秒', desc: '低频' },
  { value: 0, label: '关闭', desc: '不检测' },
];

export default function SettingsPage() {
  const { mode, setMode, monitorInterval, setMonitorInterval } = useTheme();

  const themeOptions = [
    { value: 'dark' as const, label: '深色模式', icon: <Moon style={{ width: 16, height: 16 }} />, desc: '深色背景，护眼模式' },
    { value: 'light' as const, label: '浅色模式', icon: <Sun style={{ width: 16, height: 16 }} />, desc: '浅色背景，明亮清晰' },
    { value: 'system' as const, label: '跟随系统', icon: <Monitor style={{ width: 16, height: 16 }} />, desc: '自动跟随操作系统设置' },
  ];

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Settings style={{ width: 28, height: 28, color: 'var(--color-text-secondary)' }} />
          <h1 className="page-title">设置</h1>
        </div>
        <p className="page-subtitle">配置工具箱的全局选项</p>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)', maxWidth: 640 }}>
        {/* Appearance */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <Palette style={{ width: 16, height: 16, color: 'var(--color-accent-primary)' }} />
              <span className="tool-panel-title">外观</span>
            </div>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 'var(--space-3)' }}>
            {themeOptions.map(opt => {
              const active = mode === opt.value;
              return (
                <div key={opt.value} onClick={() => setMode(opt.value)} style={{
                  padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1.5px solid ${active ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
                  background: active ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s', position: 'relative',
                  display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)',
                }}>
                  {active && (
                    <div style={{ position: 'absolute', top: 8, right: 8, width: 18, height: 18, borderRadius: '50%', background: 'var(--color-accent-primary)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                      <Check style={{ width: 11, height: 11, color: '#fff' }} />
                    </div>
                  )}
                  <div style={{ width: 36, height: 36, borderRadius: 'var(--radius-sm)', background: active ? 'rgba(124,92,252,0.12)' : 'rgba(255,255,255,0.04)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: active ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)' }}>
                    {opt.icon}
                  </div>
                  <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{opt.label}</span>
                  <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{opt.desc}</span>
                </div>
              );
            })}
          </div>
        </div>

        {/* Monitor Interval */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <Activity style={{ width: 16, height: 16, color: '#4ade80' }} />
              <span className="tool-panel-title">系统监控</span>
            </div>
          </div>
          <div>
            <label className="form-label" style={{ marginBottom: 8 }}>检测间隔</label>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 'var(--space-2)' }}>
              {intervalOptions.map(opt => {
                const active = monitorInterval === opt.value;
                return (
                  <div key={opt.value} onClick={() => setMonitorInterval(opt.value)} style={{
                    padding: '8px 12px', borderRadius: 'var(--radius-sm)',
                    border: `1.5px solid ${active ? (opt.value === 0 ? 'rgba(248,113,113,0.5)' : 'var(--color-border-active)') : 'var(--color-border)'}`,
                    background: active ? (opt.value === 0 ? 'rgba(248,113,113,0.06)' : 'rgba(124,92,252,0.06)') : 'var(--color-bg-input)',
                    cursor: 'pointer', transition: 'all 0.15s',
                    display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                  }}>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
                      <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{opt.label}</span>
                      <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>{opt.desc}</span>
                    </div>
                    {active && (
                      <div style={{ width: 16, height: 16, borderRadius: '50%', background: opt.value === 0 ? '#f87171' : 'var(--color-accent-primary)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                        <Check style={{ width: 10, height: 10, color: '#fff' }} />
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
            <p style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 8, lineHeight: 1.6 }}>
              控制顶栏和首页系统性能指标（CPU/RAM/GPU）的刷新频率。间隔越短数据越实时，但会略微增加系统开销。选择「关闭」将完全停止检测。
            </p>
          </div>
        </div>


        {/* About */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <Info style={{ width: 16, height: 16, color: 'var(--color-text-tertiary)' }} />
              <span className="tool-panel-title">关于</span>
            </div>
          </div>
          <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.8 }}>
            <p><strong>AI Train Tools</strong> · v0.1.0</p>
            <p>基于 Tauri 2 + React + TypeScript 构建</p>
            <p style={{ color: 'var(--color-text-tertiary)' }}>高性能 AI 训练数据处理工具箱</p>
          </div>
        </div>
      </div>
    </div>
  );
}
