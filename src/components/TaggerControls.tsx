import { useTranslation } from 'react-i18next';
import Checkbox from './Checkbox';
import { TAGGER_CATEGORIES } from '../utils/taggerOptions';
import type { TaggerCategory } from '../api/commandOptions';

export function TaggerCategoryGrid({ enabled, onChange, supported }: { enabled: ReadonlySet<TaggerCategory>; onChange: (next: Set<TaggerCategory>) => void; supported?: readonly string[] }) {
  const { t } = useTranslation();
  return <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(85px, 1fr))', gap: 6, marginBottom: 'var(--space-3)' }}>
    {TAGGER_CATEGORIES.map(c => <span key={c.key} title={supported && !supported.includes(c.key) ? t('hybridTagger.catUnsupported') : undefined}><Checkbox checked={enabled.has(c.key)} disabled={!!supported && !supported.includes(c.key)}
      onChange={on => { const next = new Set(enabled); if (on) next.add(c.key); else next.delete(c.key); onChange(next); }}
      label={t(c.labelKey)} /></span>)}
  </div>;
}

export function ThresholdSliders({ general, character, onGeneral, onCharacter }: { general: number; character: number; onGeneral: (n: number) => void; onCharacter: (n: number) => void }) {
  const { t } = useTranslation();
  return <div style={{ display: 'flex', gap: 'var(--space-4)' }}>
    {([{ key: 'generalTh', value: general, change: onGeneral }, { key: 'charTh', value: character, change: onCharacter }]).map(s =>
      <label key={s.key} style={{ flex: 1, minWidth: 0, fontSize: 12 }}>
        <span style={{ display: 'flex', justifyContent: 'space-between', gap: 6 }}>{t(`aiTagger.${s.key}`)}<span style={{ fontFamily: 'monospace', color: '#f59e0b' }}>{s.value.toFixed(2)}</span></span>
        <input aria-label={t(`aiTagger.${s.key}`)} type="range" min="0.05" max="1" step="0.01" value={s.value} onChange={e => s.change(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
      </label>)}
  </div>;
}
