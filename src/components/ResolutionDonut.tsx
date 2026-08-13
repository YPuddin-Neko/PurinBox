import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useTheme } from './ThemeProvider';

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

// 分类色板（前 6 名各一色，经 dataviz 校验器在两套主题面板底色上全项通过；
// 亮色模式的对比度 WARN 由图例全量标注 + 2px 表面间隙补偿）
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

function polar(cx: number, cy: number, r: number, angle: number): [number, number] {
  return [cx + r * Math.cos(angle), cy + r * Math.sin(angle)];
}

/** 环形扇区路径（外弧 → 内弧回勾） */
function arcPath(cx: number, cy: number, rOuter: number, rInner: number, a0: number, a1: number): string {
  const largeArc = a1 - a0 > Math.PI ? 1 : 0;
  const [x0, y0] = polar(cx, cy, rOuter, a0);
  const [x1, y1] = polar(cx, cy, rOuter, a1);
  const [x2, y2] = polar(cx, cy, rInner, a1);
  const [x3, y3] = polar(cx, cy, rInner, a0);
  return [
    `M ${x0} ${y0}`,
    `A ${rOuter} ${rOuter} 0 ${largeArc} 1 ${x1} ${y1}`,
    `L ${x2} ${y2}`,
    `A ${rInner} ${rInner} 0 ${largeArc} 0 ${x3} ${y3}`,
    'Z',
  ].join(' ');
}

/**
 * 分辨率分布环形图：前 6 名各占一个分类色，其余折入中性灰的"其他"。
 * 悬停扇区/图例行联动高亮，环心显示悬停项的数量与占比。
 */
export default function ResolutionDonut({ groups, totalImages }: ResolutionDonutProps) {
  const { t } = useTranslation();
  const { resolved } = useTheme();
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
  const rInner = 56; // 细环

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
          {arcs.map(a => (
            <path
              key={a.index}
              d={a.a1 - a.a0 >= Math.PI * 2 - 1e-6
                ? // 单一扇区占满时画整环（arc 命令无法表示 360°）
                  `M ${cx} ${cy - rOuter} A ${rOuter} ${rOuter} 0 1 1 ${cx - 0.01} ${cy - rOuter} Z
                   M ${cx} ${cy - rInner} A ${rInner} ${rInner} 0 1 0 ${cx - 0.01} ${cy - rInner} Z`
                : arcPath(cx, cy, rOuter, rInner, a.a0, a.a1)}
              fill={a.color}
              fillRule="evenodd"
              stroke="var(--color-bg-card)"
              strokeWidth={2}
              opacity={hovered == null || hovered === a.index ? 1 : 0.4}
              style={{ transition: 'opacity 0.15s', cursor: 'default' }}
              onMouseEnter={() => setHovered(a.index)}
              onMouseLeave={() => setHovered(null)}
            >
              <title>{`${a.label} · ${a.count} (${a.percent.toFixed(1)}%)`}</title>
            </path>
          ))}
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
              background: hovered === i ? 'var(--color-bg-hover, rgba(124, 92, 252, 0.06))' : 'transparent',
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
