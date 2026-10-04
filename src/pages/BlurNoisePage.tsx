import { invoke } from '@tauri-apps/api/core';
import { Sparkles } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { BLUR_NOISE_DEFAULTS, buildBlurNoiseOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import PathFields from '../components/ui/PathFields';
import RangeField from '../components/ui/RangeField';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function BlurNoisePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'blur-noise-progress', taskId: 'blur-noise' });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [blurRadius, setBlurRadius] = useState(BLUR_NOISE_DEFAULTS.blur_radius);
  const [noiseStrength, setNoiseStrength] = useState(BLUR_NOISE_DEFAULTS.noise_strength);

  const handleProcess = () => {
    return task.run({
      taskName: t('blurNoise.taskName'),
      startLog: t('blurNoise.startMsg', { blur: blurRadius.toFixed(1), noise: noiseStrength }),
      exec: () => invoke<ProcessResult>('blur_noise_images', {
        options: buildBlurNoiseOptions({ input_path: inputPath, output_path: outputPath, recursive }, { blur_radius: blurRadius, noise_strength: noiseStrength }),
      }),
    });
  };

  return (
    <ToolPageLayout icon={Sparkles} color="#60a5fa" title={t('blurNoise.title')} subtitle={t('blurNoise.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath || (blurRadius <= 0 && noiseStrength <= 0)}
          cancelCommand="cancel_blur_noise" startText={t('blurNoise.startProcess')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('blurNoise.paramSettings')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <PathFields embedded allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)' }}>
            <RangeField label={t('blurNoise.blurRadius')} value={blurRadius} onChange={setBlurRadius}
              min={0} max={10} step={0.5} color="#60a5fa" format={v => v.toFixed(1)}
              ends={[t('blurNoise.blurMin'), t('blurNoise.blurMax')]} />
            <RangeField label={t('blurNoise.noiseStrength')} value={noiseStrength} onChange={setNoiseStrength}
              min={0} max={100} color="#a78bfa"
              ends={[t('blurNoise.noiseMin'), t('blurNoise.noiseMax')]} />
          </div>
        </div>
      </div>
    </ToolPageLayout>
  );
}
