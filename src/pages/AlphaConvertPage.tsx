import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import { Layers, FolderOpen } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

export default function AlphaConvertPage() {
  const { t } = useTranslation();

  const bgOptions = [
    { value: 'white', label: t('alphaConvert.whiteBg'), desc: t('alphaConvert.whiteBgDesc'), color: '#e8eaf0' },
    { value: 'black', label: t('alphaConvert.blackBg'), desc: t('alphaConvert.blackBgDesc'), color: '#5a5e78' },
  ];
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [background, setBackground] = useState('white');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('alpha-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) setProgress((d.current / d.total) * 100);
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs((prev) => [...prev, { time: getTimeStr(), message: d.message, status: d.status === 'done' ? 'info' : d.status as LogEntry['status'] }]);
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, []);

  const selectInputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('pages.selectInputTitle') });
    if (selected) setInputPath(selected as string);
  };

  const selectOutputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('pages.selectOutputTitle') });
    if (selected) setOutputPath(selected as string);
  };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true);
    addTask('alpha', t('alphaConvert.taskName'));
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: t('alphaConvert.startMsg', { bg: background === 'white' ? t('alphaConvert.bgWhite') : t('alphaConvert.bgBlack') }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('convert_alpha', {
        options: { input_path: inputPath, output_path: outputPath, background },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Layers style={{ width: 28, height: 28, color: '#c084fc' }} />
          <h1 className="page-title">{t('alphaConvert.title')}</h1>
        </div>
        <p className="page-subtitle">{t('alphaConvert.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径 */}
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

          {/* 背景色 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('alphaConvert.fillArea')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {bgOptions.map((opt) => (
                <div key={opt.value} onClick={() => setBackground(opt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${background === opt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: background === opt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 20, height: 20, borderRadius: '50%', minWidth: 20, border: `2px solid ${background === opt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {background === opt.value && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <div style={{
                    width: 28, height: 28, borderRadius: 'var(--radius-sm)', minWidth: 28,
                    background: opt.value === 'white' ? '#ffffff' : '#1a1a1a',
                    border: '1px solid var(--color-border)',
                  }} />
                  <div>
                    <div style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>


        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_alpha" startText={t('alphaConvert.startConvert')} processingText={t('pages.processing')}
            onCancelLog={addCancelLog} />
          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
