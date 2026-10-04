import { invoke } from '@tauri-apps/api/core';
import { Move3D } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildPerspectiveOptions, PERSPECTIVE_DEFAULTS } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import PathFields from '../components/ui/PathFields';
import RangeField from '../components/ui/RangeField';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function PerspectivePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'perspective-progress', taskId: 'perspective' });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [intensity, setIntensity] = useState(PERSPECTIVE_DEFAULTS.intensity);

  const handleProcess = () => {
    return task.run({
      taskName: t('perspective.taskName'),
      startLog: t('perspective.startMsg', { intensity: intensity.toFixed(2) }),
      exec: () => invoke<ProcessResult>('perspective_transform', {
        options: buildPerspectiveOptions({ input_path: inputPath, output_path: outputPath, recursive }, { intensity }),
      }),
    });
  };

  return (
    <ToolPageLayout icon={Move3D} color="#f472b6" title={t('perspective.title')} subtitle={t('perspective.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath}
          cancelCommand="cancel_perspective" startText={t('perspective.startProcess')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('perspective.paramSettings')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <PathFields embedded allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          <RangeField label={t('perspective.intensity')} value={intensity} onChange={setIntensity}
            min={0.02} max={0.30} step={0.01} color="#f472b6" format={v => v.toFixed(2)}
            ends={[t('perspective.intensityMin'), t('perspective.intensityMax')]} />
        </div>
      </div>
    </ToolPageLayout>
  );
}
