import { useEffect, useState, type InputHTMLAttributes } from 'react';

type NativeInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'type' | 'value' | 'defaultValue' | 'onChange' | 'min' | 'max'
>;

export interface NumberInputProps extends NativeInputProps {
  /** 始终是合法数字 */
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  /** 草稿为空或不是数字时提交的值；不传则回到当前值 */
  fallback?: number;
  /** 只接受整数，提交时四舍五入 */
  integer?: boolean;
}

function parseDraft(draft: string): number | null {
  if (draft.trim() === '') return null;
  const parsed = Number(draft);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * 数字输入框：输入过程中保存草稿字符串，草稿是范围内的合法数字时立即同步给 onChange；
 * 失焦或回车时按 fallback / integer / min / max 规范后提交，父组件的 state 不会出现空串或越界值。
 */
export default function NumberInput({
  value,
  onChange,
  min,
  max,
  fallback,
  integer = false,
  className = 'form-input',
  onBlur,
  onKeyDown,
  ...rest
}: NumberInputProps) {
  const [draft, setDraft] = useState(() => String(value));

  // 外部改值（预设按钮、读取配置等）时同步草稿；草稿已表示该值时保持原样，不打断输入
  useEffect(() => {
    setDraft(prev => (parseDraft(prev) === value ? prev : String(value)));
  }, [value]);

  const accepts = (n: number) =>
    (min === undefined || n >= min)
    && (max === undefined || n <= max)
    && (!integer || Number.isInteger(n));

  const normalize = (n: number | null): number => {
    let next = n ?? fallback ?? value;
    if (integer) next = Math.round(next);
    if (min !== undefined) next = Math.max(min, next);
    if (max !== undefined) next = Math.min(max, next);
    return next;
  };

  const commit = () => {
    const next = normalize(parseDraft(draft));
    setDraft(String(next));
    if (next !== value) onChange(next);
  };

  return (
    <input
      {...rest}
      type="number"
      className={className}
      value={draft}
      min={min}
      max={max}
      onChange={e => {
        const text = e.target.value;
        setDraft(text);
        const parsed = parseDraft(text);
        if (parsed !== null && accepts(parsed) && parsed !== value) onChange(parsed);
      }}
      onBlur={e => {
        commit();
        onBlur?.(e);
      }}
      onKeyDown={e => {
        if (e.key === 'Enter' && !e.nativeEvent.isComposing) commit();
        onKeyDown?.(e);
      }}
    />
  );
}
