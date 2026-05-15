import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  Scaling,
  FolderOpen,
  ArrowUpCircle,
  ArrowDownCircle,
  Info,
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

export default function ScalePage() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [enableUpscale, setEnableUpscale] = useState(true);
  const [enableDownscale, setEnableDownscale] = useState(false);
  const [upWidth, setUpWidth] = useState(1024);
  const [upHeight, setUpHeight] = useState(1024);
  const [downWidth, setDownWidth] = useState(512);
  const [downHeight, setDownHeight] = useState(512);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  // Listen for progress events
  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('scale-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) {
        setProgress((d.current / d.total) * 100);
      }
      if (d.status === 'done') {
        setIsDone(true);
      }
      if (d.status === 'error') {
        setHasError(true);
      }
      // Only add meaningful log entries (not processing status)
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

  // Determine mode string
  const getMode = () => {
    if (enableUpscale && enableDownscale) return 'both';
    if (enableUpscale) return 'upscale';
    if (enableDownscale) return 'downscale';
    return '';
  };

  const getModeLabel = () => {
    const mode = getMode();
    if (mode === 'both') return t('scale.startBoth');
    if (mode === 'upscale') return t('scale.startUp');
    if (mode === 'downscale') return t('scale.startDown');
    return '';
  };

  const handleProcess = async () => {
    const mode = getMode();
    if (!inputPath || !outputPath || !mode) return;
    setProcessing(true);
    addTask('scale', t('scale.taskName'));
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);

    const targetW = mode === 'downscale' ? downWidth : upWidth;
    const targetH = mode === 'downscale' ? downHeight : upHeight;
    const label = getModeLabel();
    setLogs([{ time: getTimeStr(), message: `${t('pages.startPrefix')}${label}${t('pages.process')} → ${targetW}×${targetH}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('scale_images', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode,
          target_width: targetW,
          target_height: targetH,
          down_target_width: mode === 'both' ? downWidth : 0,
          down_target_height: mode === 'both' ? downHeight : 0,
        },
      });
      // done
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

  const canStart = inputPath && outputPath && (enableUpscale || enableDownscale);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Scaling style={{ width: 28, height: 28, color: '#818cf8' }} />
          <h1 className="page-title">{t('scale.title')}</h1>
        </div>
        <p className="page-subtitle">{t('scale.subtitle')}</p>
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

          {/* 缩放选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('scale.scaleOptions')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {/* 上采样 */}
              <div onClick={() => setEnableUpscale(!enableUpscale)} style={{
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${enableUpscale ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                background: enableUpscale ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                  <div style={{
                    width: 18, height: 18, borderRadius: 4,
                    border: `2px solid ${enableUpscale ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`,
                    background: enableUpscale ? 'var(--color-accent-primary)' : 'transparent',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    transition: 'all 0.15s',
                  }}>
                    {enableUpscale && <span style={{ color: '#fff', fontSize: 12, fontWeight: 700, lineHeight: 1 }}>✓</span>}
                  </div>
                  <ArrowUpCircle style={{ width: 18, height: 18, color: enableUpscale ? '#4ade80' : 'var(--color-text-tertiary)' }} />
                  <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{t('scale.upscale')}</span>
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', opacity: enableUpscale ? 1 : 0.4, pointerEvents: enableUpscale ? 'auto' : 'none' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.width')}</label>
                    <input className="form-input" type="number" value={upWidth} onChange={(e) => setUpWidth(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.height')}</label>
                    <input className="form-input" type="number" value={upHeight} onChange={(e) => setUpHeight(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(74, 222, 128, 0.06)', border: '1px solid rgba(74, 222, 128, 0.1)' }}>
                  <Info style={{ width: 14, height: 14, color: '#4ade80', marginTop: 2, minWidth: 14 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                    {t('scale.upscaleDesc')}
                  </span>
                </div>
              </div>

              {/* 下采样 */}
              <div onClick={() => setEnableDownscale(!enableDownscale)} style={{
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${enableDownscale ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                background: enableDownscale ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                  <div style={{
                    width: 18, height: 18, borderRadius: 4,
                    border: `2px solid ${enableDownscale ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`,
                    background: enableDownscale ? 'var(--color-accent-primary)' : 'transparent',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    transition: 'all 0.15s',
                  }}>
                    {enableDownscale && <span style={{ color: '#fff', fontSize: 12, fontWeight: 700, lineHeight: 1 }}>✓</span>}
                  </div>
                  <ArrowDownCircle style={{ width: 18, height: 18, color: enableDownscale ? '#60a5fa' : 'var(--color-text-tertiary)' }} />
                  <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{t('scale.downscale')}</span>
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', opacity: enableDownscale ? 1 : 0.4, pointerEvents: enableDownscale ? 'auto' : 'none' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.width')}</label>
                    <input className="form-input" type="number" value={downWidth} onChange={(e) => setDownWidth(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">{t('scale.height')}</label>
                    <input className="form-input" type="number" value={downHeight} onChange={(e) => setDownHeight(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(96, 165, 250, 0.06)', border: '1px solid rgba(96, 165, 250, 0.1)' }}>
                  <Info style={{ width: 14, height: 14, color: '#60a5fa', marginTop: 2, minWidth: 14 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                    {t('scale.downscaleDesc')}
                  </span>
                </div>
              </div>

              {/* 提示文字 */}
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(251, 191, 36, 0.06)', border: '1px solid rgba(251, 191, 36, 0.15)' }}>
                <Info style={{ width: 14, height: 14, color: '#fbbf24', marginTop: 2, minWidth: 14 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  {t('scale.resizeHint')}
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 执行按钮 */}
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!canStart}
            cancelCommand="cancel_scale" startText={t('scale.startScale')} processingText={t('pages.processing')}
            onCancelLog={addCancelLog} />

          {/* 进度条和日志 */}
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
