import { useState, useRef, useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown, Check } from 'lucide-react';
import '../styles/custom-select.css';
import { useTranslation } from 'react-i18next';

interface SelectOption {
  value: string;
  label: string;
}

interface CustomSelectProps {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  style?: React.CSSProperties;
  /** compact 模式：更小字号/高度 */
  compact?: boolean;
  disabled?: boolean;
  labelledBy?: string;
}

export default function CustomSelect({
  value, options, onChange, style, compact = false, disabled = false, labelledBy,
}: CustomSelectProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const selected = options.find(o => o.value === value);
  // 下拉经 portal 渲染到 body（fixed 定位），避免被 overflow 容器裁剪
  const [dropdownPos, setDropdownPos] = useState<{ top?: number; bottom?: number; left: number; width: number; flipUp: boolean } | null>(null);

  const updateDropdownPosition = useCallback(() => {
    if (!ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    const dropdownMaxH = Math.min(220, Math.max(options.length, 1) * (compact ? 30 : 34) + 12);
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const flipUp = spaceBelow < dropdownMaxH && rect.top > spaceBelow;
    setDropdownPos({
      top: flipUp ? undefined : rect.bottom + 4,
      bottom: flipUp ? window.innerHeight - rect.top + 4 : undefined,
      left: rect.left,
      width: rect.width,
      flipUp,
    });
  }, [options.length, compact]);

  // 打开时计算位置，并跟随滚动/缩放更新
  useEffect(() => {
    if (!open) return;
    updateDropdownPosition();
    window.addEventListener('scroll', updateDropdownPosition, true);
    window.addEventListener('resize', updateDropdownPosition);
    return () => {
      window.removeEventListener('scroll', updateDropdownPosition, true);
      window.removeEventListener('resize', updateDropdownPosition);
    };
  }, [open, updateDropdownPosition]);

  // 点击外部关闭（下拉已 portal 到 body，需同时排除下拉自身）
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (ref.current?.contains(target)) return;
      if (listRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

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

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (disabled) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      setOpen(o => !o);
    } else if (e.key === 'Escape') {
      setOpen(false);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (options.length === 0) return; // 空列表按方向键会取 options[-1] 抛异常
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
      className={`cs-root ${open ? 'cs-open' : ''} ${compact ? 'cs-compact' : ''}`}
      style={style}
      role="button"
      aria-labelledby={labelledBy}
      aria-expanded={open && !disabled}
      aria-disabled={disabled || undefined}
      tabIndex={disabled ? -1 : 0}
      onKeyDown={handleKeyDown}
    >
      <div className="cs-trigger" onClick={() => { if (!disabled) setOpen(o => !o); }}>
        <span className={`cs-value ${!selected && !value ? 'cs-placeholder' : ''}`}>
          {selected ? selected.label : (value || t('common.select'))}
        </span>
        <ChevronDown className={`cs-arrow ${open ? 'cs-arrow-open' : ''}`} />
      </div>

      {open && !disabled && dropdownPos && createPortal(
        <div
          className={`cs-dropdown ${compact ? 'cs-compact' : ''} ${dropdownPos.flipUp ? 'cs-flip-up' : ''}`}
          ref={listRef}
          style={{
            position: 'fixed',
            top: dropdownPos.top,
            bottom: dropdownPos.bottom,
            left: dropdownPos.left,
            width: dropdownPos.width,
            right: 'auto',
          }}
        >
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
        </div>,
        document.body,
      )}
    </div>
  );
}
