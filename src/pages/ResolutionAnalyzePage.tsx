import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import { open, save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { FolderOpen, Download, AlertCircle } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';
import RecursiveScanToggle from '../components/RecursiveScanToggle';
import { useTaskQueue } from '../components/TaskContext';

interface ResolutionGroup {
  width: number;
  height: number;
  count: number;
  percent: number;
  aspect_ratio: number;
  aspect_label: string;
  is_rare: boolean;
  files: string[];
}

interface AnalyzeResult {
  total_images: number;
  failed_count: number;
  failed_files: string[];
  distinct_count: number;
  groups: ResolutionGroup[];
  min_width: number;
  max_width: number;
  min_height: number;
  max_height: number;
}

export default function ResolutionAnalyzePage() {
  const { t } = useTranslation();
  const { addTask, updateTask } = useTaskQueue();
  const [inputPath, setInputPath] = useState('');
  const [rareThreshold, setRareThreshold] = useState(10);
  const [recursive, setRecursive] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [result, setResult] = useState<AnalyzeResult | null>(null);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [expandedRare, setExpandedRare] = useState<Set<string>>(new Set());

  useEffect(() => {
    let active = true;
    const p = listen('resolution_analyze_progress', (event: any) => {
      if (!active) return;
      const d = event.payload;
      if (d.status === 'processing') {
        setProgress(d.progress ?? 0);
        setProgressCurrent(d.current ?? 0);
        setProgressTotal(d.total ?? 0);
      } else if (d.status === 'done') {
        setProcessing(false);
        setIsDone(true);
        updateTask('resolution-analyze', { status: 'done' });
      } else if (d.status === 'error') {
        setProcessing(false);
        setHasError(true);
        updateTask('resolution-analyze', { status: 'error', message: d.message });
      }
      if (d.status !== 'processing') {
        setLogs((prev) => [
          ...prev,
          {
            time: getTimeStr(),
            message: d.message,
            status: d.status === 'done' ? 'info' : (d.status as LogEntry['status']),
          },
        ]);
      }
    });
    return () => {
      active = false;
      p.then((unlisten) => unlisten());
    };
  }, [updateTask]);

  const handleOpenInput = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setInputPath(selected);
  };

  const handleAnalyze = async () => {
    if (!inputPath) return;
    setProcessing(true);
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setLogs([]);
    setResult(null);
    setIsDone(false);
    setHasError(false);
    setExpandedRare(new Set());

    addTask('resolution-analyze', t('resolutionAnalyze.taskName'));
    setLogs([{
      time: getTimeStr(),
      message: `${t('pages.startPrefix')}${t('resolutionAnalyze.analyzing')}...`,
      status: 'info',
    }]);

    try {
      const res = await invoke<AnalyzeResult>('analyze_resolutions', {
        options: {
          input_path: inputPath,
          rare_threshold: rareThreshold,
          recursive,
        },
      });
      setResult(res);
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: t('resolutionAnalyze.analyzeComplete', {
            total: res.total_images,
            distinct: res.distinct_count,
          }),
          status: 'success',
        },
      ]);
    } catch (e: any) {
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: typeof e === 'string' ? e : e?.message || t('resolutionAnalyze.analyzeError'),
          status: 'error',
        },
      ]);
      setProcessing(false);
      setHasError(true);
      updateTask('resolution-analyze', { status: 'error', message: String(e) });
    }
  };

  const handleExport = async (format: 'txt' | 'csv' | 'json') => {
    if (!result) return;
    const filters = format === 'txt'
      ? [{ name: 'Text', extensions: ['txt'] }]
      : format === 'csv'
      ? [{ name: 'CSV', extensions: ['csv'] }]
      : [{ name: 'JSON', extensions: ['json'] }];

    const savePath = await save({ filters, defaultPath: `resolution_report.${format}` });
    if (!savePath) return;

    try {
      const msg = await invoke<string>('export_resolution_report', {
        result,
        format,
        savePath,
        inputPath,
      });
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: msg,
          status: 'success',
        },
      ]);
    } catch (e: any) {
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: typeof e === 'string' ? e : e?.message || t('resolutionAnalyze.exportError'),
          status: 'error',
        },
      ]);
    }
  };

  const clearLogs = () => {
    setLogs([]);
    setIsDone(false);
    setHasError(false);
  };

  const toggleRareExpand = (key: string) => {
    setExpandedRare((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">{t('resolutionAnalyze.title')}</h1>
        </div>
        <p className="page-subtitle">{t('resolutionAnalyze.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-5)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('pages.pathSettings')}</span>
            </div>
            <div className="tool-panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <div className="form-label-row">
                  <label className="form-label">{t('pages.inputPathShort')}</label>
                  <button className="form-label-action" onClick={handleOpenInput}>
                    <FolderOpen size={12} />
                    {t('pages.selectInputFolderOption')}
                  </button>
                </div>
                <input
                  className="form-input"
                  value={inputPath}
                  onChange={(e) => setInputPath(e.target.value)}
                  placeholder={t('pages.selectInputFolder')}
                />
              </div>

              <div className="form-group">
                <label className="form-label">{t('resolutionAnalyze.rareThreshold')}</label>
                <input
                  className="form-input"
                  type="number"
                  value={rareThreshold}
                  onChange={(e) => setRareThreshold(Math.max(1, parseInt(e.target.value) || 1))}
                  onBlur={(e) => { if (e.target.value === "") setRareThreshold(10); }}
                  min={1}
                />
                <p style={{ fontSize: 11, color: 'var(--color-text-tertiary)', margin: '4px 0 0' }}>
                  {t('resolutionAnalyze.rareThresholdDesc')}
                </p>
              </div>

              <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
            </div>
          </div>

          {result && (
            <div className="tool-panel">
              <div className="tool-panel-header" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <span className="tool-panel-title">{t('resolutionAnalyze.analysisResults')}</span>
                <div style={{ display: 'flex', gap: 6 }}>
                  <button
                    className="icon-btn"
                    onClick={() => handleExport('txt')}
                    title={t('resolutionAnalyze.exportTXT')}
                    style={{ padding: '4px 8px', fontSize: 11 }}
                  >
                    <Download size={12} />
                    TXT
                  </button>
                  <button
                    className="icon-btn"
                    onClick={() => handleExport('csv')}
                    title={t('resolutionAnalyze.exportCSV')}
                    style={{ padding: '4px 8px', fontSize: 11 }}
                  >
                    <Download size={12} />
                    CSV
                  </button>
                  <button
                    className="icon-btn"
                    onClick={() => handleExport('json')}
                    title={t('resolutionAnalyze.exportJSON')}
                    style={{ padding: '4px 8px', fontSize: 11 }}
                  >
                    <Download size={12} />
                    JSON
                  </button>
                </div>
              </div>
              <div className="tool-panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
                {/* 概览卡片 */}
                <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 8 }}>
                  <div style={{
                    padding: 12,
                    borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--color-border)',
                    background: 'var(--color-bg-input)',
                  }}>
                    <div style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4 }}>
                      {t('resolutionAnalyze.totalImages')}
                    </div>
                    <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--color-text-primary)' }}>
                      {result.total_images.toLocaleString()}
                    </div>
                  </div>
                  <div style={{
                    padding: 12,
                    borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--color-border)',
                    background: 'var(--color-bg-input)',
                  }}>
                    <div style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4 }}>
                      {t('resolutionAnalyze.distinctResolutions')}
                    </div>
                    <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--color-text-primary)' }}>
                      {result.distinct_count}
                    </div>
                  </div>
                  <div style={{
                    padding: 12,
                    borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--color-border)',
                    background: 'var(--color-bg-input)',
                  }}>
                    <div style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4 }}>
                      {t('resolutionAnalyze.sizeRange')}
                    </div>
                    <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--color-text-primary)' }}>
                      {result.min_width}×{result.min_height} ~ {result.max_width}×{result.max_height}
                    </div>
                  </div>
                  <div style={{
                    padding: 12,
                    borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--color-border)',
                    background: 'var(--color-bg-input)',
                  }}>
                    <div style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4 }}>
                      {t('resolutionAnalyze.readErrors')}
                    </div>
                    <div style={{ fontSize: 20, fontWeight: 700, color: result.failed_count > 0 ? '#ef4444' : 'var(--color-text-primary)' }}>
                      {result.failed_count}
                    </div>
                  </div>
                </div>

                {/* 分辨率列表 */}
                <div>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)', marginBottom: 8 }}>
                    {t('resolutionAnalyze.resolutionDistribution')}
                  </div>
                  <div style={{
                    maxHeight: 400,
                    overflowY: 'auto',
                    overscrollBehavior: 'contain',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 6,
                  }}>
                    {result.groups.map((g) => {
                      const key = `${g.width}x${g.height}`;
                      const isExpanded = expandedRare.has(key);
                      return (
                        <div
                          key={key}
                          style={{
                            padding: 10,
                            borderRadius: 'var(--radius-sm)',
                            border: g.is_rare ? '1px solid #ef4444' : '1px solid var(--color-border)',
                            background: g.is_rare ? 'rgba(239, 68, 68, 0.05)' : 'var(--color-bg-input)',
                          }}
                        >
                          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                              {g.is_rare && <AlertCircle size={14} style={{ color: '#ef4444', flexShrink: 0 }} />}
                              <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--color-text-primary)' }}>
                                {g.width} × {g.height}
                              </span>
                              <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>
                                {g.aspect_label}
                              </span>
                            </div>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                              <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>
                                {g.count}
                              </span>
                              <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>
                                ({g.percent.toFixed(1)}%)
                              </span>
                            </div>
                          </div>
                          {g.is_rare && g.files.length > 0 && (
                            <>
                              <button
                                onClick={() => toggleRareExpand(key)}
                                style={{
                                  marginTop: 6,
                                  fontSize: 11,
                                  color: '#ef4444',
                                  background: 'none',
                                  border: 'none',
                                  padding: 0,
                                  cursor: 'pointer',
                                  textDecoration: 'underline',
                                }}
                              >
                                {isExpanded ? t('resolutionAnalyze.hideFiles') : t('resolutionAnalyze.showFiles', { count: g.files.length })}
                              </button>
                              {isExpanded && (
                                <div style={{
                                  marginTop: 6,
                                  padding: 8,
                                  borderRadius: 4,
                                  background: 'var(--color-bg)',
                                  fontSize: 10,
                                  color: 'var(--color-text-secondary)',
                                  maxHeight: 120,
                                  overflowY: 'auto',
                                  overscrollBehavior: 'contain',
                                  wordBreak: 'break-all',
                                }}>
                                  {g.files.map((f, i) => (
                                    <div key={i} style={{ marginBottom: 2 }}>{f}</div>
                                  ))}
                                </div>
                              )}
                            </>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>

                {/* 读取失败文件 */}
                {result.failed_count > 0 && (
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 600, color: '#ef4444', marginBottom: 8 }}>
                      {t('resolutionAnalyze.failedFiles')} ({result.failed_count})
                    </div>
                    <div style={{
                      padding: 10,
                      borderRadius: 'var(--radius-sm)',
                      border: '1px solid #ef4444',
                      background: 'rgba(239, 68, 68, 0.05)',
                      fontSize: 11,
                      color: 'var(--color-text-secondary)',
                      maxHeight: 150,
                      overflowY: 'auto',
                      overscrollBehavior: 'contain',
                      wordBreak: 'break-all',
                    }}>
                      {result.failed_files.map((f, i) => (
                        <div key={i} style={{ marginBottom: 2 }}>{f}</div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton
            processing={processing}
            onStart={handleAnalyze}
            disabled={!inputPath}
            cancelCommand="cancel_resolution_analyze"
            startText={t('resolutionAnalyze.startAnalyze')}
            processingText={t('pages.processing')}
            onCancelLog={(msg) => setLogs((prev) => [...prev, { time: getTimeStr(), message: msg, status: 'warning' }])}
          />

          <ProgressLog
            progress={progress}
            current={progressCurrent}
            total={progressTotal}
            logs={logs}
            isDone={isDone}
            hasError={hasError}
            onClearLogs={clearLogs}
          />
        </div>
      </div>
    </div>
  );
}
