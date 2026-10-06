import { useId, type CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';
import { OptionChips } from './TagEditorLayout';

/** 批量操作的应用范围：全部图片或当前图片 */
export default function ScopeToggle({ value, onChange, hasCurrent, color = '#60a5fa', appearance = 'chips', style }: {
  value: 'all' | 'current';
  onChange: (value: 'all' | 'current') => void;
  hasCurrent: boolean;
  /** 选中项的颜色：添加类弹窗为蓝色，删除类弹窗传红色 */
  color?: string;
  appearance?: 'radio' | 'chips';
  style?: CSSProperties;
}) {
  const { t } = useTranslation();
  const name = useId();
  if (appearance === 'radio') {
    return (
      <fieldset style={{ border: 0, padding: 0, margin: '0 0 12px', ...style }}>
        <legend style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4 }}>{t('tagEditor.applyScope')}</legend>
        <div style={{ display: 'flex', gap: 12 }}>
          {(['all', 'current'] as const).map(scope => {
            const disabled = scope === 'current' && !hasCurrent;
            return <label key={scope} style={{ fontSize: 11, color: disabled ? 'var(--color-text-tertiary)' : 'var(--color-text-secondary)',
              display: 'flex', alignItems: 'center', gap: 4, cursor: disabled ? 'not-allowed' : 'pointer' }}>
              <input type="radio" name={name} checked={value === scope} disabled={disabled} onChange={() => onChange(scope)} />
              {t(scope === 'all' ? 'tagEditor.scopeAllImages' : 'tagEditor.scopeCurrentImage')}
            </label>;
          })}
        </div>
      </fieldset>
    );
  }
  return (
    <OptionChips legend={t('tagEditor.applyScope')} value={value} onChange={onChange} color={color} options={[
      { value: 'all', label: t('tagEditor.scopeAllImages') },
      { value: 'current', label: t('tagEditor.scopeCurrentImage'), disabled: !hasCurrent },
    ]} />
  );
}
