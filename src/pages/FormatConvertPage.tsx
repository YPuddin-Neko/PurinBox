import { invoke } from '@tauri-apps/api/core';
import { FileType, Info } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildFormatConvertOptions, FORMAT_CONVERT_DEFAULTS, type ConvertFormat } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import PathFields from '../components/ui/PathFields';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function FormatConvertPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'convert-progress', taskId: 'convert' });

  const targetFormats: { value: ConvertFormat; desc?: string; color: string }[] = [
    { value: 'png', color: '#4ade80' },
    { value: 'jpg', desc: t('formatConvert.jpgDesc'), color: '#ffa647' },
    { value: 'jpeg', desc: t('formatConvert.jpgDesc'), color: '#ffa647' },
    { value: 'bmp', desc: t('formatConvert.bmpDesc'), color: '#f87171' },
    { value: 'webp', color: '#60a5fa' },
  ];

  const sourceFormats = ['PNG', 'JPG', 'JPEG', 'WebP', 'BMP', 'TIFF', 'GIF', 'PSD'];

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [targetFormat, setTargetFormat] = useState<ConvertFormat>(FORMAT_CONVERT_DEFAULTS.target_format);

  const handleProcess = () => {
    return task.run({
      taskName: t('formatConvert.taskName'),
      startLog: t('formatConvert.startConvertMsg', { format: targetFormat }),
      exec: () => invoke<ProcessResult>('convert_format', {
        options: buildFormatConvertOptions({ input_path: inputPath, output_path: outputPath, recursive }, { target_format: targetFormat }),
      }),
    });
  };

  return (
    <ToolPageLayout icon={FileType} color="#ffa647" title={t('formatConvert.title')} subtitle={t('formatConvert.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath}
          cancelCommand="cancel_convert" startText={t('formatConvert.startConvert')} processingText={t('formatConvert.converting')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('formatConvert.targetFormat')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          {targetFormats.map((fmt) => (
            <ChoiceCard key={fmt.value} compact selected={targetFormat === fmt.value} onSelect={() => setTargetFormat(fmt.value)} indicator="radio">
              <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: targetFormat === fmt.value ? fmt.color : 'var(--color-text-tertiary)', minWidth: 50 }}>.{fmt.value}</span>
              {fmt.desc && <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{fmt.desc}</span>}
            </ChoiceCard>
          ))}
        </div>
      </div>

      {/* 支持的源格式 */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(255, 166, 71, 0.04)', border: '1px solid rgba(255, 166, 71, 0.1)' }}>
        <Info style={{ width: 18, height: 18, color: '#ffa647', marginTop: 2, minWidth: 18 }} />
        <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
          <strong style={{ color: 'var(--color-text-primary)' }}>{t('formatConvert.supportedFormats')}</strong>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 4 }}>
            {sourceFormats.map((f) => (
              <span key={f} style={{ fontSize: 'var(--font-size-xs)', padding: '1px 8px', borderRadius: 'var(--radius-full)', background: 'rgba(255, 255, 255, 0.04)', border: '1px solid var(--color-border)', color: 'var(--color-text-secondary)' }}>{f}</span>
            ))}
          </div>
        </div>
      </div>
    </ToolPageLayout>
  );
}
