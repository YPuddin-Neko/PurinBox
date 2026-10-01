import { invoke } from '@tauri-apps/api/core';
import {
  Move3D
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PerspectiveOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function PerspectivePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'perspective-progress', taskId: 'perspective' });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [intensity, setIntensity] = useState(0.10);

  const handleProcess = () => {

    return task.run({
      taskName: t('perspective.taskName'), startLog: t('perspective.startMsg', { intensity: intensity.toFixed(2) }), exec: () => invoke<ProcessResult>('perspective_transform', {
        options: { input_path: inputPath, output_path: outputPath, intensity, recursive } satisfies PerspectiveOptions,
      })
    });
  };

  return (
    <div className="page">
      <PageHeader icon={Move3D} color={'#f472b6'} title={t('perspective.title')} subtitle={t('perspective.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('perspective.paramSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <PathFields embedded allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <span>{t('perspective.intensity')}</span>
                  <span style={{ fontFamily: 'monospace', color: '#f472b6', fontSize: 'var(--font-size-sm)' }}>{intensity.toFixed(2)}</span>
                </label>
                <input type="range" min="0.02" max="0.30" step="0.01" value={intensity} onChange={(e) => setIntensity(Number(e.target.value))}
                  style={{ width: '100%', accentColor: '#f472b6' }} />
                <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                  <span>{t('perspective.intensityMin')}</span>
                  <span>{t('perspective.intensityMax')}</span>
                </div>
              </div>

            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_perspective" startText={t('perspective.startProcess')} />
          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
