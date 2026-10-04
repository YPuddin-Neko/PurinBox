import { invoke } from '@tauri-apps/api/core';
import { Layers } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ALPHA_CONVERT_DEFAULTS, buildAlphaConvertOptions, type AlphaBackground } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import PathFields from '../components/ui/PathFields';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function AlphaConvertPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'alpha-progress', taskId: 'alpha' });

  const bgOptions: { value: AlphaBackground; label: string }[] = [
    { value: 'white', label: t('alphaConvert.whiteBg') },
    { value: 'black', label: t('alphaConvert.blackBg') },
  ];
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [background, setBackground] = useState<AlphaBackground>(ALPHA_CONVERT_DEFAULTS.background);

  const handleProcess = () => {
    return task.run({
      taskName: t('alphaConvert.taskName'),
      startLog: t('alphaConvert.startMsg', { bg: background === 'white' ? t('alphaConvert.bgWhite') : t('alphaConvert.bgBlack') }),
      exec: () => invoke<ProcessResult>('convert_alpha', {
        options: buildAlphaConvertOptions({ input_path: inputPath, output_path: outputPath, recursive }, { background }),
      }),
    });
  };

  return (
    <ToolPageLayout icon={Layers} color="#c084fc" title={t('alphaConvert.title')} subtitle={t('alphaConvert.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath}
          cancelCommand="cancel_alpha" startText={t('alphaConvert.startConvert')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('alphaConvert.fillArea')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          {bgOptions.map((opt) => (
            <ChoiceCard key={opt.value} selected={background === opt.value} onSelect={() => setBackground(opt.value)} indicator="radio">
              <div style={{
                width: 28, height: 28, borderRadius: 'var(--radius-sm)', minWidth: 28,
                background: opt.value === 'white' ? '#ffffff' : '#1a1a1a',
                border: '1px solid var(--color-border)',
              }} />
              <div style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{opt.label}</div>
            </ChoiceCard>
          ))}
        </div>
      </div>
    </ToolPageLayout>
  );
}
