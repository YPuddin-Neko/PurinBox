import { invoke } from '@tauri-apps/api/core';
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Crop,
  Crosshair,
  Info,
  Maximize2,
  RatioIcon,
  Scaling,
  Scissors
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { CropOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

const PRESETS = [
  { label: '1:1', w: 1, h: 1 },
  { label: '3:4', w: 3, h: 4 },
  { label: '4:3', w: 4, h: 3 },
  { label: '9:16', w: 9, h: 16 },
  { label: '16:9', w: 16, h: 9 },
  { label: '2:3', w: 2, h: 3 },
];

type CropAnchor = 'center' | 'top' | 'bottom' | 'left' | 'right';

export default function CropPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'crop-progress', taskId: 'crop' });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [mode, setMode] = useState<'center' | 'cover' | 'aspect' | 'edges'>('center');
  const [cropAnchor, setCropAnchor] = useState<CropAnchor>('center');
  const [centerW, setCenterW] = useState(1024);
  const [centerH, setCenterH] = useState(1024);
  const [ratioW, setRatioW] = useState(1);
  const [ratioH, setRatioH] = useState(1);
  const [cropTop, setCropTop] = useState(0);
  const [cropBottom, setCropBottom] = useState(0);
  const [cropLeft, setCropLeft] = useState(0);
  const [cropRight, setCropRight] = useState(0);

  const handleProcess = () => {

    return task.run({
      taskName: t('crop.taskName'), startLog: t('pages.startMsg', { name: modeCards.find(m => m.key === mode)!.label }), exec: () => invoke<ProcessResult>('crop_images', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode,
          target_width: centerW,
          target_height: centerH,
          crop_anchor: cropAnchor,
          aspect_ratio: ratioW / ratioH,
          crop_top: cropTop,
          crop_bottom: cropBottom,
          crop_left: cropLeft,
          crop_right: cropRight,
          recursive,
        } satisfies CropOptions,
      })
    });
  };

  const modeCards: { key: 'center' | 'cover' | 'aspect' | 'edges'; icon: React.ReactNode; label: string; color: string; colorAlpha: string }[] = [
    { key: 'center', icon: <Maximize2 style={{ width: 18, height: 18 }} />, label: t('crop.center'), color: '#4ade80', colorAlpha: 'rgba(74, 222, 128, ' },
    { key: 'cover', icon: <Scaling style={{ width: 18, height: 18 }} />, label: t('crop.cover'), color: '#06b6d4', colorAlpha: 'rgba(6, 182, 212, ' },
    { key: 'aspect', icon: <RatioIcon style={{ width: 18, height: 18 }} />, label: t('crop.aspect'), color: '#818cf8', colorAlpha: 'rgba(129, 140, 248, ' },
    { key: 'edges', icon: <Scissors style={{ width: 18, height: 18 }} />, label: t('crop.edges'), color: '#f59e0b', colorAlpha: 'rgba(245, 158, 11, ' },
  ];
  const cropAnchors: { key: CropAnchor; icon: React.ReactNode; label: string }[] = [
    { key: 'center', icon: <Crosshair style={{ width: 14, height: 14 }} />, label: t('crop.anchorCenter') },
    { key: 'top', icon: <ArrowUp style={{ width: 14, height: 14 }} />, label: t('crop.anchorTop') },
    { key: 'bottom', icon: <ArrowDown style={{ width: 14, height: 14 }} />, label: t('crop.anchorBottom') },
    { key: 'left', icon: <ArrowLeft style={{ width: 14, height: 14 }} />, label: t('crop.anchorLeft') },
    { key: 'right', icon: <ArrowRight style={{ width: 14, height: 14 }} />, label: t('crop.anchorRight') },
  ];

  return (
    <div className="page">
      <PageHeader icon={Crop} color={'#34d399'} title={t('crop.title')} subtitle={t('crop.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          {/* 裁切模式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('crop.cropMode')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {modeCards.map((mc) => (
                <ChoiceCard key={mc.key} selected={mode === mc.key} onSelect={() => setMode(mc.key)} indicator="radio" body={<>

                  {/* Mode-specific options */}
                  {(mc.key === 'center' || mc.key === 'cover') && mode === mc.key && (
                    <div style={{ marginBottom: 'var(--space-3)' }}>
                      <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">{t('crop.targetWidth')}</label>
                          <NumberInput className="form-input" value={centerW} min={1} onChange={setCenterW} fallback={1024} integer />
                        </div>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">{t('crop.targetHeight')}</label>
                          <NumberInput className="form-input" value={centerH} min={1} onChange={setCenterH} fallback={1024} integer />
                        </div>
                      </div>
                      {mc.key === 'cover' && (
                        <div className="form-group" onClick={(e) => e.stopPropagation()}>
                          <label className="form-label">{t('crop.cropAnchor')}</label>
                          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, minmax(0, 1fr))', gap: 6 }}>
                            {cropAnchors.map((anchor) => {
                              const active = cropAnchor === anchor.key;
                              return (
                                <button
                                  key={anchor.key}
                                  type="button"
                                  onClick={() => setCropAnchor(anchor.key)}
                                  title={anchor.label}
                                  style={{
                                    height: 34,
                                    borderRadius: 'var(--radius-sm)',
                                    border: `1px solid ${active ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                                    background: active ? 'rgba(6, 182, 212, 0.10)' : 'var(--color-bg-secondary)',
                                    color: active ? '#06b6d4' : 'var(--color-text-secondary)',
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                    gap: 5,
                                    cursor: 'pointer',
                                    fontSize: 'var(--font-size-xs)',
                                    fontWeight: 700,
                                    fontFamily: 'inherit',
                                  }}
                                >
                                  {anchor.icon}
                                  <span>{anchor.label}</span>
                                </button>
                              );
                            })}
                          </div>
                        </div>
                      )}
                    </div>
                  )}

                  {mc.key === 'aspect' && mode === 'aspect' && (
                    <div style={{ marginBottom: 'var(--space-3)' }}>
                      <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', alignItems: 'flex-end' }}>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">{t('crop.widthRatio')}</label>
                          <NumberInput className="form-input" value={ratioW} min={1} onChange={setRatioW} fallback={1} integer />
                        </div>
                        <span style={{ paddingBottom: 10, fontWeight: 700, color: 'var(--color-text-tertiary)', fontSize: 18 }}>:</span>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">{t('crop.heightRatio')}</label>
                          <NumberInput className="form-input" value={ratioH} min={1} onChange={setRatioH} fallback={1} integer />
                        </div>
                      </div>
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                        {PRESETS.map((p) => (
                          <button key={p.label} className="btn btn-ghost" onClick={(e) => { e.stopPropagation(); setRatioW(p.w); setRatioH(p.h); }}
                            style={{
                              fontSize: 11, height: 26, padding: '0 10px', borderRadius: 13,
                              background: ratioW === p.w && ratioH === p.h ? 'rgba(129, 140, 248, 0.15)' : undefined,
                              color: ratioW === p.w && ratioH === p.h ? '#818cf8' : undefined,
                              border: ratioW === p.w && ratioH === p.h ? '1px solid rgba(129, 140, 248, 0.3)' : undefined,
                            }}>
                            {p.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {mc.key === 'edges' && mode === 'edges' && (
                    <div style={{ marginBottom: 'var(--space-3)' }}>
                      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-3)' }}>
                        <div className="form-group">
                          <label className="form-label">{t('crop.cropTop')}</label>
                          <NumberInput className="form-input" value={cropTop} min={0} onChange={setCropTop} fallback={0} integer />
                        </div>
                        <div className="form-group">
                          <label className="form-label">{t('crop.cropBottom')}</label>
                          <NumberInput className="form-input" value={cropBottom} min={0} onChange={setCropBottom} fallback={0} integer />
                        </div>
                        <div className="form-group">
                          <label className="form-label">{t('crop.cropLeft')}</label>
                          <NumberInput className="form-input" value={cropLeft} min={0} onChange={setCropLeft} fallback={0} integer />
                        </div>
                        <div className="form-group">
                          <label className="form-label">{t('crop.cropRight')}</label>
                          <NumberInput className="form-input" value={cropRight} min={0} onChange={setCropRight} fallback={0} integer />
                        </div>
                      </div>
                    </div>
                  )}

                  {mc.key === 'cover' && (<div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: mc.colorAlpha + '0.06)', border: '1px solid ' + mc.colorAlpha + '0.1)' }}>
                    <Info style={{ width: 14, height: 14, color: mc.color, marginTop: 2, minWidth: 14 }} />
                    <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>{t('crop.coverDesc')}</span>
                  </div>)}
                </>}>

                  <span style={{ color: mode === mc.key ? mc.color : 'var(--color-text-tertiary)' }}>{mc.icon}</span>
                  <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{mc.label}</span>
                </ChoiceCard>
              ))}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_crop" startText={t('crop.startCrop')} />

          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
