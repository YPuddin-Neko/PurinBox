import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppSettings } from './ThemeProvider';
import { arcPath } from '../utils/donut';

interface DonutGroup {
  width: number;
  height: number;
  count: number;
  percent: number;
}

interface ResolutionDonutProps {
  groups: DonutGroup[];   // 需按数量降序
  totalImages: number;
}

// 分类色板（前 6 名各一色）；亮色模式下部分颜色与面板底色对比度偏低，
// 由图例全量标注 + 扇区间 2px 表面色间隙补偿
const SERIES_LIGHT = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300'];
const SERIES_DARK = ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300'];
// "其他" 是余量而非身份，用中性灰
const OTHER_LIGHT = '#9aa0b5';
const OTHER_DARK = '#565c74';

const TOP_N = 6;

interface Slice {
  label: string;
  count: number;
  percent: number;
  color: string;
  isOther: boolean;
}

/**
 * 分辨率分布环形图：前 6 名各占一个分类色，其余折入中性灰的"其他"。
 * 悬停扇区/图例行联动高亮，环心显示悬停项的数量与占比。
 */
export default function ResolutionDonut({ groups, totalImages }: ResolutionDonutProps) {
  const { t } = useTranslation();
  const { resolved } = useAppSettings();
  const [hovered, setHovered] = useState<number | null>(null);

  const slices = useMemo<Slice[]>(() => {
    const series = resolved === 'light' ? SERIES_LIGHT : SERIES_DARK;
    const other = resolved === 'light' ? OTHER_LIGHT : OTHER_DARK;
    const top = groups.slice(0, TOP_N).map((g, i) => ({
      label: `${g.width}×${g.height}`,
      count: g.count,
      percent: g.percent,
      color: series[i],
      isOther: false,
    }));
    const rest = groups.slice(TOP_N);
    if (rest.length > 0) {
      const count = rest.reduce((s, g) => s + g.count, 0);
      top.push({
        label: t('resolutionAnalyze.otherSlice', { n: rest.length }),
        count,
        percent: rest.reduce((s, g) => s + g.percent, 0),
        color: other,
        isOther: true,
      });
    }
    return top;
  }, [groups, resolved, t]);

  const size = 168;
  const cx = size / 2;
  const cy = size / 2;
  const rOuter = 80;
  const rInner = 56;

  // 从 12 点方向起，顺时针
  let angle = -Math.PI / 2;
  const total = slices.reduce((s, x) => s + x.count, 0) || 1;
  const arcs = slices.map((s, i) => {
    const sweep = (s.count / total) * Math.PI * 2;
    const a0 = angle;
    angle += sweep;
    return { ...s, a0, a1: angle, index: i };
  });

  const center = hovered != null ? slices[hovered] : null;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', justifyContent: 'center' }}>
        <svg width={size} height={size} role="img" aria-label={t('resolutionAnalyze.resolutionDistribution')}>
          {arcs.map(a => {
            const shared = {
              opacity: hovered == null || hovered === a.index ? 1 : 0.4,
              style: { transition: 'opacity 0.15s', cursor: 'default' } as const,
              onMouseEnter: () => setHovered(a.index),
              onMouseLeave: () => setHovered(null),
            };
            const title = <title>{`${a.label} · ${a.count} (${a.percent.toFixed(1)}%)`}</title>;
            // 单一扇区占满整环：arc 命令表示不了 360°（起点=终点画不出来，
            // 拆两段弧又容易选错圆心），直接用粗描边圆画环，且无需扇区分隔缝
            if (a.a1 - a.a0 >= Math.PI * 2 - 1e-6) {
              return (
                <circle
                  key={a.index}
                  cx={cx}
                  cy={cy}
                  r={(rOuter + rInner) / 2}
                  fill="none"
                  stroke={a.color}
                  strokeWidth={rOuter - rInner}
                  {...shared}
                >
                  {title}
                </circle>
              );
            }
            return (
              <path
                key={a.index}
                d={arcPath(cx, cy, rOuter, rInner, a.a0, a.a1)}
                fill={a.color}
                stroke="var(--color-bg-card)"
                strokeWidth={2}
                {...shared}
              >
                {title}
              </path>
            );
          })}
          <text x={cx} y={cy - 4} textAnchor="middle" style={{ fill: 'var(--color-text-primary)', fontSize: 18, fontWeight: 700 }}>
            {center ? center.count : totalImages.toLocaleString()}
          </text>
          <text x={cx} y={cy + 14} textAnchor="middle" style={{ fill: 'var(--color-text-tertiary)', fontSize: 10 }}>
            {center ? `${center.percent.toFixed(1)}%` : t('resolutionAnalyze.totalImages')}
          </text>
        </svg>
      </div>

      {/* 图例：全量标注（数值不依赖颜色识别） */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
        {slices.map((s, i) => (
          <div
            key={i}
            onMouseEnter={() => setHovered(i)}
            onMouseLeave={() => setHovered(null)}
            style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '2px 6px', borderRadius: 4,
              background: hovered === i ? 'var(--color-bg-hover)' : 'transparent',
              transition: 'background 0.15s',
            }}
          >
            <span style={{ width: 8, height: 8, borderRadius: 2, background: s.color, flexShrink: 0 }} />
            <span style={{
              fontSize: 11, color: 'var(--color-text-secondary)', flex: 1, minWidth: 0,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              fontFamily: s.isOther ? undefined : 'monospace',
            }}>
              {s.label}
            </span>
            <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)' }}>{s.count}</span>
            <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', minWidth: 42, textAlign: 'right' }}>
              {s.percent.toFixed(1)}%
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
