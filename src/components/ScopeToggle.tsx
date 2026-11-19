import { useId, type CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';
import { OptionChips } from './TagEditorLayout';

export type BatchScope = 'all' | 'current' | 'filtered';

/** 批量操作的应用范围：全部图片、当前图片或筛选图片 */
export default function ScopeToggle({ value, onChange, hasCurrent, hasFiltered, color = '#60a5fa', appearance = 'chips', style }: {
  value: BatchScope;
  onChange: (value: BatchScope) => void;
  hasCurrent: boolean;
  hasFiltered: boolean;
  /** 选中项的颜色：添加类弹窗为蓝色，删除类弹窗传红色 */
  color?: string;
  appearance?: 'radio' | 'chips';
  style?: CSSProperties;
}) {
  const { t } = useTranslation();
  const name = useId();
  const options = [
    { value: 'all', label: t('tagEditor.scopeAllImages'), disabled: false },
    { value: 'current', label: t('tagEditor.scopeCurrentImage'), disabled: !hasCurrent },
    { value: 'filtered', label: t('tagEditor.scopeFilteredImages'), disabled: !hasFiltered },
  ] as const;
  if (appearance === 'radio') {
    return (
      <fieldset style={{ border: 0, padding: 0, margin: '0 0 12px', ...style }}>
        <legend style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4 }}>{t('tagEditor.applyScope')}</legend>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12 }}>
          {options.map(option => (
            <label key={option.value} style={{ fontSize: 11, color: option.disabled ? 'var(--color-text-tertiary)' : 'var(--color-text-secondary)',
              display: 'flex', alignItems: 'center', gap: 4, cursor: option.disabled ? 'not-allowed' : 'pointer' }}>
              <input type="radio" name={name} checked={value === option.value} disabled={option.disabled} onChange={() => onChange(option.value)} />
              {option.label}
            </label>
          ))}
        </div>
      </fieldset>
    );
  }
  return <OptionChips legend={t('tagEditor.applyScope')} value={value} onChange={onChange} color={color} options={options} />;
}
