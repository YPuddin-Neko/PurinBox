import { useTranslation } from 'react-i18next';

export default function ScopeToggle({ value, onChange, hasCurrent }: { value: 'all' | 'current'; onChange: (value: 'all' | 'current') => void; hasCurrent: boolean }) {
  const { t } = useTranslation();
  return <fieldset style={{ border: 0, padding: 0, margin: '8px 0 12px' }}>
    <legend style={{ fontSize: 11, marginBottom: 6 }}>{t('tagManager.applyScope')}</legend>
    <div style={{ display: 'flex', gap: 14, flexWrap: 'wrap' }}>
      {(['all', 'current'] as const).map(scope => <label key={scope} style={{ display: 'flex', gap: 4, alignItems: 'center', fontSize: 11 }}>
        <input type="radio" checked={value === scope} disabled={scope === 'current' && !hasCurrent} onChange={() => onChange(scope)} />
        {t(scope === 'all' ? 'jsonTag.scopeAllImages' : 'jsonTag.scopeCurrentImage')}
      </label>)}
    </div>
  </fieldset>;
}
