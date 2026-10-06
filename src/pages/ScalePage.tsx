import { invoke } from '@tauri-apps/api/core';
import {
  ArrowDownCircle,
  ArrowUpCircle,
  Info,
  Scaling
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildScaleOptions, SCALE_DEFAULTS, type ScaleMode } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import NumberInput from '../components/ui/NumberInput';
import PathFields from '../components/ui/PathFields';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

export default function ScalePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'scale-progress', taskId: 'scale' });
  const d = SCALE_DEFAULTS;
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [enableUpscale, setEnableUpscale] = useState(true);
  const [enableDownscale, setEnableDownscale] = useState(false);
  const [upWidth, setUpWidth] = useState(d.upscale_width);
  const [upHeight, setUpHeight] = useState(d.upscale_height);
  const [downWidth, setDownWidth] = useState(d.downscale_width);
  const [downHeight, setDownHeight] = useState(d.downscale_height);

  const mode: ScaleMode | null = enableUpscale && enableDownscale ? 'both'
    : enableUpscale ? 'upscale'
      : enableDownscale ? 'downscale'
        : null;

  const handleProcess = () => {
    if (!mode) return;
    const options = buildScaleOptions({ input_path: inputPath, output_path: outputPath, recursive }, {
      mode, upscale_width: upWidth, upscale_height: upHeight, downscale_width: downWidth, downscale_height: downHeight,
    });
    const modeLabel = { both: t('scale.startBoth'), upscale: t('scale.startUp'), downscale: t('scale.startDown') }[mode];
    return task.run({
      taskName: t('scale.taskName'),
      startLog: t('scale.startMsg', { mode: modeLabel, width: options.target_width, height: options.target_height }),
      exec: () => invoke<ProcessResult>('scale_images', { options }),
    });
  };

  const scaleCards = [
    {
      key: 'up', enabled: enableUpscale, setEnabled: setEnableUpscale, Icon: ArrowUpCircle, color: '#4ade80', colorAlpha: 'rgba(74, 222, 128, ', label: t('scale.upscale'), desc: t('scale.upscaleDesc'),
      width: upWidth, setWidth: setUpWidth, height: upHeight, setHeight: setUpHeight, fallbackWidth: d.upscale_width, fallbackHeight: d.upscale_height,
    },
    {
      key: 'down', enabled: enableDownscale, setEnabled: setEnableDownscale, Icon: ArrowDownCircle, color: '#60a5fa', colorAlpha: 'rgba(96, 165, 250, ', label: t('scale.downscale'), desc: t('scale.downscaleDesc'),
      width: downWidth, setWidth: setDownWidth, height: downHeight, setHeight: setDownHeight, fallbackWidth: d.downscale_width, fallbackHeight: d.downscale_height,
    },
  ];

  return (
    <ToolPageLayout icon={Scaling} color="#818cf8" title={t('scale.title')} subtitle={t('scale.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath || !mode}
          cancelCommand="cancel_scale" startText={t('scale.startScale')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('scale.scaleOptions')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {scaleCards.map(card => (
            <ChoiceCard key={card.key} selected={card.enabled} onSelect={() => card.setEnabled(!card.enabled)} indicator="check" body={<>
              <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', opacity: card.enabled ? 1 : 0.4, pointerEvents: card.enabled ? 'auto' : 'none' }}>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{t('scale.width')}</label>
                  <NumberInput className="form-input" value={card.width} min={1} onChange={card.setWidth} fallback={card.fallbackWidth} integer />
                </div>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{t('scale.height')}</label>
                  <NumberInput className="form-input" value={card.height} min={1} onChange={card.setHeight} fallback={card.fallbackHeight} integer />
                </div>
              </div>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: card.colorAlpha + '0.06)', border: '1px solid ' + card.colorAlpha + '0.1)' }}>
                <Info style={{ width: 14, height: 14, color: card.color, marginTop: 2, minWidth: 14 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>{card.desc}</span>
              </div>
            </>}>
              <card.Icon style={{ width: 18, height: 18, color: card.enabled ? card.color : 'var(--color-text-tertiary)' }} />
              <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{card.label}</span>
            </ChoiceCard>
          ))}

          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(251, 191, 36, 0.06)', border: '1px solid rgba(251, 191, 36, 0.15)' }}>
            <Info style={{ width: 14, height: 14, color: '#fbbf24', marginTop: 2, minWidth: 14 }} />
            <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
              {t('scale.resizeHint')}
            </span>
          </div>
        </div>
      </div>
    </ToolPageLayout>
  );
}
