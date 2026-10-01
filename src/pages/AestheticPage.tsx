import { invoke } from '@tauri-apps/api/core';
import { Star } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AestheticOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';
import { listen } from '../utils/tauriRuntime';

interface DownloadPayload { percent: number; speed_mbps: number; status: string; message: string; }

export default function AestheticPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'aesthetic-progress', taskId: 'aesthetic', pythonEnv: true });
  const downloadActive = useRef(false);

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const isMac = /Mac|darwin/i.test(navigator.userAgent);
  const [useGpu, setUseGpu] = useState(!isMac);
  const [batchSize, setBatchSize] = useState(1);

  useEffect(() => {
    let active = true;
    const unlisten = listen<DownloadPayload>('aesthetic-download', (e) => {
      if (!active || !downloadActive.current) return;
      task.logger.appendDownloadLog(e.payload);
    });
    return () => { active = false; unlisten.then(fn => fn()); };
  }, [task.logger]);

  const handleProcess = () => {

    return task.run({
      taskName: t('aesthetic.taskName'), startLog: t('aesthetic.starting'), exec: async () => {
        downloadActive.current = true; try {
          return await invoke<ProcessResult>('start_aesthetic_scoring', {
            options: {
              input_path: inputPath,
              output_path: outputPath,
              use_gpu: useGpu,
              batch_size: useGpu ? batchSize : 1,
              recursive,
            } satisfies AestheticOptions
          });
        } finally { downloadActive.current = false; }
      }
    });
  };

  return (
    <div className="page">
      <PageHeader icon={Star} color={'#facc15'} title={t('aesthetic.title')} subtitle={t('aesthetic.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          {/* 路径设置 */}
          <PathFields allowFile title={t('pages.pathSettings')} outputLabel={t('aesthetic.outputPath')} headerExtra={<>{!isMac && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <span style={{ fontSize: 11, fontWeight: 600, color: useGpu ? 'var(--color-text-secondary)' : 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{t('aesthetic.batchSize')}</span>
              <NumberInput className="form-input" min={1} max={64} value={batchSize} disabled={!useGpu} style={{ width: 58, padding: '3px 6px', fontSize: 11, textAlign: 'center', opacity: useGpu ? 1 : 0.4 }} onChange={setBatchSize} fallback={1} integer />
              <div style={{ width: 1, height: 16, background: 'var(--color-border)', margin: '0 4px' }} />
              <DeviceToggle useGpu={useGpu} onChange={setUseGpu} />
            </div>
          )}</>} input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive}><div style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--color-text-tertiary)', lineHeight: 1.6 }}>
              {t('aesthetic.description')}：
              {['masterpiece', 'best', 'great', 'good', 'normal', 'low', 'worst'].map((l, i) => (
                <span key={l} style={{ fontWeight: 600, color: ['#facc15', '#a78bfa', '#34d399', '#60a5fa', '#94a3b8', '#fb923c', '#f87171'][i] }}>
                  {l}{i < 6 ? '、' : ''}
                </span>
              ))}
            </div>
            <div style={{ marginTop: 6, fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>
              {t('aesthetic.modelSource')} <a href="https://huggingface.co/deepghs/anime_aesthetic" target="_blank" rel="noreferrer" style={{ color: '#818cf8' }}>deepghs/anime_aesthetic</a>
            </div></PathFields>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath}
            cancelCommand="cancel_aesthetic_scoring" forceCancelCommand="force_cancel_aesthetic_scoring"
            startText={t('aesthetic.start')}
            processingText={t('aesthetic.processing')} />

          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
