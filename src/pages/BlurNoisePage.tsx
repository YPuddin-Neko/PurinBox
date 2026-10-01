import { invoke } from '@tauri-apps/api/core';
import {
  Sparkles
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { BlurNoiseOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function BlurNoisePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'blur-noise-progress', taskId: 'blur-noise' });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [blurRadius, setBlurRadius] = useState(2.0);
  const [noiseStrength, setNoiseStrength] = useState(15);

  const handleProcess = () => {

    return task.run({
      taskName: t('blurNoise.taskName'), startLog: t('blurNoise.startMsg', { blur: blurRadius.toFixed(1), noise: noiseStrength }), exec: () => invoke<ProcessResult>('blur_noise_images', {
        options: { input_path: inputPath, output_path: outputPath, blur_radius: blurRadius, noise_strength: noiseStrength, recursive } satisfies BlurNoiseOptions,
      })
    });
  };

  return (
    <div className="page">
      <PageHeader icon={Sparkles} color={'#60a5fa'} title={t('blurNoise.title')} subtitle={t('blurNoise.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('blurNoise.paramSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <PathFields embedded allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)' }}>
                <div className="form-group" style={{ marginBottom: 0 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>{t('blurNoise.blurRadius')}</span>
                    <span style={{ fontFamily: 'monospace', color: '#60a5fa', fontSize: 'var(--font-size-sm)' }}>{blurRadius.toFixed(1)}</span>
                  </label>
                  <input type="range" min="0" max="10" step="0.5" value={blurRadius} onChange={(e) => setBlurRadius(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#60a5fa' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    <span>{t('blurNoise.blurMin')}</span>
                    <span>{t('blurNoise.blurMax')}</span>
                  </div>
                </div>

                <div className="form-group" style={{ marginBottom: 0 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>{t('blurNoise.noiseStrength')}</span>
                    <span style={{ fontFamily: 'monospace', color: '#a78bfa', fontSize: 'var(--font-size-sm)' }}>{noiseStrength}</span>
                  </label>
                  <input type="range" min="0" max="100" step="1" value={noiseStrength} onChange={(e) => setNoiseStrength(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#a78bfa' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    <span>{t('blurNoise.noiseMin')}</span>
                    <span>{t('blurNoise.noiseMax')}</span>
                  </div>
                </div>
              </div>

            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath || (blurRadius <= 0 && noiseStrength <= 0)}
            cancelCommand="cancel_blur_noise" startText={t('blurNoise.startProcess')} />
          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
