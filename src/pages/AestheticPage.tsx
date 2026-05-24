import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import { Star, FolderOpen, Cpu, Gpu } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';
import { usePythonEnvEvents } from '../hooks/usePythonEnvEvents';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; i18n_key?: string; i18n_params?: Record<string, string>; }
interface DownloadPayload { filename: string; downloaded: number; total: number; percent: number; speed_mbps: number; status: string; message: string; }

export default function AestheticPage() {
  const { t } = useTranslation();
  const { addTask, updateTask } = useTaskQueue();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const isMac = /Mac|darwin/i.test(navigator.userAgent);
  const [useGpu, setUseGpu] = useState(!isMac);
  const [batchSize, setBatchSize] = useState(1);
  const [processing, setProcessing] = useState(false);

  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  useEffect(() => {
    let active = true;
    const unlisten = listen<ProgressPayload>('aesthetic-progress', (e) => {
      if (!active) return;
      const d = e.payload;
      if (d.status === 'done') {
        setProgress(100); setIsDone(true); setProcessing(false);
        setProgressCurrent(d.total); setProgressTotal(d.total);
        if (d.message) { const m = d.message.match(/(\d+)/g); if (m && m.length >= 2 && parseInt(m[1]) > 0) setHasError(true); }
        setLogs(p => [...p, { time: getTimeStr(), message: d.message, status: 'success' }]);
        updateTask('aesthetic', { status: 'done', message: d.message });
      } else if (d.status === 'processing') {
        setProgressCurrent(d.current); setProgressTotal(d.total);
        if (d.total > 0) setProgress(Math.round((d.current / d.total) * 100));
        updateTask('aesthetic', { status: 'running', message: `${d.current}/${d.total}`, progress: d.total > 0 ? Math.round((d.current / d.total) * 100) : 0, current: d.current, total: d.total });
      } else {
        const pct = d.total > 0 ? Math.round((d.current / d.total) * 100) : 0;
        setProgress(pct); setProgressCurrent(d.current); setProgressTotal(d.total);
        if (d.status === 'error') setHasError(true);
        const resolveMsg = (p: ProgressPayload) => p.i18n_key ? (t(p.i18n_key, p.i18n_params || {}) !== p.i18n_key ? t(p.i18n_key, p.i18n_params || {}) : p.message) : p.message;
        setLogs(p => [...p, { time: getTimeStr(), message: resolveMsg(d), status: d.status as any }]);
        updateTask('aesthetic', { status: 'running', message: `${d.current}/${d.total}`, progress: pct, current: d.current, total: d.total });
      }
    });
    return () => { active = false; unlisten.then(fn => fn()); };
  }, []);

  useEffect(() => {
    let active = true;
    const unlisten = listen<DownloadPayload>('aesthetic-download', (e) => {
      if (!active) return;
      const d = e.payload;
      if (d.status === 'done' || d.status === 'cancelled') {
        setLogs(p => [...p.filter(l => l.status !== 'download'), { time: getTimeStr(), message: d.message, status: 'success' }]);
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
    return () => { active = false; unlisten.then(fn => fn()); };
  }, []);

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
    if (!inputPath) return;
    setProcessing(true); setIsDone(false); setHasError(false); setProgress(0);
    setProgressCurrent(0); setProgressTotal(0);
    setLogs([{ time: getTimeStr(), message: t('aesthetic.starting'), status: 'info' }]);
    addTask('aesthetic', t('aesthetic.taskName'));
    try {
      await invoke<ProcessResult>('start_aesthetic_scoring', {
        options: {
          input_path: inputPath,
          output_path: outputPath || '',
          use_gpu: useGpu,
          batch_size: useGpu ? batchSize : 1,
          move_files: true,
        }
      });
    } catch (e: any) {
      setProcessing(false); setHasError(true);
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      updateTask('aesthetic', { status: 'error', message: String(e) });
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Star style={{ width: 28, height: 28, color: '#facc15' }} />
          <h1 className="page-title">{t('aesthetic.title')}</h1>
        </div>
        <p className="page-subtitle">{t('aesthetic.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <span className="tool-panel-title">{t('pages.pathSettings')}</span>
              {!isMac && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                  <span style={{ fontSize: 11, fontWeight: 600, color: useGpu ? 'var(--color-text-secondary)' : 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{t('aesthetic.batchSize')}</span>
                  <input type="number" className="form-input" min={1} max={64} value={batchSize} onChange={e => setBatchSize(e.target.value === "" ? "" as any : Math.max(1, parseInt(e.target.value) || 1))} onBlur={e => { if (e.target.value === "") setBatchSize(1); }} disabled={!useGpu} style={{ width: 58, padding: '3px 6px', fontSize: 11, textAlign: 'center', opacity: useGpu ? 1 : 0.4 }} />
                  <div style={{ width: 1, height: 16, background: 'var(--color-border)', margin: '0 4px' }} />
                  {([{ val: false, label: 'CPU', icon: <Cpu style={{ width: 13, height: 13 }} />, color: '#fbbf24' },
                    { val: true, label: 'GPU', icon: <Gpu style={{ width: 13, height: 13 }} />, color: '#4ade80' }] as const).map(d => (
                    <button key={d.label} onClick={() => setUseGpu(d.val)} style={{
                      padding: '4px 12px', borderRadius: 'var(--radius-sm)', fontSize: 11, fontWeight: 700,
                      cursor: 'pointer', transition: 'all 0.15s',
                      display: 'flex', alignItems: 'center', gap: 4,
                      border: `1.5px solid ${useGpu === d.val ? d.color : 'var(--color-border)'}`,
                      background: useGpu === d.val ? `${d.color}12` : 'transparent',
                      color: useGpu === d.val ? d.color : 'var(--color-text-tertiary)',
                    }}>
                      {d.icon} {d.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">{t('pages.inputPathShort')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectInputFolder')} value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">{t('aesthetic.outputPath')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectOutputFolder')} value={outputPath} onChange={(e) => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
            {/* 功能说明 */}
            <div style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--color-text-tertiary)', lineHeight: 1.6 }}>
              {t('aesthetic.description')}：
              {['masterpiece', 'best', 'great', 'good', 'normal', 'low', 'worst'].map((l, i) => (
                <span key={l} style={{ fontWeight: 600, color: ['#facc15','#a78bfa','#34d399','#60a5fa','#94a3b8','#fb923c','#f87171'][i] }}>
                  {l}{i < 6 ? '、' : ''}
                </span>
              ))}
            </div>
            <div style={{ marginTop: 6, fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>
              {t('aesthetic.modelSource')} <a href="https://huggingface.co/deepghs/anime_aesthetic" target="_blank" rel="noreferrer" style={{ color: '#818cf8' }}>deepghs/anime_aesthetic</a>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath}
            cancelCommand="cancel_aesthetic_scoring"
            startText={t('aesthetic.start')}
            processingText={t('aesthetic.processing')}
            onCancelLog={addCancelLog} />

          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
