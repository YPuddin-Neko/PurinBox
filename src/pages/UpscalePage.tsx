import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  ZoomIn,
  FolderOpen,
  Download,
  CheckCircle2,
  Cpu,
  Gpu,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';
import { usePythonEnvEvents } from '../hooks/usePythonEnvEvents';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }

interface UpscaleModelChoice { id: string; name: string; dir_name: string; }
interface UpscaleEngineInfo {
  id: string; name: string; description: string; downloaded: boolean;
  size_mb: number; scales: number[]; models: UpscaleModelChoice[];
  supports_denoise: boolean; denoise_range: [number, number];
  supports_cpu: boolean; use_python: boolean;
}

interface DownloadProgress {
  downloaded: number; total: number; percent: number;
  speed_mbps: number; status: string; message: string;
}

export default function UpscalePage() {
  const { t } = useTranslation();
  const { addTask, updateTask } = useTaskQueue();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [engines, setEngines] = useState<UpscaleEngineInfo[]>([]);
  const [selectedEngine, setSelectedEngine] = useState('realcugan');
  const [selectedModel, setSelectedModel] = useState('');
  const [scale, setScale] = useState(2);
  const [denoiseLevel, setDenoiseLevel] = useState(-1);
  const [tta, setTta] = useState(false);
  const [useGpu, setUseGpu] = useState(true);
  const [tileSize, setTileSize] = useState(-1);
  const [processing, setProcessing] = useState(false);

  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  const engine = engines.find(e => e.id === selectedEngine);

  // Load engines
  useEffect(() => {
    invoke<UpscaleEngineInfo[]>('get_upscale_engines').then(list => {
      setEngines(list);
      if (list.length > 0 && !selectedModel) {
        setSelectedModel(list[0].models[0]?.id || '');
      }
    }).catch(() => {});
  }, []);

  // When engine changes, set defaults
  useEffect(() => {
    if (!engine) return;
    setSelectedModel(engine.models[0]?.id || '');
    setScale(engine.scales.includes(2) ? 2 : engine.scales[0] || 2);
    setDenoiseLevel(engine.supports_denoise ? -1 : 0);
    // Force GPU on if engine doesn't support CPU
    if (!engine.supports_cpu) setUseGpu(true);
  }, [selectedEngine, engines]);

  // Listen to progress events
  useEffect(() => {
    const unlisten = listen<any>('upscale-progress', (e) => {
      const d = e.payload;
      if (d.status === 'done') {
        setProgress(100); setIsDone(true); setProcessing(false);
        setProgressCurrent(d.total); setProgressTotal(d.total);
        if (d.message?.includes('失败') && !d.message?.includes('失败 0')) setHasError(true);
        setLogs(p => [...p, { time: getTimeStr(), message: d.message, status: 'success' }]);
        updateTask('upscale', { status: 'done', message: d.message });
      } else if (d.status === 'processing') {
        // Show the first processing event ("开始超分") as info log, then just update progress
        if (d.current === 0) {
          setLogs(p => [...p, { time: getTimeStr(), message: d.message, status: 'info' }]);
        }
        setProgressCurrent(d.current); setProgressTotal(d.total);
        if (d.total > 0) setProgress(Math.round((d.current / d.total) * 100));
      } else {
        // info, success per-file, error
        const pct = d.total > 0 ? Math.round(((d.current) / d.total) * 100) : 0;
        setProgress(pct); setProgressCurrent(d.current); setProgressTotal(d.total);
        if (d.status === 'error') setHasError(true);
        setLogs(p => [...p, { time: getTimeStr(), message: d.message, status: d.status }]);
        updateTask('upscale', { status: 'running', message: `${d.current}/${d.total}` });
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // Listen to download events — inline progress in ProgressLog (same as tagger)
  useEffect(() => {
    const unlisten = listen<DownloadProgress>('upscale-download', (e) => {
      const d = e.payload;
      if (d.status === 'done' || d.status === 'cancelled') {
        setLogs(p => [...p.filter(l => l.status !== 'download'), { time: getTimeStr(), message: d.message, status: 'success' }]);
        invoke<UpscaleEngineInfo[]>('get_upscale_engines').then(setEngines).catch(() => {});
      } else if (d.status === 'error') {
        setLogs(p => [...p.filter(l => l.status !== 'download'), { time: getTimeStr(), message: d.message, status: 'error' }]);
      } else {
        const avgSpeed = d.speed_mbps > 0 ? `${d.speed_mbps.toFixed(1)} MB/s` : '';
        setLogs(p => {
          const idx = p.findIndex(l => l.status === 'download');
          const entry: LogEntry = { time: getTimeStr(), message: d.message, status: 'download', dlPercent: d.percent, dlSpeed: avgSpeed };
          if (idx >= 0) { const next = [...p]; next[idx] = entry; return next; }
          return [...p, entry];
        });
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // Python 环境事件（统一 hook）
  usePythonEnvEvents(processing, setLogs);

  const selectInputFolder = async () => {
    const p = await open({ directory: true, title: t('pages.selectInputTitle') });
    if (p) setInputPath(p as string);
  };
  const selectOutputFolder = async () => {
    const p = await open({ directory: true, title: t('pages.selectOutputTitle') });
    if (p) setOutputPath(p as string);
  };

  const handleProcess = async () => {
    if (!engine || !inputPath || !outputPath) return;
    setProcessing(true); setIsDone(false); setHasError(false); setProgress(0);
    addTask('upscale', t('upscale.taskName'));
    try {
      // If engine not downloaded, download first
      // For Python engines, always run setup to ensure deps + weights are ready
      if (!engine.downloaded || engine.use_python) {
        setLogs([{ time: getTimeStr(), message: t('upscale.downloadingEngine', { name: engine.name }), status: 'info' }]);
        await invoke('download_upscale_engine', { engineId: engine.id });
        // Refresh engines list
        const updated = await invoke<UpscaleEngineInfo[]>('get_upscale_engines');
        setEngines(updated);
      }
      await invoke<ProcessResult>('start_upscale', {
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
        }
      });
    } catch (e: any) {
      setProcessing(false); setHasError(true);
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      updateTask('upscale', { status: 'error', message: String(e) });
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <ZoomIn style={{ width: 28, height: 28, color: '#22d3ee' }} />
          <h1 className="page-title">{t('upscale.title')}</h1>
        </div>
        <p className="page-subtitle">{t('upscale.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 - 参数设置 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('pages.pathSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">{t('pages.inputPathShort')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectInputFolder')} value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">{t('pages.outputPath')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectOutputFolder')} value={outputPath} onChange={(e) => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          {/* 超分引擎 */}
          <div className="tool-panel">
            <div className="tool-panel-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <span className="tool-panel-title">{t('upscale.engine')}</span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                {([{ val: false, label: 'CPU', icon: <Cpu style={{ width: 13, height: 13 }} />, color: '#fbbf24' },
                  { val: true, label: 'GPU', icon: <Gpu style={{ width: 13, height: 13 }} />, color: '#4ade80' }] as const).map(d => (
                  <button key={d.label} onClick={() => {
                    if (d.val === false && engine && !engine.supports_cpu) return;
                    setUseGpu(d.val);
                  }} style={{
                    padding: '4px 12px', borderRadius: 'var(--radius-sm)', fontSize: 11, fontWeight: 700,
                    cursor: (!d.val && engine && !engine.supports_cpu) ? 'not-allowed' : 'pointer',
                    transition: 'all 0.15s',
                    display: 'flex', alignItems: 'center', gap: 4,
                    border: `1.5px solid ${useGpu === d.val ? d.color : 'var(--color-border)'}`,
                    background: useGpu === d.val ? `${d.color}12` : 'transparent',
                    color: useGpu === d.val ? d.color : 'var(--color-text-tertiary)',
                    opacity: (!d.val && engine && !engine.supports_cpu) ? 0.35 : 1,
                  }}>
                    {d.icon} {d.label}
                  </button>
                ))}
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>

              {/* 引擎选择按钮 */}
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                {engines.map(e => (
                  <button key={e.id}
                    className={`btn ${selectedEngine === e.id ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setSelectedEngine(e.id)}
                    style={{ flex: 1, position: 'relative' }}>
                    {e.name}
                    {e.downloaded && <CheckCircle2 style={{ width: 12, height: 12, position: 'absolute', top: 4, right: 4, color: '#4ade80' }} />}
                  </button>
                ))}
              </div>

              {engine && (
                <>
                  {/* 引擎描述 + 状态 */}
                  <div style={{ fontSize: 12, color: 'var(--color-text-secondary)', padding: '8px 12px', background: 'var(--color-bg-input)', borderRadius: 'var(--radius-sm)', border: '1px solid var(--color-border)' }}>
                    {engine.description}
                    {!engine.downloaded && (
                      <span style={{ color: '#f87171', marginLeft: 8 }}>（{t('upscale.notDownloaded')}）</span>
                    )}
                    {engine.downloaded && (
                      <span style={{ color: '#4ade80', marginLeft: 8 }}>{t('upscale.downloaded')}</span>
                    )}
                  </div>



                  {/* 模型/风格选择 */}
                  <div className="form-group">
                    <label className="form-label">{engine.id === 'realesrgan' ? t('upscale.modelSelect') : t('upscale.styleSelect')}</label>
                    <div style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
                      {engine.models.map(m => (
                        <button key={m.id}
                          className={`btn btn-sm ${selectedModel === m.id ? 'btn-primary' : 'btn-secondary'}`}
                          onClick={() => setSelectedModel(m.id)}>
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
                    <div className="form-group">
                      <label className="form-label">{t('upscale.denoiseLevel')}</label>
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
                      <input
                        className="form-input"
                        type="number"
                        value={tileSize}
                        onChange={(e) => setTileSize(parseInt(e.target.value) || -1)}
                        style={{ width: 100 }}
                        min={-1}
                      />
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
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_upscale" forceCancelCommand="force_cancel_upscale"
            startText={engine && !engine.downloaded ? t('upscale.downloadAndUpscale') : t('upscale.startUpscale')}
            startIcon={engine && !engine.downloaded ? <Download style={{ width: 18, height: 18 }} /> : undefined}
            processingText={t('upscale.upscaling')}
            onCancelLog={addCancelLog} />

          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
