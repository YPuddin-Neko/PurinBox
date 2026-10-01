import { invoke } from '@tauri-apps/api/core';
import {
  CheckCircle2,
  Download,
  ZoomIn
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { UpscaleOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';
import { UnifiedDownloadPayload } from '../hooks/useUnifiedTaskLogs';
import { listen } from '../utils/tauriRuntime';

interface UpscaleModelChoice { id: string; name: string; }
interface UpscaleEngineInfo {
  id: string; name: string; downloaded: boolean;
  scales: number[]; models: UpscaleModelChoice[];
  supports_denoise: boolean; denoise_range: [number, number];
  supports_cpu: boolean; use_python: boolean;
}

export default function UpscalePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'upscale-progress', taskId: 'upscale', pythonEnv: true, logProcessing: p => p.current === 0 });
  const downloadActive = useRef(false);

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [engines, setEngines] = useState<UpscaleEngineInfo[]>([]);
  const [selectedEngine, setSelectedEngine] = useState('realcugan');
  const [selectedModel, setSelectedModel] = useState('');
  const [scale, setScale] = useState(2);
  const [denoiseLevel, setDenoiseLevel] = useState(-1);
  const [tta, setTta] = useState(false);
  const [useGpu, setUseGpu] = useState(true);
  const [tileSize, setTileSize] = useState(-1);

  const engine = engines.find(e => e.id === selectedEngine);

  const applyEngineDefaults = (e: UpscaleEngineInfo) => {
    setSelectedModel(e.models[0]?.id || '');
    setScale(e.scales.includes(2) ? 2 : e.scales[0] || 2);
    setDenoiseLevel(e.supports_denoise ? -1 : 0);
    if (!e.supports_cpu) setUseGpu(true);
  };

  const selectEngine = (e: UpscaleEngineInfo) => {
    if (e.id === selectedEngine) return;
    setSelectedEngine(e.id);
    applyEngineDefaults(e);
  };

  useEffect(() => {
    invoke<UpscaleEngineInfo[]>('get_upscale_engines').then(list => {
      setEngines(list);
      const initial = list.find(e => e.id === selectedEngine) ?? list[0];
      if (initial) applyEngineDefaults(initial);
    }).catch(() => { });
  }, []);

  useEffect(() => {
    let active = true;
    const unlisten = listen<UnifiedDownloadPayload>('upscale-download', (e) => {
      if (!active || !downloadActive.current) return;
      task.logger.appendDownloadLog(e.payload);
    });
    return () => { active = false; unlisten.then(fn => fn()); };
  }, [task.logger]);

  const handleProcess = () => {
    if (!engine) return;
    return task.run({
      taskName: t('upscale.taskName'), startLog: t('pages.startMsg', { name: t('upscale.title') }), exec: async () => {
        downloadActive.current = true;
        try {
          // Python engines also prepare dependencies when their weights already exist.
          if (!engine.downloaded || engine.use_python) {
            task.logger.appendLog(t('upscale.downloadingEngine', { name: engine.name }), 'info');
            await invoke('download_upscale_engine', { engineId: engine.id });
            setEngines(await invoke<UpscaleEngineInfo[]>('get_upscale_engines'));
          }
          return await invoke<ProcessResult>('start_upscale', {
            options: {
              input_path: inputPath,
              output_path: outputPath,
              engine_id: selectedEngine,
              model_id: selectedModel,
              scale,
              denoise_level: denoiseLevel,
              tta,
              gpu_id: useGpu ? 0 : -1,
              tile_size: tileSize,
              recursive,
            } satisfies UpscaleOptions
          });
        } finally { downloadActive.current = false; }
      }
    });
  };

  return (
    <div className="page">
      <PageHeader icon={ZoomIn} color={'#22d3ee'} title={t('upscale.title')} subtitle={t('upscale.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 - 参数设置 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          {/* 路径设置 */}
          <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          {/* 超分引擎 */}
          <div className="tool-panel">
            <div className="tool-panel-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <span className="tool-panel-title">{t('upscale.engine')}</span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <DeviceToggle useGpu={useGpu} onChange={setUseGpu} cpuDisabled={!engine?.supports_cpu} />
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>

              {/* 引擎选择按钮 */}
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                {engines.map(e => (
                  <button key={e.id}
                    className={`btn ${selectedEngine === e.id ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => selectEngine(e)}
                    style={{ flex: 1, position: 'relative' }}>
                    {e.name}
                    {e.downloaded && <CheckCircle2 style={{ width: 12, height: 12, position: 'absolute', top: 4, right: 4, color: '#4ade80' }} />}
                  </button>
                ))}
              </div>

              {engine && (
                <>
                  {/* 引擎描述 + 状态 */}

                  {/* 模型/风格选择 */}
                  <div className="form-group">
                    <label className="form-label">{engine.id === 'realesrgan' ? t('upscale.modelSelect') : t('upscale.styleSelect')}</label>
                    <div style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
                      {engine.models.map(m => (
                        <button key={m.id}
                          className={`btn btn-sm ${selectedModel === m.id ? 'btn-primary' : 'btn-secondary'}`}
                          onClick={() => {
                            setSelectedModel(m.id);
                            if (m.id === 'models-nose') setDenoiseLevel(-1);
                          }}>
                          {m.name}
                        </button>
                      ))}
                    </div>
                  </div>

                  {/* 超分倍率 */}
                  <div className="form-group">
                    <label className="form-label">{t('upscale.scaleRatio')}</label>
                    <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                      {engine.scales.map(s => (
                        <button key={s}
                          className={`btn btn-sm ${scale === s ? 'btn-primary' : 'btn-secondary'}`}
                          onClick={() => setScale(s)}>
                          {s}x
                        </button>
                      ))}
                    </div>
                  </div>

                  {/* 降噪等级 */}
                  {engine.supports_denoise && (
                    <div className="form-group" style={{ opacity: selectedModel === 'models-nose' ? 0.4 : 1, pointerEvents: selectedModel === 'models-nose' ? 'none' : 'auto' }}>
                      <label className="form-label">
                        {t('upscale.denoiseLevel')}
                        {selectedModel === 'models-nose' && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginLeft: 8 }}>{t('upscale.noDenoiseModelHint')}</span>}
                      </label>
                      <div style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
                        {Array.from({ length: engine.denoise_range[1] - engine.denoise_range[0] + 1 }, (_, i) => engine.denoise_range[0] + i).map(n => (
                          <button key={n}
                            className={`btn btn-sm ${denoiseLevel === n ? 'btn-primary' : 'btn-secondary'}`}
                            onClick={() => setDenoiseLevel(n)}>
                            {n === -1 ? t('upscale.noDenoiseBtn') : t('upscale.levelBtn', { n })}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* TTA 增强 */}
                  <div className="form-group">
                    <label className="form-label">{t('upscale.tta')}</label>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                      <button
                        className={`btn btn-sm ${tta ? 'btn-primary' : 'btn-secondary'}`}
                        onClick={() => setTta(!tta)}>
                        {tta ? t('upscale.ttaOn') : t('upscale.ttaOff')}
                      </button>
                      <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>
                        {t('upscale.ttaDesc')}
                      </span>
                    </div>
                  </div>

                  {/* 分块大小 */}
                  <div className="form-group">
                    <label className="form-label">{t('upscale.tileSize')}</label>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                      <NumberInput className="form-input" value={tileSize} style={{ width: 100 }} min={-1} onChange={setTileSize} fallback={-1} integer />
                      <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>
                        {t('upscale.tileSizeDesc')}
                      </span>
                    </div>
                  </div>
                </>
              )}
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath || !engine}
            cancelCommand="cancel_upscale" forceCancelCommand="force_cancel_upscale"
            startText={engine && !engine.downloaded ? t('upscale.downloadAndUpscale') : t('upscale.startUpscale')}
            startIcon={engine && !engine.downloaded ? <Download style={{ width: 18, height: 18 }} /> : undefined}
            processingText={t('upscale.upscaling')} />

          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
