import { useTranslation } from 'react-i18next';
import Checkbox from './Checkbox';
import { TAGGER_CATEGORIES } from '../utils/taggerOptions';
import type { TaggerCategory } from '../api/commandOptions';
import '../styles/tags.css';

export function TaggerCategoryGrid({ enabled, onChange, supported }: { enabled: ReadonlySet<TaggerCategory>; onChange: (next: Set<TaggerCategory>) => void; supported?: readonly string[] }) {
  const { t } = useTranslation();
  return <div className="tagger-category-grid">
    {TAGGER_CATEGORIES.map(c => {
      const available = !supported || supported.includes(c.key);
      const checked = available && enabled.has(c.key);
      return <div key={c.key} className="tagger-category" data-checked={checked} data-disabled={!available}
        title={available ? undefined : t('hybridTagger.catUnsupported')}>
        <Checkbox checked={checked} disabled={!available} size={14}
          onChange={on => { const next = new Set(enabled); if (on) next.add(c.key); else next.delete(c.key); onChange(next); }}
          style={{ display: 'flex', padding: '6px 10px', minWidth: 0, fontSize: 12, fontWeight: 600, opacity: 1 }} label={t(c.labelKey)} />
      </div>;
    })}
  </div>;
}

export function ThresholdSliders({ general, character, onGeneral, onCharacter }: { general: number; character: number; onGeneral: (n: number) => void; onCharacter: (n: number) => void }) {
  const { t } = useTranslation();
  return <div style={{ display: 'flex', gap: 'var(--space-4)' }}>
    {([{ key: 'generalTh', value: general, change: onGeneral }, { key: 'charTh', value: character, change: onCharacter }]).map(s =>
      <label key={s.key} style={{ flex: 1, minWidth: 0, fontSize: 12 }}>
        <span className="form-label" style={{ display: 'flex', justifyContent: 'space-between', gap: 6, fontSize: 12, marginBottom: 4 }}>{t(`aiTagger.${s.key}`)}<span style={{ fontFamily: 'monospace', fontSize: 12, fontWeight: 700, color: '#f59e0b' }}>{s.value.toFixed(2)}</span></span>
        <input aria-label={t(`aiTagger.${s.key}`)} type="range" min="0.05" max="1" step="0.01" value={s.value} onChange={e => s.change(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
      </label>)}
  </div>;
}
