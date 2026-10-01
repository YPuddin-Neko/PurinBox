import { useTranslation } from 'react-i18next';
import type { ParamDef } from '../workflowTypes';
import { useDynamicItems } from '../useDynamicItems';

interface Props {
  param: ParamDef;
  value: string;
  onChange: (value: string) => void;
  className?: string;
}

export default function DynamicSelect({ param, value, onChange, className = 'wf-inline-select' }: Props) {
  const { t } = useTranslation();
  const { items, loading } = useDynamicItems(param.tauriListCommand);
  const filtered = param.optionFilter ? items.filter(item => item[param.optionFilter!.key] === param.optionFilter!.value) : items;
  const options = filtered.map(item => ({ value: String(item[param.optionValueKey || 'id']), label: String(item[param.optionLabelKey || 'name']) }));
  return <select className={className} aria-label={t(param.labelKey)} value={value} disabled={loading || !options.length} onChange={e => onChange(e.target.value)}>
    {!options.some(option => option.value === value) && <option value={value}>{loading ? '...' : value || '-'}</option>}
    {options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
  </select>;
}
