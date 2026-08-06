// 动态下拉选择控件 —— 从 Tauri 命令加载选项列表
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ParamDef } from '../workflowTypes';

interface Props {
  param: ParamDef;
  value: string;
  onChange: (value: string) => void;
  /** 加载完成后回传完整的原始数据列表 */
  onItemsLoaded?: (items: any[]) => void;
}

export default function DynamicSelect({ param, value, onChange, onItemsLoaded }: Props) {
  const [options, setOptions] = useState<{ value: string; label: string }[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!param.tauriListCommand) return;

    let cancelled = false;
    setLoading(true);

    invoke<any[]>(param.tauriListCommand)
      .then(list => {
        if (cancelled) return;
        const vKey = param.optionValueKey || 'id';
        const lKey = param.optionLabelKey || 'name';
        const filter = param.optionFilter;

        const filtered = filter
          ? list.filter(item => item[filter.key] === filter.value)
          : list;

        setOptions(filtered.map(item => ({
          value: String(item[vKey]),
          label: String(item[lKey]),
        })));
        setLoading(false);

        // 回传完整的过滤后列表
        onItemsLoaded?.(filtered);
      })
      .catch(() => {
        if (!cancelled) setLoading(false);
      });

    return () => { cancelled = true; };
  }, [param.tauriListCommand, param.optionValueKey, param.optionLabelKey]);

  if (loading) {
    return <select className="wf-inline-select" disabled><option>...</option></select>;
  }

  if (options.length === 0) {
    return <select className="wf-inline-select" disabled><option>—</option></select>;
  }

  return (
    <select
      className="wf-inline-select"
      value={value}
      onChange={e => onChange(e.target.value)}
    >
      {options.map(opt => (
        <option key={opt.value} value={opt.value}>{opt.label}</option>
      ))}
    </select>
  );
}
