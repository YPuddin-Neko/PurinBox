import { useId, type CSSProperties, type ReactNode } from 'react';
import './ui.css';

export interface RangeFieldProps {
  label: ReactNode;
  value: number;
  onChange: (value: number) => void;
  min: number;
  max: number;
  /** 默认 1 */
  step?: number;
  /** 滑块与数值的颜色（inline 时标签也用它），默认主题色 */
  color?: string;
  /** 数值的显示，默认 String(value)，如 `v => v.toFixed(1)`、`` v => `${v.toFixed(1)}x` `` */
  format?: (value: number) => ReactNode;
  /** 滑块下方两端的标注 [最小端, 最大端]；不传不显示（inline 时不显示） */
  ends?: readonly [ReactNode, ReactNode];
  /** 标签、滑块、数值排成一行（如聚类的融合权重）；默认标签与数值在上、滑块在下 */
  inline?: boolean;
  /** 数值文字的额外样式（如更大的字号） */
  valueStyle?: CSSProperties;
  className?: string;
  /** 外层样式，常用于外边距，如 { marginTop: 8, marginBottom: 4 } */
  style?: CSSProperties;
}

/**
 * 带数值显示的滑块。
 *
 * 例：`<RangeField label={t('blurNoise.blurRadius')} value={blur} onChange={setBlur} min={0} max={10} step={0.5}
 *   color="#60a5fa" format={v => v.toFixed(1)} ends={[t('blurNoise.blurMin'), t('blurNoise.blurMax')]} />`
 */
export default function RangeField({
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  color = 'var(--color-accent-primary)',
  format,
  ends,
  inline = false,
  valueStyle,
  className,
  style,
}: RangeFieldProps) {
  const inputId = useId();
  const shown = format ? format(value) : String(value);
  const slider = (
    <input
      id={inputId}
      type="range"
      className="ui-range-input"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={e => onChange(Number(e.target.value))}
      style={{ accentColor: color }}
    />
  );

  if (inline) {
    return (
      <div className={className ? `ui-range-inline ${className}` : 'ui-range-inline'} style={style}>
        <label htmlFor={inputId} className="ui-range-inline-label" style={{ color }}>{label}</label>
        {slider}
        <span className="ui-range-inline-value" style={{ color, ...valueStyle }}>{shown}</span>
      </div>
    );
  }

  return (
    <div className={className ? `form-group ${className}` : 'form-group'} style={style}>
      <label htmlFor={inputId} className="form-label ui-range-head">
        <span>{label}</span>
        <span className="ui-range-value" style={{ color, ...valueStyle }}>{shown}</span>
      </label>
      {slider}
      {ends && (
        <div className="ui-range-ends">
          <span>{ends[0]}</span>
          <span>{ends[1]}</span>
        </div>
      )}
    </div>
  );
}
