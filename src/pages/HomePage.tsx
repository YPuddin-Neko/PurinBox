import { Link } from 'react-router-dom';
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppSettings } from '../components/ThemeProvider';
import {
  Scaling, FlipHorizontal2, ScanSearch, FileCheck2, FileType,
  Layers, TextCursorInput, Tags, Crop, MonitorDot, Zap,
} from 'lucide-react';

const tools = [
  { id: 'scale', title: '图片缩放', desc: '批量上采样或下采样图片到目标分辨率，适配模型训练需求', icon: <Scaling />, color: 'purple', path: '/scale', tags: ['上采样', '下采样', '批量'], category: '数据集预处理' },
  { id: 'flip', title: '图片处理', desc: '对图片进行水平或垂直镜像翻转，用于数据增强', icon: <FlipHorizontal2 />, color: 'cyan', path: '/flip', tags: ['水平翻转', '垂直翻转', '批量'], category: '数据集预处理' },
  { id: 'filter', title: '分辨率筛选', desc: '根据分辨率条件筛选图片，支持删除或输出匹配的图片', icon: <ScanSearch />, color: 'pink', path: '/filter', tags: ['筛选', '分辨率', '清理'], category: '数据集预处理' },
  { id: 'file-keeper', title: '保留指定文件', desc: '勾选要保留的文件类型，一键删除其他文件', icon: <FileCheck2 />, color: 'orange', path: '/file-keeper', tags: ['清理', '后缀', '保留'], category: '数据集预处理' },
  { id: 'format-convert', title: '图片格式转换', desc: '支持 PSD 在内的主流格式转换到 PNG/JPG/WebP 等', icon: <FileType />, color: 'green', path: '/format-convert', tags: ['PSD', '格式', '批量'], category: '数据集预处理' },
  { id: 'alpha-convert', title: '转换透明通道', desc: '检测图片透明通道并转换为不透明图片', icon: <Layers />, color: 'blue', path: '/alpha-convert', tags: ['Alpha', '透明', '背景'], category: '数据集预处理' },
  { id: 'batch-rename', title: '批量重命名', desc: '按规则批量重命名图片，支持自定义前缀、编号和打乱顺序', icon: <TextCursorInput />, color: 'cyan', path: '/batch-rename', tags: ['重命名', '前缀', '编号'], category: '数据集预处理' },
  { id: 'tagger', title: '图片打标', desc: '使用 AI 模型自动为训练图片生成标签，支持 WD Tagger 等主流模型', icon: <Tags />, color: 'orange', path: '/tagger', tags: ['AI打标', 'WD Tagger', 'ONNX'], category: '数据集处理' },
  { id: 'crop', title: '图像裁切', desc: '智能裁切训练图像，支持自定义比例和批量处理', icon: <Crop />, color: 'green', path: '/crop', tags: ['裁切', '比例', '智能'], category: '更多工具' },
];

interface SystemStats {
  cpu_usage: number; cpu_name: string; cpu_cores: number;
  memory_used: number; memory_total: number; memory_percent: number;
  gpu_name: string; gpu_usage: number; vram_used: number; vram_total: number; vram_percent: number;
}

function formatBytes(bytes: number): string {
  const gb = bytes / (1024 * 1024 * 1024);
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
}

function GaugeRing({ value, color, label, detail, size = 80 }: { value: number; color: string; label: string; detail: string; size?: number }) {
  const r = (size - 8) / 2;
  const circ = 2 * Math.PI * r;
  const pct = Math.min(Math.max(value, 0), 100);
  const offset = circ - (pct / 100) * circ;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4 }}>
      <div style={{ position: 'relative', width: size, height: size }}>
        <svg width={size} height={size} style={{ transform: 'rotate(-90deg)' }}>
          <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--color-border)" strokeWidth={6} />
          <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke={color} strokeWidth={6}
            strokeDasharray={circ} strokeDashoffset={offset} strokeLinecap="round"
            style={{ transition: 'stroke-dashoffset 0.8s ease' }} />
        </svg>
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <span style={{ fontSize: 16, fontWeight: 800, fontFamily: "'JetBrains Mono', monospace", color }}>{pct.toFixed(0)}%</span>
        </div>
      </div>
      <span style={{ fontSize: 11, fontWeight: 700, color: 'var(--color-text-primary)' }}>{label}</span>
      <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', textAlign: 'center', lineHeight: 1.3 }}>{detail}</span>
    </div>
  );
}

function getUsageColor(pct: number) {
  if (pct < 50) return '#4ade80';
  if (pct < 80) return '#fbbf24';
  return '#f87171';
}

