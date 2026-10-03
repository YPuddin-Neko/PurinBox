import { useId } from 'react';
import { useTranslation } from 'react-i18next';

export default function ScopeToggle({ value, onChange, hasCurrent, appearance = 'radio' }: { value: 'all' | 'current'; onChange: (value: 'all' | 'current') => void; hasCurrent: boolean; appearance?: 'radio' | 'chips' }) {
  const { t } = useTranslation();
  const groupName = useId();
  return <fieldset style={{ border: 0, padding: 0, margin: '8px 0 12px' }}>
    <legend style={{ fontSize: 11, marginBottom: 6 }}>{t('tagManager.applyScope')}</legend>
    <div style={{ display: 'flex', gap: appearance === 'chips' ? 4 : 14, flexWrap: 'wrap' }}>
      {(['all', 'current'] as const).map(scope => <label key={scope} className={appearance === 'chips' ? 'tag-option' : undefined} style={appearance === 'chips' ? { '--option-color': '#60a5fa' } as React.CSSProperties : { display: 'flex', gap: 4, alignItems: 'center', fontSize: 11 }}>
        <input type="radio" name={groupName} checked={value === scope} disabled={scope === 'current' && !hasCurrent} onChange={() => onChange(scope)} />
        {t(scope === 'all' ? 'jsonTag.scopeAllImages' : 'jsonTag.scopeCurrentImage')}
      </label>)}
    </div>
  </fieldset>;
}
