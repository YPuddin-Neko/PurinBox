import { invoke } from '@tauri-apps/api/core';
import {
  ArrowDownCircle,
  ArrowUpCircle,
  Info,
  Scaling
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ScaleOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function ScalePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'scale-progress', taskId: 'scale' });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [enableUpscale, setEnableUpscale] = useState(true);
  const [enableDownscale, setEnableDownscale] = useState(false);
  const [upWidth, setUpWidth] = useState(1024);
  const [upHeight, setUpHeight] = useState(1024);
  const [downWidth, setDownWidth] = useState(512);
  const [downHeight, setDownHeight] = useState(512);

  const mode = enableUpscale && enableDownscale ? 'both'
    : enableUpscale ? 'upscale'
      : enableDownscale ? 'downscale'
        : '';

  const handleProcess = () => {
    if (!mode) return;
    const targetW = mode === 'downscale' ? downWidth : upWidth;
    const targetH = mode === 'downscale' ? downHeight : upHeight;
    return task.run({
      taskName: t('scale.taskName'), startLog: t('scale.startMsg', { mode: { both: t('scale.startBoth'), upscale: t('scale.startUp'), downscale: t('scale.startDown') }[mode], width: targetW, height: targetH }), exec: () => invoke<ProcessResult>('scale_images', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode,
          target_width: targetW,
          target_height: targetH,
          down_target_width: mode === 'both' ? downWidth : 0,
          down_target_height: mode === 'both' ? downHeight : 0,
          recursive,
        } satisfies ScaleOptions,
      })
    });
  };

  const canStart = inputPath && outputPath && mode;

  return (
    <div className="page">
      <PageHeader icon={Scaling} color={'#818cf8'} title={t('scale.title')} subtitle={t('scale.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          {/* 缩放选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('scale.scaleOptions')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {/* 上采样 */}
              <ChoiceCard selected={enableUpscale} onSelect={() => setEnableUpscale(!enableUpscale)} indicator="check" body={<>
                <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', opacity: enableUpscale ? 1 : 0.4, pointerEvents: enableUpscale ? 'auto' : 'none' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.width')}</label>
                    <NumberInput className="form-input" value={upWidth} min={1} onChange={setUpWidth} fallback={1024} integer />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.height')}</label>
                    <NumberInput className="form-input" value={upHeight} min={1} onChange={setUpHeight} fallback={1024} integer />
                  </div>
                </div>

              </>}>

                <ArrowUpCircle style={{ width: 18, height: 18, color: enableUpscale ? '#4ade80' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{t('scale.upscale')}</span>
              </ChoiceCard>

              {/* 下采样 */}
              <ChoiceCard selected={enableDownscale} onSelect={() => setEnableDownscale(!enableDownscale)} indicator="check" body={<>
                <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', opacity: enableDownscale ? 1 : 0.4, pointerEvents: enableDownscale ? 'auto' : 'none' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.width')}</label>
                    <NumberInput className="form-input" value={downWidth} min={1} onChange={setDownWidth} fallback={512} integer />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.height')}</label>
                    <NumberInput className="form-input" value={downHeight} min={1} onChange={setDownHeight} fallback={512} integer />
                  </div>
                </div>

              </>}>

                <ArrowDownCircle style={{ width: 18, height: 18, color: enableDownscale ? '#60a5fa' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{t('scale.downscale')}</span>
              </ChoiceCard>

              {/* 提示文字 */}
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(251, 191, 36, 0.06)', border: '1px solid rgba(251, 191, 36, 0.15)' }}>
                <Info style={{ width: 14, height: 14, color: '#fbbf24', marginTop: 2, minWidth: 14 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  {t('scale.areaRule')}<br />{t('scale.resizeHint')}
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 执行按钮 */}
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!canStart}
            cancelCommand="cancel_scale" startText={t('scale.startScale')} />

          {/* 进度条和日志 */}
          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
