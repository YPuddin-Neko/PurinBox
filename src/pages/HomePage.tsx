import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppSettings } from '../components/ThemeProvider';
import { MonitorDot } from 'lucide-react';

interface SystemStats {
  cpu_usage: number; cpu_name: string; cpu_cores: number;
  memory_used: number; memory_total: number; memory_percent: number;
  gpu_name: string; gpu_usage: number; vram_used: number; vram_total: number; vram_percent: number;
}

function formatBytes(bytes: number): string {
  const gb = bytes / (1024 * 1024 * 1024);
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
}

function GaugeRing({ value, color, label, detail, subtitle, size = 80 }: { value: number; color: string; label: string; detail: string; subtitle?: string; size?: number }) {
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
      {subtitle && <span style={{ fontSize: 9, color: '#a78bfa', fontWeight: 600 }}>{subtitle}</span>}
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

  return (
    <div className="page">
      {/* Logo */}
      <div style={{ display: 'flex', justifyContent: 'center', marginTop: '-8px', marginBottom: '16px' }}>
        <img src="/logo.png" alt="PurinBox"
          style={{ maxWidth: 340, width: '100%', height: 'auto', objectFit: 'contain', userSelect: 'none', pointerEvents: 'none' }}
          draggable={false} />
      </div>

      {/* System Monitor */}
      <div className="tool-panel" style={{ padding: 'var(--space-5) var(--space-6)' }}>
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
            <GaugeRing value={stats.memory_percent} color={getUsageColor(stats.memory_percent)} label="RAM"
              detail={`${formatBytes(stats.memory_used)} / ${formatBytes(stats.memory_total)}`} />
            {stats.gpu_usage >= 0 ? (
              <>
                <GaugeRing value={stats.gpu_usage} color={getUsageColor(stats.gpu_usage)} label="GPU"
                  detail={stats.gpu_name}
                  subtitle={stats.gpu_name.includes('Apple') ? '统一内存' : undefined} />
                {!stats.gpu_name.includes('Apple') && (
                  <GaugeRing value={stats.vram_percent >= 0 ? stats.vram_percent : 0}
                    color={stats.vram_percent >= 0 ? getUsageColor(stats.vram_percent) : '#5a5e78'}
                    label="VRAM"
                    detail={stats.vram_total > 0 ? `${formatBytes(stats.vram_used)} / ${formatBytes(stats.vram_total)}` : 'N/A'} />
                )}
              </>
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
    </div>
  );
}
