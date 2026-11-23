import { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomSelect from './CustomSelect';
import { GBC_MODEL_OPTIONS, type GbcModels } from '../hooks/useGbcOptions';
import { GBC_DEFAULTS } from '../utils/gbcSettings';

type Limits = Pick<typeof GBC_DEFAULTS, 'max_nodes' | 'max_depth'>;
export function useGbcAnalysisLimits() {
  const [limits, setLimits] = useState<Limits>(() => {
    const defaults = { max_nodes: GBC_DEFAULTS.max_nodes, max_depth: GBC_DEFAULTS.max_depth };
    try {
      const saved = JSON.parse(localStorage.getItem('gbc_analysis_limits') || '{}');
      for (const [key, min, max] of [['max_nodes', 1, 64], ['max_depth', 0, 5]] as const) {
        if (Number.isInteger(saved[key]) && saved[key] >= min && saved[key] <= max) defaults[key] = saved[key];
      }
    } catch {}
    return defaults;
  });
  useEffect(() => { try { localStorage.setItem('gbc_analysis_limits', JSON.stringify(limits)); } catch {} }, [limits]);
  return [limits, setLimits] as const;
}

export default function GbcAnalysisFields({ models, value, onChange, disabled }: { models: GbcModels; value: Limits; onChange: (value: Limits) => void; disabled: boolean }) {
  const { t } = useTranslation();
  const id = useId();
  return <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: 'var(--space-3)' }}>
    {(['detector', 'embedder'] as const).map(role => <div key={role}>
      <label className="form-label" id={`${id}-${role}`}>{t(`gbc.${role}`)}</label>
      <CustomSelect labelledBy={`${id}-${role}`} disabled={disabled} value={models.value[role]} options={GBC_MODEL_OPTIONS[role]}
        onChange={model => models.onChange(role, model)} />
    </div>)}
    {(['max_nodes', 'max_depth'] as const).map(key => <div key={key}>
      <label className="form-label" htmlFor={`${id}-${key}`} title={t(`gbc.${key}Help`)} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>{t(`gbc.${key}`)}
          <span tabIndex={0} aria-label={t(`gbc.${key}Help`)} title={t(`gbc.${key}Help`)} style={{ cursor: 'help', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: '50%', fontSize: 9, fontWeight: 700, color: 'var(--color-text-tertiary)', border: '1px solid var(--color-border)' }}>?</span>
        </span>
        <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{value[key]}</span>
      </label>
      <input type="range" id={`${id}-${key}`} title={t(`gbc.${key}Help`)} disabled={disabled} step={1} min={key === 'max_nodes' ? 1 : 0} max={key === 'max_nodes' ? 64 : 5} value={value[key]} onChange={e => onChange({ ...value, [key]: Number(e.target.value) })} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
    </div>)}
  </div>;
}
