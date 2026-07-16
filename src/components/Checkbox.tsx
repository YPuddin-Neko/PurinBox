import { useId, type CSSProperties } from 'react';

interface Props {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
  color?: string;
  size?: number;
  style?: CSSProperties;
}

export default function Checkbox({
  checked,
  onChange,
  disabled = false,
  label,
  color = 'var(--color-accent-primary)',
  size = 16,
  style,
}: Props) {
  const id = useId();

  return (
    <label
      htmlFor={id}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        cursor: disabled ? 'not-allowed' : 'pointer',
        userSelect: 'none',
        opacity: disabled ? 0.5 : 1,
        ...style,
      }}
    >
      {/* 隐藏原生 checkbox */}
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        style={{ position: 'absolute', opacity: 0, width: 0, height: 0, pointerEvents: 'none' }}
      />
      {/* 自定义外观 */}
      <span
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          width: size,
          height: size,
          borderRadius: size * 0.25,
          border: `1.5px solid ${checked ? color : 'var(--color-text-tertiary)'}`,
          background: checked ? color : 'transparent',
          transition: 'all 0.2s cubic-bezier(0.4, 0, 0.2, 1)',
          flexShrink: 0,
          boxShadow: checked ? `0 0 0 2px ${color}25` : 'none',
        }}
      >
        {/* 勾选动画 SVG */}
        <svg
          viewBox="0 0 12 12"
          width={size * 0.6}
          height={size * 0.6}
          style={{
            opacity: checked ? 1 : 0,
            transform: checked ? 'scale(1)' : 'scale(0.5)',
            transition: 'all 0.2s cubic-bezier(0.4, 0, 0.2, 1)',
          }}
        >
          <polyline
            points="2.5 6.5 5 9 9.5 4"
            fill="none"
            stroke="#fff"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </span>
      {label && <span>{label}</span>}
    </label>
  );
}
