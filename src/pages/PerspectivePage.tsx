import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  Move3D,
  FolderOpen,
  Info,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

export default function PerspectivePage() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [intensity, setIntensity] = useState(0.10);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('perspective-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) setProgress((d.current / d.total) * 100);
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs((prev) => [...prev, {
          time: getTimeStr(),
          message: d.message,
          status: d.status === 'done' ? 'info' : d.status as LogEntry['status'],
        }]);
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, []);

  const selectInputFolder = async () => { const s = await open({ directory: true, multiple: false, title: t('pages.selectInputTitle') }); if (s) setInputPath(s as string); };
  const selectOutputFolder = async () => { const s = await open({ directory: true, multiple: false, title: t('pages.selectOutputTitle') }); if (s) setOutputPath(s as string); };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true); addTask('perspective', t('perspective.taskName'));
    setProgress(0); setProgressCurrent(0); setProgressTotal(0); setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: t('perspective.startMsg', { intensity: intensity.toFixed(2) }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('perspective_transform', {
        options: { input_path: inputPath, output_path: outputPath, intensity },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Move3D style={{ width: 28, height: 28, color: '#f472b6' }} />
          <h1 className="page-title">{t('perspective.title')}</h1>
        </div>
        <p className="page-subtitle">{t('perspective.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('perspective.paramSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group">
                <label className="form-label">{t('blurNoise.inputFolder')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('blurNoise.inputPlaceholder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">{t('blurNoise.outputFolder')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('blurNoise.outputPlaceholder')} value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>

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

              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(244, 114, 182, 0.06)', border: '1px solid rgba(244, 114, 182, 0.1)' }}>
                <Info style={{ width: 13, height: 13, color: '#f472b6', marginTop: 2, minWidth: 13 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  {t('perspective.tip')}
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_perspective" startText={t('perspective.startProcess')} processingText={t('pages.processing')}
            onCancelLog={addCancelLog} />
          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
