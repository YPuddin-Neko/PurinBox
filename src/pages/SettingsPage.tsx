import { Settings, Monitor, Palette, Globe, Info } from 'lucide-react';
import { useState } from 'react';

export default function SettingsPage() {
  const [theme, setTheme] = useState('dark');
  const [language, setLanguage] = useState('zh-CN');
  const [outputDir, setOutputDir] = useState('');

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
          <div className="form-group" style={{ marginBottom: 'var(--space-4)' }}>
            <label className="form-label">主题</label>
            <select className="form-select" value={theme} onChange={(e) => setTheme(e.target.value)}>
              <option value="dark">深色模式</option>
              <option value="light">浅色模式 (即将推出)</option>
              <option value="system">跟随系统</option>
            </select>
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
            <select className="form-select" value={language} onChange={(e) => setLanguage(e.target.value)}>
              <option value="zh-CN">简体中文</option>
              <option value="en">English</option>
              <option value="ja">日本語</option>
            </select>
          </div>
        </div>

        {/* Output */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <Monitor style={{ width: 16, height: 16, color: 'var(--color-accent-tertiary)' }} />
              <span className="tool-panel-title">输出设置</span>
            </div>
          </div>
          <div className="form-group">
            <label className="form-label">默认输出目录</label>
            <input className="form-input" placeholder="选择默认输出文件夹路径..." value={outputDir} onChange={(e) => setOutputDir(e.target.value)} />
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
