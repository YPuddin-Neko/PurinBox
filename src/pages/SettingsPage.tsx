import { Settings, Palette, Globe, Info, Sun, Moon, Monitor, Check } from 'lucide-react';
import { useTheme } from '../components/ThemeProvider';

export default function SettingsPage() {
  const { mode, setMode } = useTheme();

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

        {/* Language */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <Globe style={{ width: 16, height: 16, color: 'var(--color-accent-secondary)' }} />
              <span className="tool-panel-title">语言</span>
            </div>
          </div>
          <div className="form-group">
            <label className="form-label">界面语言</label>
            <select className="form-select" defaultValue="zh-CN">
              <option value="zh-CN">简体中文</option>
              <option value="en">English</option>
              <option value="ja">日本語</option>
            </select>
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
