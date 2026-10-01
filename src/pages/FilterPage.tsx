import { invoke } from '@tauri-apps/api/core';
import {
  Copy,
  Info,
  ScanSearch,
  Trash2
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { FilterOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

type ConditionType = 'min_width' | 'min_height' | 'below_resolution' | 'above_resolution';
type ActionType = 'copy' | 'delete';

export default function FilterPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'filter-progress', taskId: 'filter' });

  const conditionOptions: { value: ConditionType; label: string; desc: string }[] = [
    { value: 'min_width', label: t('filter.condMinWidth'), desc: t('filter.condMinWidthDesc') },
    { value: 'min_height', label: t('filter.condMinHeight'), desc: t('filter.condMinHeightDesc') },
    { value: 'below_resolution', label: t('filter.condBelowRes'), desc: t('filter.condBelowResDesc') },
    { value: 'above_resolution', label: t('filter.condAboveRes'), desc: t('filter.condAboveResDesc') },
  ];
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [action, setAction] = useState<ActionType>('copy');
  const [condition, setCondition] = useState<ConditionType>('below_resolution');
  const [width, setWidth] = useState(512);
  const [height, setHeight] = useState(512);

  const needsOutput = action === 'copy';
  const needsWidth = condition !== 'min_height';
  const needsHeight = condition !== 'min_width';

  const handleProcess = () => {

    return task.run({
      taskName: t('filter.taskName'), startLog: t('filter.startMsg', { condition: conditionOptions.find(c => c.value === condition)!.label, action: action === 'copy' ? t('filter.actionCopy') : t('filter.actionDelete') }), exec: () => invoke<ProcessResult>('filter_by_resolution', {
        options: { input_path: inputPath, output_path: outputPath || inputPath, action, condition, width: width, height: height, recursive } satisfies FilterOptions,
      })
    });
  };

  return (
    <div className="page">
      <PageHeader icon={ScanSearch} color={'#ff6b9d'} title={t('filter.title')} subtitle={t('filter.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <PathFields input={inputPath} onInput={setInputPath} output={needsOutput ? outputPath : undefined} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          {/* 操作方式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('filter.actionMode')}</span></div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-3)' }}>
              <ChoiceCard selected={action === 'copy'} onSelect={() => setAction('copy')}>
                <Copy style={{ width: 24, height: 24, color: action === 'copy' ? '#4ade80' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{t('filter.actionCopy')}</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{t('filter.actionCopyDesc')}</span>
              </ChoiceCard>
              <ChoiceCard selected={action === 'delete'} onSelect={() => setAction('delete')} tone="danger">
                <Trash2 style={{ width: 24, height: 24, color: action === 'delete' ? '#f87171' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{t('filter.actionDelete')}</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{t('filter.actionDeleteDesc')}</span>
              </ChoiceCard>
            </div>
            {action === 'delete' && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(248, 113, 113, 0.06)', border: '1px solid rgba(248, 113, 113, 0.15)', marginTop: 'var(--space-3)' }}>
                <Info style={{ width: 14, height: 14, color: '#f87171', minWidth: 14 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: '#f87171' }}>{t('filter.deleteWarning')}</span>
              </div>
            )}
          </div>

          {/* 筛选条件 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('filter.filterCondition')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {conditionOptions.map((opt) => (
                <ChoiceCard key={opt.value} selected={condition === opt.value} onSelect={() => setCondition(opt.value)} indicator="radio">

                  <div>
                    <div style={{ fontWeight: 600, fontSize: 'var(--font-size-base)', color: 'var(--color-text-primary)' }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </ChoiceCard>
              ))}
            </div>

            <div style={{ display: 'flex', gap: 'var(--space-3)', marginTop: 'var(--space-4)' }}>
              {needsWidth && (
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{condition === 'min_width' ? t('filter.minWidthPx') : t('filter.widthPx')}</label>
                  <NumberInput className="form-input" value={width} min={1} onChange={setWidth} fallback={512} integer />
                </div>
              )}
              {needsHeight && (
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{condition === 'min_height' ? t('filter.minHeightPx') : t('filter.heightPx')}</label>
                  <NumberInput className="form-input" value={height} min={1} onChange={setHeight} fallback={512} integer />
                </div>
              )}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || (action === 'copy' && !outputPath)}
            cancelCommand="cancel_filter"
            startText={action === 'delete' ? t('filter.startFilterDelete') : t('filter.startFilterOutput')} />

          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
