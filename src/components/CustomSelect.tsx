import { useState, useRef, useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
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
  /** 允许手动输入自定义值 */
  allowCustom?: boolean;
}

export default function CustomSelect({
  value, options, onChange, placeholder, disabled = false, style, compact = false, allowCustom = false,
}: CustomSelectProps) {
  const { t } = useTranslation();
  const resolvedPlaceholder = placeholder || t('common.select');
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const selected = options.find(o => o.value === value);
  const [customInput, setCustomInput] = useState('');
  // 下拉经 portal 渲染到 body（fixed 定位），避免被 overflow 容器裁剪
  const [dropdownPos, setDropdownPos] = useState<{ top: number; left: number; width: number; flipUp: boolean } | null>(null);

  const updateDropdownPosition = useCallback(() => {
    if (!ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    const dropdownMaxH = 220; // 与 .cs-dropdown 的 max-height 一致
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const flipUp = spaceBelow < dropdownMaxH && rect.top > spaceBelow;
    setDropdownPos({
      top: flipUp ? rect.top - 4 : rect.bottom + 4,
      left: rect.left,
      width: rect.width,
      flipUp,
    });
  }, []);

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
      className={`cs-root ${open ? 'cs-open' : ''} ${disabled ? 'cs-disabled' : ''} ${compact ? 'cs-compact' : ''}`}
      style={style}
      tabIndex={disabled ? -1 : 0}
      onKeyDown={handleKeyDown}
    >
      <div className="cs-trigger" onClick={() => !disabled && setOpen(o => !o)}>
        <span className={`cs-value ${!selected && !value ? 'cs-placeholder' : ''}`}>
          {selected ? selected.label : (value || resolvedPlaceholder)}
        </span>
        <ChevronDown className={`cs-arrow ${open ? 'cs-arrow-open' : ''}`} />
      </div>

      {open && dropdownPos && createPortal(
        <div
          className={`cs-dropdown ${compact ? 'cs-compact' : ''}`}
          ref={listRef}
          style={{
            position: 'fixed',
            top: dropdownPos.top,
            left: dropdownPos.left,
            width: dropdownPos.width,
            right: 'auto',
            transform: dropdownPos.flipUp ? 'translateY(-100%)' : undefined,
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
          {options.length === 0 && !allowCustom && (
            <div className="cs-empty">{t('common.noOptions')}</div>
          )}
          {allowCustom && (
            <div style={{ borderTop: '1px solid var(--color-border)', padding: '4px 6px', display: 'flex', gap: 4 }}>
              <input className="form-input" placeholder={t('llmTagger.modelPlaceholder')}
                value={customInput} onChange={e => setCustomInput(e.target.value)}
                onClick={e => e.stopPropagation()}
                onKeyDown={e => { if (e.key === 'Enter' && customInput.trim()) { onChange(customInput.trim()); setCustomInput(''); setOpen(false); } e.stopPropagation(); }}
                style={{ flex: 1, fontSize: 11, height: 26 }} />
            </div>
          )}
        </div>,
        document.body,
      )}
    </div>
  );
}
