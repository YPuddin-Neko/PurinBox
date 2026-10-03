import { useId } from 'react';
import { useTranslation } from 'react-i18next';

interface Props {
  dhash: number;
  onDhash: (value: number) => void;
  phash: number;
  onPhash: (value: number) => void;
  color: number;
  onColor: (value: number) => void;
}

export default function HashThresholdFields({ dhash, onDhash, phash, onPhash, color, onColor }: Props) {
  const { t } = useTranslation();
  const id = useId();
  const fields = [
    { key: 'dhashThreshold', value: dhash, onChange: onDhash, min: 1, max: 20 },
    { key: 'phashThreshold', value: phash, onChange: onPhash, min: 1, max: 20 },
    { key: 'colorThreshold', value: Math.round(color * 100), onChange: (v: number) => onColor(v / 100), min: 0, max: 100 },
  ];
  return <>{fields.map(field => (
    <div key={field.key} style={{ marginBottom: 12 }}>
      <label htmlFor={`${id}-${field.key}`} style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 12, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 6 }}>
        <span>{t(`imageDedup.${field.key}`)}</span>
        <span style={{ fontSize: 13, fontWeight: 700, color: '#7c5cfc', fontFamily: 'monospace' }}>{field.max === 100 ? color.toFixed(2) : field.value}</span>
      </label>
      <input id={`${id}-${field.key}`} type="range" min={field.min} max={field.max} value={field.value}
        onChange={e => field.onChange(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)' }}>
        <span>{t(`imageDedup.${field.max === 100 ? 'loose' : 'strict'}`)} ({field.min})</span>
        <span>{t(`imageDedup.${field.max === 100 ? 'strict' : 'loose'}`)} ({field.max === 100 ? '1.0' : field.max})</span>
      </div>
    </div>
  ))}</>;
}
