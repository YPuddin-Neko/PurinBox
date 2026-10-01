import type { CSSProperties, ReactNode } from 'react';
import Checkbox from './Checkbox';

interface ExportBarProps {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
  label: string;
  disabled?: boolean;
  children: ReactNode;
  style?: CSSProperties;
}

export default function ExportBar({ enabled, onChange, label, disabled, children, style }: ExportBarProps) {
  return (
    <div style={{ flexShrink: 0, padding: '10px 16px', borderRadius: 'var(--radius-md)',
      border: '1px solid var(--color-border)', background: 'var(--color-bg-secondary)',
      display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 12, ...style }}>
      <Checkbox checked={enabled} onChange={onChange} disabled={disabled} label={label}
        style={{ fontSize: 12, fontWeight: 600 }} />
      {enabled && children}
    </div>
  );
}