export default function HomePage() {
  const [stats, setStats] = useState<SystemStats | null>(null);
  const { monitorInterval } = useAppSettings();

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

  const trainingTools = tools.filter(t => t.category === '数据集预处理');
  const processingTools = tools.filter(t => t.category === '数据集处理');
  const otherTools = tools.filter(t => t.category === '更多工具');

  return (
    <div className="page">
      {/* Page Header */}
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '4px' }}>
          <Zap style={{ width: 28, height: 28, color: 'var(--color-accent-primary)' }} />
          <h1 className="page-title">工作台</h1>
        </div>
        <p className="page-subtitle">AI 训练数据处理工具集，助力高效准备训练数据</p>
      </div>

      {/* System Monitor */}
      <div className="tool-panel" style={{ marginBottom: 'var(--space-8)', padding: 'var(--space-5) var(--space-6)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
            <MonitorDot style={{ width: 16, height: 16, color: 'var(--color-accent-secondary)' }} />
            <span style={{ fontSize: 14, fontWeight: 700, color: 'var(--color-text-primary)' }}>系统监控</span>
          </div>
          {stats && (
            <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>
              {stats.cpu_name} · {stats.cpu_cores} 核心
            </span>
          )}
        </div>
        {stats ? (
          <div style={{ display: 'flex', justifyContent: 'space-around', alignItems: 'flex-start' }}>
            <GaugeRing value={stats.cpu_usage} color={getUsageColor(stats.cpu_usage)} label="CPU" detail={`${stats.cpu_cores} 核心`} />
            <GaugeRing value={stats.memory_percent} color={getUsageColor(stats.memory_percent)} label="内存"
              detail={`${formatBytes(stats.memory_used)} / ${formatBytes(stats.memory_total)}`} />
            {stats.gpu_usage >= 0 ? (
              <>
                <GaugeRing value={stats.gpu_usage} color={getUsageColor(stats.gpu_usage)} label="GPU" detail={stats.gpu_name} />
                <GaugeRing value={stats.vram_percent >= 0 ? stats.vram_percent : 0}
                  color={stats.vram_percent >= 0 ? getUsageColor(stats.vram_percent) : '#5a5e78'}
                  label="显存"
                  detail={stats.vram_total > 0 ? `${formatBytes(stats.vram_used)} / ${formatBytes(stats.vram_total)}` : 'N/A'} />
              </>
            ) : stats.gpu_name && !stats.gpu_name.includes('未检测') ? (
              <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4 }}>
                <div style={{ width: 80, height: 80, borderRadius: '50%', border: '6px solid rgba(124,92,252,0.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <span style={{ fontSize: 11, fontWeight: 700, color: '#a78bfa' }}>✓</span>
                </div>
                <span style={{ fontSize: 11, fontWeight: 700, color: 'var(--color-text-primary)' }}>GPU</span>
                <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', textAlign: 'center', lineHeight: 1.3, maxWidth: 120 }}>{stats.gpu_name}</span>
                <span style={{ fontSize: 9, color: '#a78bfa' }}>统一内存</span>
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4, opacity: 0.5 }}>
                <div style={{ width: 80, height: 80, borderRadius: '50%', border: '6px solid var(--color-border)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>N/A</span>
                </div>
                <span style={{ fontSize: 11, fontWeight: 700, color: 'var(--color-text-primary)' }}>GPU</span>
                <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>未检测到</span>
              </div>
            )}
          </div>
        ) : (
          <div style={{ display: 'flex', justifyContent: 'center', padding: 'var(--space-6)', color: 'var(--color-text-tertiary)', fontSize: 12 }}>
            正在检测系统性能...
          </div>
        )}
      </div>

      {/* 数据集预处理 */}
      <SectionHeader title="数据集预处理" gradient="var(--color-gradient-primary)" />
      <div className="tools-grid" style={{ marginBottom: 'var(--space-8)' }}>
        {trainingTools.map((tool, index) => (
          <Link key={tool.id} to={tool.path} className="tool-card" style={{ animationDelay: `${index * 0.05}s` }}>
            <div className={`tool-card-icon ${tool.color}`}>{tool.icon}</div>
            <div>
              <div className="tool-card-title">{tool.title}</div>
              <div className="tool-card-desc">{tool.desc}</div>
            </div>
            <div className="tool-card-tags">
              {tool.tags.map(tag => (<span key={tag} className="tool-card-tag">{tag}</span>))}
            </div>
          </Link>
        ))}
      </div>

      {/* 数据集处理 */}
      <SectionHeader title="数据集处理" gradient="linear-gradient(135deg, #f59e0b, #f97316)" />
      <div className="tools-grid" style={{ marginBottom: 'var(--space-8)' }}>
        {processingTools.map((tool, index) => (
          <Link key={tool.id} to={tool.path} className="tool-card" style={{ animationDelay: `${index * 0.05}s` }}>
            <div className={`tool-card-icon ${tool.color}`}>{tool.icon}</div>
            <div>
              <div className="tool-card-title">{tool.title}</div>
              <div className="tool-card-desc">{tool.desc}</div>
            </div>
            <div className="tool-card-tags">
              {tool.tags.map(tag => (<span key={tag} className="tool-card-tag">{tag}</span>))}
            </div>
          </Link>
        ))}
      </div>

      {/* 更多工具 */}
      <SectionHeader title="更多工具" gradient="var(--color-gradient-secondary)" />
      <div className="tools-grid">
        {otherTools.map((tool, index) => (
          <Link key={tool.id} to={tool.path} className="tool-card" style={{ animationDelay: `${index * 0.05}s` }}>
            <div className={`tool-card-icon ${tool.color}`}>{tool.icon}</div>
            <div>
              <div className="tool-card-title">{tool.title}</div>
              <div className="tool-card-desc">{tool.desc}</div>
            </div>
            <div className="tool-card-tags">
              {tool.tags.map(tag => (<span key={tag} className="tool-card-tag">{tag}</span>))}
            </div>
          </Link>
        ))}
      </div>
    </div>
  );
}

function SectionHeader({ title, gradient }: { title: string; gradient: string }) {
  return (
    <div style={{ marginBottom: 'var(--space-4)' }}>
      <h2 style={{
        fontSize: 'var(--font-size-lg)', fontWeight: 700,
        color: 'var(--color-text-primary)', marginBottom: 'var(--space-5)',
        letterSpacing: '-0.01em',
        display: 'flex', alignItems: 'center', gap: 'var(--space-2)',
      }}>
        <span style={{ width: 3, height: 18, borderRadius: 2, background: gradient, display: 'inline-block' }} />
        {title}
      </h2>
    </div>
  );
}
