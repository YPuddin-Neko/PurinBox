import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import CustomSelect from './CustomSelect';
import { CUSTOM_FAMILY, groupTaggerModels, modelFamily, type TaggerModelLike } from '../utils/taggerModelGroups';

interface Props<T extends TaggerModelLike> {
  models: T[];
  value: string;
  onChange: (id: string) => void;
  /** 模型项显示文本（下载标记等不同页面不同的部分交给调用方） */
  formatLabel?: (m: T) => string;
}

/** 打标模型选择：左侧系列下拉（WD / PixAI / CL / 自定义），右侧该系列内的模型下拉，
 *  组内按版本新在前排序。切换系列时自动选中组内第一个（即最新版）模型。 */
export default function TaggerModelSelect<T extends TaggerModelLike>({ models, value, onChange, formatLabel }: Props<T>) {
  const { t } = useTranslation();
  const groups = useMemo(() => groupTaggerModels(models), [models]);
  const cur = models.find(m => m.id === value);
  const activeFamily = (cur && modelFamily(cur.name, cur.is_builtin)) || groups[0]?.family || '';
  const familyModels = groups.find(g => g.family === activeFamily)?.models ?? [];

  return (
    <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
      <CustomSelect
        value={activeFamily}
        onChange={f => {
          const first = groups.find(g => g.family === f)?.models[0];
          if (first && first.id !== value) onChange(first.id);
        }}
        options={groups.map(g => ({ value: g.family, label: g.family === CUSTOM_FAMILY ? t('aiTagger.customGroup') : g.family }))}
        style={{ width: 110, flexShrink: 0 }}
      />
      <CustomSelect
        value={value}
        onChange={onChange}
        options={familyModels.map(m => ({ value: m.id, label: formatLabel ? formatLabel(m) : m.name }))}
        style={{ flex: 1, minWidth: 0 }}
      />
    </div>
  );
}
