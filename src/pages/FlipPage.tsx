import { invoke } from '@tauri-apps/api/core';
import {
  FlipHorizontal2,
  FlipVertical2,
  RotateCcw,
} from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { FlipDirection, FlipOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function FlipPage() {
  const { t } = useTranslation();

  const flipOptions: { value: FlipDirection; label: string; icon: ReactNode }[] = [
    { value: 'horizontal', label: t('flip.horizontal'), icon: <FlipHorizontal2 /> },
    { value: 'vertical', label: t('flip.vertical'), icon: <FlipVertical2 /> },
    { value: 'both', label: t('flip.both'), icon: <RotateCcw /> },
  ];

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [direction, setDirection] = useState<FlipDirection>('horizontal');
  const task = useBatchTask({ event: 'flip-progress', taskId: 'flip' });

  const handleProcess = () => {
    const dirLabel = flipOptions.find((o) => o.value === direction)?.label ?? direction;
    task.run({
      taskName: t('flip.taskName'),
      startLog: t('pages.startMsg', { name: dirLabel }),
      exec: () => invoke<ProcessResult>('flip_images', {
        options: { input_path: inputPath, output_path: outputPath, direction, recursive } satisfies FlipOptions,
      }),
    });
  };

  return (
    <div className="page">
      <PageHeader icon={FlipHorizontal2} color="#00d4ff" title={t('flip.title')} subtitle={t('flip.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <PathFields
            allowFile
            input={inputPath} onInput={setInputPath}
            output={outputPath} onOutput={setOutputPath}
            recursive={recursive} onRecursive={setRecursive}
          />

          {/* 翻转选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('flip.flipDirection')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {flipOptions.map((opt) => {
                const selected = direction === opt.value;
                return (
                  <ChoiceCard key={opt.value} indicator="radio" selected={selected} onSelect={() => setDirection(opt.value)}>
                    <div style={{ width: 36, height: 36, borderRadius: 'var(--radius-sm)', minWidth: 36, background: selected ? 'rgba(0, 212, 255, 0.12)' : 'rgba(255,255,255,0.04)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: selected ? '#00d4ff' : 'var(--color-text-tertiary)' }}>
                      {opt.icon}
                    </div>
                    <div>
                      <div style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)', marginBottom: 2 }}>{opt.label}</div>
                    </div>
                  </ChoiceCard>
                );
              })}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_flip" />

          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
