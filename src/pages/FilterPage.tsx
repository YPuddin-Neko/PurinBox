import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  ScanSearch,
  FolderOpen,
  Info,
  Trash2,
  Copy,
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

type ConditionType = 'min_width' | 'min_height' | 'below_resolution' | 'above_resolution';
type ActionType = 'copy' | 'delete';


export default function FilterPage() {
  const { t } = useTranslation();

  const conditionOptions: { value: ConditionType; label: string; desc: string }[] = [
    { value: 'min_width', label: t('filter.condMinWidth'), desc: t('filter.condMinWidthDesc') },
    { value: 'min_height', label: t('filter.condMinHeight'), desc: t('filter.condMinHeightDesc') },
    { value: 'below_resolution', label: t('filter.condBelowRes'), desc: t('filter.condBelowResDesc') },
    { value: 'above_resolution', label: t('filter.condAboveRes'), desc: t('filter.condAboveResDesc') },
  ];
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [action, setAction] = useState<ActionType>('copy');
  const [condition, setCondition] = useState<ConditionType>('below_resolution');
  const [width, setWidth] = useState(512);
  const [height, setHeight] = useState(512);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  const needsOutput = action === 'copy';
  const needsWidth = condition !== 'min_height';
  const needsHeight = condition !== 'min_width';

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('filter-progress', (event) => {
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
    const selected = await open({ directory: true, multiple: false, title: t('filter.selectSaveFolder') });
    if (selected) setOutputPath(selected as string);
  };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath) return;
    if (action === 'copy' && !outputPath) return;
    setProcessing(true);
    addTask('filter', t('filter.taskName'));
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);
    const condLabel = conditionOptions.find((c) => c.value === condition)!.label;
    setLogs([{ time: getTimeStr(), message: `${t('pages.startPrefix')}${t('filter.startFilter')}: ${condLabel}, ${action === 'copy' ? t('filter.actionCopy') : t('filter.actionDelete')}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('filter_by_resolution', {
        options: { input_path: inputPath, output_path: outputPath || inputPath, action, condition, width, height },
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
          <ScanSearch style={{ width: 28, height: 28, color: '#ff6b9d' }} />
          <h1 className="page-title">{t('filter.title')}</h1>
        </div>
        <p className="page-subtitle">{t('filter.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('pages.pathSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">{t('filter.inputFolder')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectInputFolder')} value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              {needsOutput && (
                <div className="form-group">
                  <label className="form-label">{t('filter.savePath')}</label>
                  <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                    <input className="form-input" placeholder={t('filter.selectSaveFolder')} value={outputPath} onChange={(e) => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                    <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* 操作方式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('filter.actionMode')}</span></div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-3)' }}>
              <div onClick={() => setAction('copy')} style={{
                display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)',
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${action === 'copy' ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                background: action === 'copy' ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <Copy style={{ width: 24, height: 24, color: action === 'copy' ? '#4ade80' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{t('filter.actionCopy')}</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{t('filter.actionCopyDesc')}</span>
              </div>
              <div onClick={() => setAction('delete')} style={{
                display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)',
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${action === 'delete' ? 'rgba(248, 113, 113, 0.5)' : 'var(--color-border)'}`,
                background: action === 'delete' ? 'rgba(248, 113, 113, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <Trash2 style={{ width: 24, height: 24, color: action === 'delete' ? '#f87171' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{t('filter.actionDelete')}</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{t('filter.actionDeleteDesc')}</span>
              </div>
            </div>
            {action === 'delete' && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(248, 113, 113, 0.06)', border: '1px solid rgba(248, 113, 113, 0.15)', marginTop: 'var(--space-3)' }}>
                <Info style={{ width: 14, height: 14, color: '#f87171', minWidth: 14 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: '#f87171' }}>{t('filter.deleteWarning')}</span>
              </div>
            )}
          </div>

          {/* 筛选条件 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('filter.filterCondition')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {conditionOptions.map((opt) => (
                <div key={opt.value} onClick={() => setCondition(opt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-3) var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${condition === opt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: condition === opt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 18, height: 18, borderRadius: '50%', minWidth: 18, border: `2px solid ${condition === opt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {condition === opt.value && <div style={{ width: 8, height: 8, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <div>
                    <div style={{ fontWeight: 600, fontSize: 'var(--font-size-base)', color: 'var(--color-text-primary)' }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </div>
              ))}
            </div>

            <div style={{ display: 'flex', gap: 'var(--space-3)', marginTop: 'var(--space-4)' }}>
              {needsWidth && (
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{condition === 'min_width' ? t('filter.minWidthPx') : t('filter.widthPx')}</label>
                  <input className="form-input" type="number" value={width} onChange={(e) => setWidth(Number(e.target.value))} min={1} />
                </div>
              )}
              {needsHeight && (
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{condition === 'min_height' ? t('filter.minHeightPx') : t('filter.heightPx')}</label>
                  <input className="form-input" type="number" value={height} onChange={(e) => setHeight(Number(e.target.value))} min={1} />
                </div>
              )}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>


          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || (action === 'copy' && !outputPath)}
            cancelCommand="cancel_filter"
            startText={action === 'delete' ? t('filter.startFilterDelete') : t('filter.startFilterOutput')}
            processingText={t('pages.processing')}
            onCancelLog={addCancelLog} />

          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
