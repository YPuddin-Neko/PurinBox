import type { CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';
import Checkbox from './Checkbox';

interface Props {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  style?: CSSProperties;
}

export default function RecursiveScanToggle({ checked, onChange, disabled = false, style }: Props) {
  const { t } = useTranslation();

  return (
    <Checkbox
      checked={checked}
      onChange={onChange}
      disabled={disabled}
      label={t('pages.recursiveScan')}
      size={14}
      style={{
        fontSize: 12,
        color: 'var(--color-text-secondary)',
        whiteSpace: 'nowrap',
        flexShrink: 0,
        ...style,
      }}
    />
  );
}
