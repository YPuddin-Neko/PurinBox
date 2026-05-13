import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import { FileType, FolderOpen, Info } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

export default function FormatConvertPage() {
  const { t } = useTranslation();

  const targetFormats = [
    { value: 'png', label: 'PNG', desc: t('formatConvert.pngDesc'), color: '#4ade80' },
    { value: 'jpg', label: 'JPG', desc: t('formatConvert.jpgDesc'), color: '#ffa647' },
    { value: 'jpeg', label: 'JPEG', desc: t('formatConvert.jpegDesc'), color: '#ffa647' },
    { value: 'bmp', label: 'BMP', desc: t('formatConvert.bmpDesc'), color: '#f87171' },
    { value: 'webp', label: 'WebP', desc: t('formatConvert.webpDesc'), color: '#60a5fa' },
  ];

  const sourceFormats = ['PNG', 'JPG', 'JPEG', 'WebP', 'BMP', 'TIFF', 'GIF', 'PSD'];

  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [targetFormat, setTargetFormat] = useState('png');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('convert-progress', (event) => {
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
    addTask('convert', t('formatConvert.taskName'));
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: t('formatConvert.startConvertMsg', { format: targetFormat }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('convert_format', {
        options: { input_path: inputPath, output_path: outputPath, target_format: targetFormat },
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
          <FileType style={{ width: 28, height: 28, color: '#ffa647' }} />
          <h1 className="page-title">{t('formatConvert.title')}</h1>
        </div>
        <p className="page-subtitle">{t('formatConvert.subtitle')}</p>
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

          {/* 目标格式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('formatConvert.targetFormat')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {targetFormats.map((fmt) => (
                <div key={fmt.value} onClick={() => setTargetFormat(fmt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-3) var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${targetFormat === fmt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: targetFormat === fmt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 18, height: 18, borderRadius: '50%', minWidth: 18, border: `2px solid ${targetFormat === fmt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {targetFormat === fmt.value && <div style={{ width: 8, height: 8, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: targetFormat === fmt.value ? fmt.color : 'var(--color-text-tertiary)', minWidth: 50 }}>.{fmt.value}</span>
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{fmt.desc}</span>
                </div>
              ))}
            </div>
          </div>

          {/* 支持的源格式 */}
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(255, 166, 71, 0.04)', border: '1px solid rgba(255, 166, 71, 0.1)' }}>
            <Info style={{ width: 18, height: 18, color: '#ffa647', marginTop: 2, minWidth: 18 }} />
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
              <strong style={{ color: 'var(--color-text-primary)' }}>{t('formatConvert.supportedFormats')}</strong>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 4 }}>
                {sourceFormats.map((f) => (
                  <span key={f} style={{ fontSize: 'var(--font-size-xs)', padding: '1px 8px', borderRadius: 'var(--radius-full)', background: 'rgba(255, 255, 255, 0.04)', border: '1px solid var(--color-border)', color: 'var(--color-text-secondary)' }}>{f}</span>
                ))}
              </div>
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_convert" startText={t('formatConvert.startConvert')} processingText={t('formatConvert.converting')}
            onCancelLog={addCancelLog} />

          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
