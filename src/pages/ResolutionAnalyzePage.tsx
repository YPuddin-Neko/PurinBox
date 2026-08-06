import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import { open, save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { FolderOpen, BarChart3, Download, AlertCircle, FileText, FileSpreadsheet } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';
import RecursiveScanToggle from '../components/RecursiveScanToggle';

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

interface ProgressPayload {
  current: number;
  total: number;
  filename: string;
  status: string;
  message: string;
}

export default function ResolutionAnalyzePage() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [rareThreshold, setRareThreshold] = useState(10);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [result, setResult] = useState<AnalyzeResult | null>(null);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [exporting, setExporting] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('resolution-analyze-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) setProgress((d.current / d.total) * 100);
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
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
  }, []);

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
      setHasError(true);
    } finally {
      setProcessing(false);
    }
  };

  const handleExport = async (format: 'txt' | 'csv' | 'json') => {
    if (!result) return;
    const ext = format;
    const defaultFileName = `resolution_report_${new Date().getTime()}.${ext}`;
    const savePath = await save({
      defaultPath: defaultFileName,
      filters: [
        { name: format.toUpperCase(), extensions: [ext] },
        { name: 'All Files', extensions: ['*'] },
      ],
    });
    if (!savePath) return;

    setExporting(true);
    try {
      await invoke<string>('export_resolution_report', {
        result,
        format,
        savePath,
        inputPath,
      });
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: t('resolutionAnalyze.exportSuccess', { path: savePath }),
          status: 'success',
        },
      ]);
    } catch (e: any) {
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: t('resolutionAnalyze.exportError', { error: e?.message || e }),
          status: 'error',
        },
      ]);
    } finally {
      setExporting(false);
    }
  };

  const rareGroups = result?.groups.filter((g) => g.is_rare) || [];

  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <div style={{ padding: '20px 30px', borderBottom: '1px solid var(--color-border)' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <BarChart3 style={{ width: 24, height: 24, color: 'var(--color-primary)' }} />
          <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>{t('resolutionAnalyze.title')}</h1>
        </div>
        <p style={{ fontSize: 12, color: 'var(--color-text-secondary)', margin: 0 }}>
          {t('resolutionAnalyze.subtitle')}
        </p>
      </div>

      <div style={{ flex: 1, overflowY: 'auto', padding: '20px 30px' }}>
        {/* Input Controls */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 15, marginBottom: 20 }}>
          <div>
            <label style={{ fontSize: 13, fontWeight: 500, display: 'block', marginBottom: 6 }}>
              {t('resolutionAnalyze.inputPath')}
            </label>
            <div style={{ display: 'flex', gap: 8 }}>
              <input
                type="text"
                value={inputPath}
                readOnly
                placeholder={t('resolutionAnalyze.inputPathPlaceholder')}
                style={{
                  flex: 1,
                  padding: '8px 12px',
                  fontSize: 13,
                  border: '1px solid var(--color-border)',
                  borderRadius: 6,
                  background: 'var(--color-bg-secondary)',
                }}
              />
              <button onClick={handleOpenInput} className="btn btn-secondary" style={{ gap: 5 }}>
                <FolderOpen style={{ width: 14, height: 14 }} />
                {t('common.browse')}
              </button>
            </div>
          </div>

          <div style={{ display: 'flex', gap: 15, alignItems: 'flex-end' }}>
            <div style={{ flex: 1 }}>
              <label style={{ fontSize: 13, fontWeight: 500, display: 'block', marginBottom: 6 }}>
                {t('resolutionAnalyze.rareThreshold')}
              </label>
              <input
                type="number"
                value={rareThreshold}
                onChange={(e) => setRareThreshold(Math.max(1, parseInt(e.target.value) || 1))}
                min={1}
                style={{
                  width: '100%',
                  padding: '8px 12px',
                  fontSize: 13,
                  border: '1px solid var(--color-border)',
                  borderRadius: 6,
                }}
              />
              <p style={{ fontSize: 11, color: 'var(--color-text-tertiary)', margin: '4px 0 0' }}>
                {t('resolutionAnalyze.rareThresholdDesc')}
              </p>
            </div>
            <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
          </div>
        </div>

        {/* Action Button */}
        <div style={{ marginBottom: 20 }}>
          <ProcessButton
            processing={processing}
            onStart={handleAnalyze}
            cancelCommand="cancel_resolution_analyze"
            startText={t('resolutionAnalyze.analyze')}
            startIcon={<BarChart3 style={{ width: 16, height: 16 }} />}
            processingText={t('resolutionAnalyze.analyzing')}
            disabled={!inputPath}
            onCancelLog={(msg) => setLogs((prev) => [...prev, { time: getTimeStr(), message: msg, status: 'info' }])}
          />
        </div>

        {/* Progress */}
        {processing && (
          <div style={{ marginBottom: 20 }}>
            <div
              style={{
                width: '100%',
                height: 6,
                background: 'var(--color-bg-tertiary)',
                borderRadius: 3,
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  width: `${progress}%`,
                  height: '100%',
                  background: 'var(--color-primary)',
                  transition: 'width 0.2s',
                }}
              />
            </div>
            <p style={{ fontSize: 12, color: 'var(--color-text-secondary)', marginTop: 6 }}>
              {progressCurrent} / {progressTotal}
            </p>
          </div>
        )}

        {/* Results */}
        {result && !processing && (
          <div style={{ marginBottom: 20 }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                marginBottom: 12,
              }}
            >
              <h2 style={{ fontSize: 15, fontWeight: 600, margin: 0 }}>{t('resolutionAnalyze.results')}</h2>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  onClick={() => handleExport('txt')}
                  disabled={exporting}
                  className="btn btn-secondary"
                  style={{ fontSize: 12, padding: '6px 12px', gap: 4 }}
                >
                  <FileText style={{ width: 12, height: 12 }} />
                  TXT
                </button>
                <button
                  onClick={() => handleExport('csv')}
                  disabled={exporting}
                  className="btn btn-secondary"
                  style={{ fontSize: 12, padding: '6px 12px', gap: 4 }}
                >
                  <FileSpreadsheet style={{ width: 12, height: 12 }} />
                  CSV
                </button>
                <button
                  onClick={() => handleExport('json')}
                  disabled={exporting}
                  className="btn btn-secondary"
                  style={{ fontSize: 12, padding: '6px 12px', gap: 4 }}
                >
                  <Download style={{ width: 12, height: 12 }} />
                  JSON
                </button>
              </div>
            </div>

            {/* Summary Cards */}
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))', gap: 12, marginBottom: 16 }}>
              <div style={{ padding: 12, background: 'var(--color-bg-secondary)', borderRadius: 8, border: '1px solid var(--color-border)' }}>
                <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginBottom: 4 }}>{t('resolutionAnalyze.totalImages')}</div>
                <div style={{ fontSize: 20, fontWeight: 600 }}>{result.total_images}</div>
              </div>
              <div style={{ padding: 12, background: 'var(--color-bg-secondary)', borderRadius: 8, border: '1px solid var(--color-border)' }}>
                <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginBottom: 4 }}>{t('resolutionAnalyze.distinctCount')}</div>
                <div style={{ fontSize: 20, fontWeight: 600 }}>{result.distinct_count}</div>
              </div>
              <div style={{ padding: 12, background: 'var(--color-bg-secondary)', borderRadius: 8, border: '1px solid var(--color-border)' }}>
                <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginBottom: 4 }}>{t('resolutionAnalyze.widthRange')}</div>
                <div style={{ fontSize: 14, fontWeight: 600 }}>{result.min_width} - {result.max_width}</div>
              </div>
              <div style={{ padding: 12, background: 'var(--color-bg-secondary)', borderRadius: 8, border: '1px solid var(--color-border)' }}>
                <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginBottom: 4 }}>{t('resolutionAnalyze.heightRange')}</div>
                <div style={{ fontSize: 14, fontWeight: 600 }}>{result.min_height} - {result.max_height}</div>
              </div>
            </div>

            {/* Groups Table */}
            <div style={{ border: '1px solid var(--color-border)', borderRadius: 8, overflow: 'hidden' }}>
              <div style={{ maxHeight: 400, overflowY: 'auto' }}>
                <table style={{ width: '100%', fontSize: 12, borderCollapse: 'collapse' }}>
                  <thead style={{ position: 'sticky', top: 0, background: 'var(--color-bg-secondary)', borderBottom: '1px solid var(--color-border)' }}>
                    <tr>
                      <th style={{ padding: '8px 12px', textAlign: 'left', fontWeight: 600 }}>{t('resolutionAnalyze.resolution')}</th>
                      <th style={{ padding: '8px 12px', textAlign: 'right', fontWeight: 600 }}>{t('resolutionAnalyze.count')}</th>
                      <th style={{ padding: '8px 12px', textAlign: 'right', fontWeight: 600 }}>{t('resolutionAnalyze.percent')}</th>
                      <th style={{ padding: '8px 12px', textAlign: 'left', fontWeight: 600 }}>{t('resolutionAnalyze.aspectRatio')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {result.groups.map((g, i) => (
                      <tr key={i} style={{ borderBottom: i < result.groups.length - 1 ? '1px solid var(--color-border-light)' : 'none', background: g.is_rare ? 'rgba(251, 191, 36, 0.05)' : 'transparent' }}>
                        <td style={{ padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 6 }}>
                          {g.is_rare && <AlertCircle style={{ width: 12, height: 12, color: '#f59e0b', flexShrink: 0 }} />}
                          <span>{g.width} × {g.height}</span>
                        </td>
                        <td style={{ padding: '8px 12px', textAlign: 'right' }}>{g.count}</td>
                        <td style={{ padding: '8px 12px', textAlign: 'right' }}>{g.percent.toFixed(1)}%</td>
                        <td style={{ padding: '8px 12px' }}>{g.aspect_label || g.aspect_ratio.toFixed(2)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Rare Files List */}
            {rareGroups.length > 0 && (
              <div style={{ marginTop: 16, padding: 12, background: 'rgba(251, 191, 36, 0.08)', border: '1px solid rgba(251, 191, 36, 0.2)', borderRadius: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}>
                  <AlertCircle style={{ width: 14, height: 14, color: '#f59e0b' }} />
                  <h3 style={{ fontSize: 13, fontWeight: 600, margin: 0 }}>{t('resolutionAnalyze.rareResolutions')}</h3>
                </div>
                <div style={{ maxHeight: 200, overflowY: 'auto', fontSize: 11 }}>
                  {rareGroups.map((g, gi) => (
                    <div key={gi} style={{ marginBottom: 12 }}>
                      <div style={{ fontWeight: 600, marginBottom: 4 }}>
                        {g.width} × {g.height} ({g.count} {t('resolutionAnalyze.images')})
                      </div>
                      {g.files.map((f, fi) => (
                        <div key={fi} style={{ paddingLeft: 12, color: 'var(--color-text-secondary)', wordBreak: 'break-all' }}>
                          • {f}
                        </div>
                      ))}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {result.failed_count > 0 && (
              <div style={{ marginTop: 12, padding: 12, background: 'rgba(239, 68, 68, 0.08)', border: '1px solid rgba(239, 68, 68, 0.2)', borderRadius: 8 }}>
                <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 8 }}>{t('resolutionAnalyze.failedFiles', { count: result.failed_count })}</h3>
                <div style={{ maxHeight: 120, overflowY: 'auto', fontSize: 11, color: 'var(--color-text-secondary)' }}>
                  {result.failed_files.map((f, i) => (
                    <div key={i} style={{ marginBottom: 2 }}>• {f}</div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}

        {/* Logs */}
        <ProgressLog 
          progress={progress} 
          current={progressCurrent} 
          total={progressTotal} 
          logs={logs} 
          isDone={isDone} 
          hasError={hasError} 
        />
      </div>
    </div>
  );
}
