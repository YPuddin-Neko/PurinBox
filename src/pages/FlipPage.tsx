import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  FlipHorizontal2,
  FlipVertical2,
  FolderOpen,
  RotateCcw,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult {
  success_count: number;
  fail_count: number;
  total: number;
  errors: string[];
}

interface ProgressPayload {
  current: number;
  total: number;
  filename: string;
  status: string;
  message: string;
}

type FlipDirection = 'horizontal' | 'vertical' | 'both';

export default function FlipPage() {
  const { t } = useTranslation();

  const flipOptions: { value: FlipDirection; label: string; desc: string; icon: React.ReactNode }[] = [
    { value: 'horizontal', label: t('flip.horizontal'), desc: t('flip.horizontalDesc'), icon: <FlipHorizontal2 /> },
    { value: 'vertical', label: t('flip.vertical'), desc: t('flip.verticalDesc'), icon: <FlipVertical2 /> },
    { value: 'both', label: t('flip.both'), desc: t('flip.bothDesc'), icon: <RotateCcw /> },
  ];

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [direction, setDirection] = useState<FlipDirection>('horizontal');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('flip-progress', (event) => {
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
    addTask('flip', t('flip.taskName'));
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);
    const dirLabel = flipOptions.find((o) => o.value === direction)!.label;
    setLogs([{ time: getTimeStr(), message: `${t('pages.startPrefix')}${dirLabel}${t('pages.process')}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('flip_images', {
        options: { input_path: inputPath, output_path: outputPath, direction },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      setHasError(true);
      setIsDone(true);
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
          <FlipHorizontal2 style={{ width: 28, height: 28, color: '#00d4ff' }} />
          <h1 className="page-title">{t('flip.title')}</h1>
        </div>
        <p className="page-subtitle">{t('flip.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('pages.pathSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">{t('pages.inputPath')}</label>
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

          {/* 翻转选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('flip.flipDirection')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {flipOptions.map((opt) => (
                <div key={opt.value} onClick={() => setDirection(opt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${direction === opt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: direction === opt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 20, height: 20, borderRadius: '50%', minWidth: 20, border: `2px solid ${direction === opt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {direction === opt.value && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <div style={{ width: 36, height: 36, borderRadius: 'var(--radius-sm)', minWidth: 36, background: direction === opt.value ? 'rgba(0, 212, 255, 0.12)' : 'rgba(255,255,255,0.04)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: direction === opt.value ? '#00d4ff' : 'var(--color-text-tertiary)' }}>
                    {opt.icon}
                  </div>
                  <div>
                    <div style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)', marginBottom: 2 }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>


        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_flip" startText={t('pages.startProcess')} processingText={t('pages.processing')}
            onCancelLog={addCancelLog} />

          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
