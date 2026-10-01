import type { CSSProperties } from 'react';
import './ui.css';

export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  /** md 36×20（默认），sm 30×16 */
  size?: 'sm' | 'md';
  disabled?: boolean;
  /** 开启时的轨道颜色，默认主题色 */
  color?: string;
  /** 关闭时的轨道颜色，默认边框色 */
  offColor?: string;
  /** 配合外部 <label htmlFor> 使用 */
  id?: string;
  title?: string;
  'aria-label'?: string;
  'aria-labelledby'?: string;
  className?: string;
  style?: CSSProperties;
}

/**
 * 开关：button role="switch"，空格/回车切换。
 * 需要整块可点时把它放进 <label>（点文字也会切换，且只触发一次）。
 */
export default function Switch({
  checked,
  onChange,
  size = 'md',
  disabled = false,
  color,
  offColor,
  className,
  style,
  ...attrs
}: SwitchProps) {
  const track = checked ? color : offColor;
  return (
    <button
      {...attrs}
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      className={`ui-switch ui-switch-${size}${checked ? ' is-on' : ''}${className ? ` ${className}` : ''}`}
      style={track === undefined ? style : { ...style, background: track }}
      onClick={() => onChange(!checked)}
    >
      <span className="ui-switch-knob" />
    </button>
  );
}
