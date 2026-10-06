import { invoke } from '@tauri-apps/api/core';
import { Star } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AESTHETIC_DEFAULTS, buildAestheticOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PathFields from '../components/ui/PathFields';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

const GRADES = [
  { label: 'masterpiece', color: '#facc15' },
  { label: 'best', color: '#a78bfa' },
  { label: 'great', color: '#34d399' },
  { label: 'good', color: '#60a5fa' },
  { label: 'normal', color: '#94a3b8' },
  { label: 'low', color: '#fb923c' },
  { label: 'worst', color: '#f87171' },
];

export default function AestheticPage() {
  const { t } = useTranslation();
  const task = useBatchTask({
    event: 'aesthetic-progress', taskId: 'aesthetic', pythonEnv: true,
    download: { event: 'aesthetic-download', appendDone: false },
  });

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const isMac = /Mac|darwin/i.test(navigator.userAgent);
  const [useGpu, setUseGpu] = useState(!isMac && AESTHETIC_DEFAULTS.use_gpu);
  const [batchSize, setBatchSize] = useState(AESTHETIC_DEFAULTS.batch_size);

  const handleProcess = () => {
    return task.run({
      taskName: t('aesthetic.taskName'),
      startLog: t('aesthetic.starting'),
      exec: () => invoke<ProcessResult>('start_aesthetic_scoring', {
        options: buildAestheticOptions({ input_path: inputPath, output_path: outputPath, recursive }, { use_gpu: useGpu, batch_size: batchSize }),
      }),
    });
  };

  return (
    <ToolPageLayout icon={Star} color="#facc15" title={t('aesthetic.title')} subtitle={t('aesthetic.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath}
          cancelCommand="cancel_aesthetic_scoring" forceCancelCommand="force_cancel_aesthetic_scoring"
          startText={t('aesthetic.start')}
          processingText={t('aesthetic.processing')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields allowFile outputLabel={t('aesthetic.outputPath')}
        input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive}
        headerExtra={!isMac && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
            <span style={{ fontSize: 11, fontWeight: 600, color: useGpu ? 'var(--color-text-secondary)' : 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{t('aesthetic.batchSize')}</span>
            <NumberInput className="form-input" min={1} max={64} value={batchSize} disabled={!useGpu} style={{ width: 58, padding: '3px 6px', fontSize: 11, textAlign: 'center', opacity: useGpu ? 1 : 0.4 }} onChange={setBatchSize} fallback={AESTHETIC_DEFAULTS.batch_size} integer />
            <div style={{ width: 1, height: 16, background: 'var(--color-border)', margin: '0 4px' }} />
            <DeviceToggle useGpu={useGpu} onChange={setUseGpu} />
          </div>
        )} footer={<>
        <div style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--color-text-tertiary)', lineHeight: 1.6 }}>
          {t('aesthetic.description')}：
          {GRADES.map((grade, i) => (
            <span key={grade.label} style={{ fontWeight: 600, color: grade.color }}>
              {grade.label}{i < GRADES.length - 1 ? '、' : ''}
            </span>
          ))}
        </div>
        <div style={{ marginTop: 6, fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>
          {t('aesthetic.modelSource')} <a href="https://huggingface.co/deepghs/anime_aesthetic" target="_blank" rel="noreferrer" style={{ color: '#818cf8' }}>deepghs/anime_aesthetic</a>
        </div>
      </>} />
    </ToolPageLayout>
  );
}
