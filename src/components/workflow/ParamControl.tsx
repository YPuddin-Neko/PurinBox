import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen } from 'lucide-react';
import type { ParamDef } from './workflowTypes';
import DynamicSelect from './nodes/DynamicSelect';
import NumberInput from '../ui/NumberInput';

interface Props {
  param: ParamDef;
  value: any;
  onChange: (key: string, value: any) => void;
  disabled?: boolean;
  inline?: boolean;
}

export default function ParamControl({ param: p, value, onChange, disabled = false, inline = false }: Props) {
  const { t } = useTranslation();
  const className = inline ? 'wf-inline-input' : 'form-input';
  const label = t(p.labelKey);
  switch (p.type) {
    case 'path':
      return <div className={inline ? 'wf-inline-path' : 'wf-prop-path'}>
        <input className={className} aria-label={label} value={String(value)} onChange={e => onChange(p.key, e.target.value)} placeholder={t('workflow.selectFolder')} />
        <button className={inline ? 'wf-inline-path-btn' : 'btn btn-secondary'} title={t('workflow.selectFolder')} onClick={async e => {
          e.stopPropagation();
          const path = await open({ directory: true, multiple: false, title: t('workflow.selectFolder') });
          if (typeof path === 'string') onChange(p.key, path);
        }}><FolderOpen size={13} /></button>
      </div>;
    case 'number':
      return <NumberInput className={className} aria-label={label} value={Number(value)} min={p.min} max={p.max} step={p.step}
        integer={!p.step || Number.isInteger(p.step)} onChange={v => onChange(p.key, v)} />;
    case 'boolean':
      return <label className={inline ? `wf-inline-checkbox ${disabled ? 'wf-inline-checkbox-disabled' : ''}` : 'wf-prop-checkbox'}>
        <input type="checkbox" checked={!disabled && Boolean(value)} disabled={disabled} onChange={e => onChange(p.key, e.target.checked)} />
        <span className={inline ? 'wf-inline-checkbox-text' : undefined}>{label}</span>
      </label>;
    case 'select':
      return <select className={inline ? 'wf-inline-select' : className} aria-label={label} value={String(value)} onChange={e => onChange(p.key, e.target.value)}>
        {p.options?.map(option => <option key={option.value} value={option.value}>{t(option.labelKey)}</option>)}
      </select>;
    case 'dynamic-select':
      return <DynamicSelect param={p} value={String(value)} onChange={v => onChange(p.key, v)} className={inline ? 'wf-inline-select' : className} />;
    default:
      return <input className={className} aria-label={label} value={String(value)} onChange={e => onChange(p.key, e.target.value)} />;
  }
}
