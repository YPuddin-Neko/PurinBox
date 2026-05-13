import { useState, useRef, useEffect, useCallback } from 'react';
import { ChevronDown, Check } from 'lucide-react';
import '../styles/custom-select.css';
import { useTranslation } from 'react-i18next';

export interface SelectOption {
  value: string;
  label: string;
}

interface CustomSelectProps {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  style?: React.CSSProperties;
  /** compact 模式：更小字号/高度 */
  compact?: boolean;
}

export default function CustomSelect({
  value, options, onChange, placeholder, disabled = false, style, compact = false,
}: CustomSelectProps) {
  const { t } = useTranslation();
  const resolvedPlaceholder = placeholder || t('common.select');
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const selected = options.find(o => o.value === value);

  // 点击外部关闭
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  // 打开时滚动到选中项
  useEffect(() => {
    if (open && listRef.current) {
      const active = listRef.current.querySelector('.cs-option.active');
      if (active) active.scrollIntoView({ block: 'nearest' });
    }
  }, [open]);

  const handleSelect = useCallback((val: string) => {
    onChange(val);
    setOpen(false);
  }, [onChange]);

  // 键盘支持
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (disabled) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      setOpen(o => !o);
    } else if (e.key === 'Escape') {
      setOpen(false);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const idx = options.findIndex(o => o.value === value);
      const next = e.key === 'ArrowDown'
        ? Math.min(idx + 1, options.length - 1)
        : Math.max(idx - 1, 0);
      onChange(options[next].value);
    }
  }, [disabled, options, value, onChange]);

  return (
    <div
      ref={ref}
      className={`cs-root ${open ? 'cs-open' : ''} ${disabled ? 'cs-disabled' : ''} ${compact ? 'cs-compact' : ''}`}
      style={style}
      tabIndex={disabled ? -1 : 0}
      onKeyDown={handleKeyDown}
    >
      <div className="cs-trigger" onClick={() => !disabled && setOpen(o => !o)}>
        <span className={`cs-value ${!selected ? 'cs-placeholder' : ''}`}>
          {selected ? selected.label : resolvedPlaceholder}
        </span>
        <ChevronDown className={`cs-arrow ${open ? 'cs-arrow-open' : ''}`} />
      </div>

      {open && (
        <div className="cs-dropdown" ref={listRef}>
          {options.map(opt => (
            <div
              key={opt.value}
              className={`cs-option ${opt.value === value ? 'active' : ''}`}
              onClick={() => handleSelect(opt.value)}
            >
              <span className="cs-option-label">{opt.label}</span>
              {opt.value === value && <Check className="cs-option-check" />}
            </div>
          ))}
          {options.length === 0 && (
            <div className="cs-empty">{t('common.noOptions')}</div>
          )}
        </div>
      )}
    </div>
  );
}
