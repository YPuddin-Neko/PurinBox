import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  TextCursorInput,
  FolderOpen,
  Play,
  Loader2,
  Eye,
  Shuffle,
  ArrowRight,
  Hash,
  Type,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface PreviewItem { original: string; renamed: string; }

export default function BatchRenamePage() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [prefix, setPrefix] = useState('img_');
  const [startNumber, setStartNumber] = useState(1);
  const [digitCount, setDigitCount] = useState(4);
  const [shuffleOrder, setShuffleOrder] = useState(false);
  const [previews, setPreviews] = useState<PreviewItem[]>([]);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('rename-progress', (event) => {
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

  const selectFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('pages.selectInputTitle') });
    if (selected) {
      setInputPath(selected as string);
      setPreviews([]);
    }
  };

  const handlePreview = async () => {
    if (!inputPath) return;
    setPreviewLoading(true);
    try {
      const result = await invoke<PreviewItem[]>('preview_rename', {
        options: { input_path: inputPath, prefix, start_number: startNumber, digit_count: digitCount, shuffle: shuffleOrder },
      });
      setPreviews(result);
    } catch (e: any) {
      setPreviews([]);
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('batchRename.previewFailed')}: ${String(e)}`, status: 'error' }]);
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleShuffle = async () => {
    if (!inputPath) return;
    setPreviewLoading(true);
    try {
      const result = await invoke<PreviewItem[]>('preview_rename', {
        options: { input_path: inputPath, prefix, start_number: startNumber, digit_count: digitCount, shuffle: true },
      });
      setPreviews(result);
      setShuffleOrder(true);
    } catch (e: any) {
      setPreviews([]);
    } finally {
      setPreviewLoading(false);
    }
  };

  const { addTask } = useTaskQueue();

  const handleExecute = async () => {
    if (!inputPath || previews.length === 0) return;
    setProcessing(true);
    addTask('rename', t('batchRename.taskName'));
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: t('batchRename.startMsg', { prefix, start: startNumber, digits: digitCount }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('execute_rename', {
        options: { input_path: inputPath, prefix, start_number: startNumber, digit_count: digitCount, shuffle: shuffleOrder },
      });
      setPreviews([]);
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);

  // Example preview string
  const exampleNum = String(startNumber).padStart(digitCount, '0');
  const exampleName = `${prefix}${exampleNum}.png`;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <TextCursorInput style={{ width: 28, height: 28, color: '#38bdf8' }} />
          <h1 className="page-title">{t('batchRename.title')}</h1>
        </div>
        <p className="page-subtitle">{t('batchRename.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 400px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('batchRename.imageFolder')}</span></div>
            <div className="form-group">
              <label className="form-label">{t('batchRename.folderDesc')}</label>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <input className="form-input" placeholder={t('batchRename.selectFolder')} value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                <button className="btn btn-secondary" onClick={selectFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
              </div>
            </div>
          </div>

          {/* 命名规则 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('batchRename.namingRule')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Type style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                  {t('batchRename.prefix')}
                </label>
                <input className="form-input" value={prefix} onChange={(e) => setPrefix(e.target.value)} placeholder={t('batchRename.prefixPlaceholder')} />
              </div>
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                    {t('batchRename.startNum')}
                  </label>
                  <input className="form-input" type="number" value={startNumber} onChange={(e) => setStartNumber(Number(e.target.value))} min={0} />
                </div>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                    {t('batchRename.digitCount')}
                  </label>
                  <input className="form-input" type="number" value={digitCount} onChange={(e) => setDigitCount(Number(e.target.value))} min={1} max={10} />
                </div>
              </div>

              {/* 命名示例 */}
              <div style={{
                padding: 'var(--space-3) var(--space-4)',
                borderRadius: 'var(--radius-md)',
                background: 'rgba(56, 189, 248, 0.06)',
                border: '1px solid rgba(56, 189, 248, 0.12)',
                display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
              }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>{t('batchRename.namingExample')}</span>
                <span style={{ fontSize: 'var(--font-size-md)', fontWeight: 700, color: '#38bdf8', fontFamily: 'monospace' }}>{exampleName}</span>
              </div>
            </div>
          </div>

          {/* 操作按钮 */}
          <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
            <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={handlePreview} disabled={!inputPath || previewLoading}>
              {previewLoading ? <Loader2 style={{ width: 16, height: 16, animation: 'spin 1s linear infinite' }} /> : <Eye style={{ width: 16, height: 16 }} />}
              {t('batchRename.generatePreview')}
            </button>
            <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={handleShuffle} disabled={!inputPath || previewLoading}>
              <Shuffle style={{ width: 16, height: 16 }} />
              {t('batchRename.shufflePreview')}
            </button>
          </div>

          {/* 预览表格 */}
          {previews.length > 0 && (
            <div className="tool-panel" style={{ padding: 0, overflow: 'hidden' }}>
              <div className="tool-panel-header" style={{ padding: 'var(--space-3) var(--space-4)' }}>
                <span className="tool-panel-title">{t('batchRename.previewTitle')}</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{t('batchRename.fileCount', { count: previews.length })}</span>
              </div>
              <div style={{ maxHeight: 360, overflowY: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                  <thead>
                    <tr style={{ borderBottom: '1px solid var(--color-border)' }}>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>#</th>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('batchRename.originalName')}</th>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'center', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', width: 30 }}></th>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('batchRename.newName')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {previews.map((item, idx) => (
                      <tr key={idx} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                        <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>{idx + 1}</td>
                        <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', fontFamily: 'monospace', wordBreak: 'break-all' }}>{item.original}</td>
                        <td style={{ padding: '6px 0', textAlign: 'center' }}><ArrowRight style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /></td>
                        <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: '#38bdf8', fontFamily: 'monospace', fontWeight: 600 }}>{item.renamed}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleExecute}
            disabled={processing || !inputPath || previews.length === 0}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> {t('batchRename.executing')}</> : <><Play style={{ width: 18, height: 18 }} /> {t('batchRename.executeRename')}</>}
          </button>


          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
