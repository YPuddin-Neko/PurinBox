import type { CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';

interface Props {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  style?: CSSProperties;
}

export default function RecursiveScanToggle({ checked, onChange, disabled = false, style }: Props) {
  const { t } = useTranslation();

  return (
    <label
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        fontSize: 12,
        color: 'var(--color-text-secondary)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        userSelect: 'none',
        whiteSpace: 'nowrap',
        flexShrink: 0,
        ...style,
      }}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        style={{ accentColor: 'var(--color-accent-primary)' }}
      />
      {t('pages.recursiveScan')}
    </label>
  );
}
