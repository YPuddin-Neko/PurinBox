import { invoke } from '@tauri-apps/api/core';
import {
  FlipHorizontal2,
  FlipVertical2,
  RotateCcw,
} from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { buildFlipOptions, FLIP_DEFAULTS, type FlipDirection } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import PathFields from '../components/ui/PathFields';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function FlipPage() {
  const { t } = useTranslation();

  const flipOptions: { value: FlipDirection; label: string; desc: string; icon: ReactNode }[] = [
    { value: 'horizontal', label: t('flip.horizontal'), desc: t('flip.horizontalDesc'), icon: <FlipHorizontal2 /> },
    { value: 'vertical', label: t('flip.vertical'), desc: t('flip.verticalDesc'), icon: <FlipVertical2 /> },
    { value: 'both', label: t('flip.both'), desc: t('flip.bothDesc'), icon: <RotateCcw /> },
  ];

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [direction, setDirection] = useState<FlipDirection>(FLIP_DEFAULTS.direction);
  const task = useBatchTask({ event: 'flip-progress', taskId: 'flip' });

  const handleProcess = () => {
    const dirLabel = flipOptions.find((o) => o.value === direction)?.label ?? direction;
    task.run({
      taskName: t('flip.taskName'),
      startLog: t('pages.startMsg', { name: dirLabel }),
      exec: () => invoke<ProcessResult>('flip_images', {
        options: buildFlipOptions({ input_path: inputPath, output_path: outputPath, recursive }, { direction }),
      }),
    });
  };

  return (
    <ToolPageLayout icon={FlipHorizontal2} color="#00d4ff" title={t('flip.title')} subtitle={t('flip.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath}
          cancelCommand="cancel_flip" />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields
        allowFile
        input={inputPath} onInput={setInputPath}
        output={outputPath} onOutput={setOutputPath}
        recursive={recursive} onRecursive={setRecursive}
      />

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
                  <div style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{opt.label}</div>
                  <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', marginTop: 2 }}>{opt.desc}</div>
                </div>
              </ChoiceCard>
            );
          })}
        </div>
      </div>
    </ToolPageLayout>
  );
}
