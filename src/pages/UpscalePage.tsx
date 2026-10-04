import { invoke } from '@tauri-apps/api/core';
import {
  CheckCircle2,
  Download,
  ZoomIn
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildUpscaleOptions, UPSCALE_DEFAULTS } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PathFields from '../components/ui/PathFields';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

interface UpscaleModelChoice { id: string; name: string; }
interface UpscaleEngineInfo {
  id: string; name: string; downloaded: boolean;
  scales: number[]; models: UpscaleModelChoice[];
  supports_denoise: boolean; denoise_range: [number, number];
  supports_cpu: boolean; use_python: boolean;
}

export default function UpscalePage() {
  const { t } = useTranslation();
  const task = useBatchTask({
    event: 'upscale-progress', taskId: 'upscale', pythonEnv: true, logProcessing: p => p.current === 0,
    download: { event: 'upscale-download' },
  });
  const d = UPSCALE_DEFAULTS;

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [engines, setEngines] = useState<UpscaleEngineInfo[]>([]);
  const [selectedEngine, setSelectedEngine] = useState(d.engine_id);
  const [selectedModel, setSelectedModel] = useState(d.model_id);
  const [scale, setScale] = useState(d.scale);
  const [denoiseLevel, setDenoiseLevel] = useState(d.denoise_level);
  const [tta, setTta] = useState(d.tta);
  const [useGpu, setUseGpu] = useState(d.use_gpu);
  const [tileSize, setTileSize] = useState(d.tile_size);

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

  // 刷新引擎列表会起一次 Python 探测（1–3 秒），不能夹在下载与开始超分之间
  const refreshEngines = useCallback(() => {
    invoke<UpscaleEngineInfo[]>('get_upscale_engines').then(setEngines).catch(() => { });
  }, []);

  const handleProcess = () => {
    if (!engine) return;
    const options = buildUpscaleOptions({ input_path: inputPath, output_path: outputPath, recursive }, {
      engine_id: selectedEngine, model_id: selectedModel, scale, denoise_level: denoiseLevel, tta, use_gpu: useGpu, tile_size: tileSize,
    });
    return task.run({
      taskName: t('upscale.taskName'),
      startLog: t('pages.startMsg', { name: t('upscale.title') }),
      exec: async run => {
        let downloaded = false;
        try {
          // Python 引擎的权重已存在时，下载命令仍要检查并补齐依赖
          if (!engine.downloaded || engine.use_python) {
            task.logger.appendLog(t('upscale.downloadingEngine', { name: engine.name }), 'info');
            await invoke('download_upscale_engine', { engineId: engine.id });
            downloaded = true;
          }
          // start_upscale 开头会复位取消标志，下载结束后才点的取消只能在这里拦下；没有后端事件报告这次取消，终态日志由这里写
          if (run.cancelRequested()) {
            task.logger.appendLog(t('header.cancelled'), 'warning');
            return undefined;
          }
          return await invoke<ProcessResult>('start_upscale', { options });
        } finally {
          if (downloaded) refreshEngines();
        }
      },
    });
  };

  return (
    <ToolPageLayout icon={ZoomIn} color="#22d3ee" title={t('upscale.title')} subtitle={t('upscale.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath || !engine}
          cancelCommand="cancel_upscale" forceCancelCommand="force_cancel_upscale"
          startText={engine && !engine.downloaded ? t('upscale.downloadAndUpscale') : t('upscale.startUpscale')}
          startIcon={engine && !engine.downloaded ? <Download style={{ width: 18, height: 18 }} /> : undefined}
          processingText={t('upscale.upscaling')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields allowFile input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('upscale.engine')}</span>
          <DeviceToggle useGpu={useGpu} onChange={setUseGpu} cpuDisabled={!engine?.supports_cpu} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
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

              <div className="form-group">
                <label className="form-label">{t('upscale.tileSize')}</label>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                  <NumberInput className="form-input" value={tileSize} style={{ width: 100 }} min={-1} onChange={setTileSize} fallback={d.tile_size} integer />
                  <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>
                    {t('upscale.tileSizeDesc')}
                  </span>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </ToolPageLayout>
  );
}
