import { useState, useEffect, useRef, useMemo } from 'react';
import { CheckCircle2, XCircle, Loader2, Info, ScrollText, Trash2, Download, Timer, AlertTriangle } from 'lucide-react';
import '../styles/progress.css';
import { useTranslation } from 'react-i18next';

export interface LogEntry {
  time: string;
  message: string;
  status: 'success' | 'error' | 'processing' | 'info' | 'download' | 'warning';
  /** 下载专用字段 */
  dlPercent?: number;
  dlSpeed?: string;
}

interface ProgressLogProps {
  progress: number; // 0 - 100
  current: number;
  total: number;
  logs: LogEntry[];
  isDone: boolean;
  hasError: boolean;
  onClearLogs?: () => void;
  /** 外部传入的开始时间戳，优先使用 */
  externalStartTime?: number;
}

function getTimeStr(): string {
  const now = new Date();
  return `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}`;
}

function formatElapsed(ms: number): string {
  const sec = Math.floor(ms / 1000);
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return m > 0 ? `${m}m${s}s` : `${s}s`;
}

export { getTimeStr };

const MAX_LOGS = 500;

export default function ProgressLog({ progress, current, total, logs, isDone, hasError, onClearLogs, externalStartTime }: ProgressLogProps) {
  const { t } = useTranslation();
  const logEndRef = useRef<HTMLDivElement>(null);
  const [startTime, setStartTime] = useState<number>(0);
  const [elapsed, setElapsed] = useState('');

  // 记录开始时间（外部优先）
  useEffect(() => {
    if (externalStartTime && externalStartTime > 0) {
      setStartTime(externalStartTime);
    } else if (current >= 1 && startTime === 0) {
      setStartTime(Date.now());
    }
    if (current === 0 && !externalStartTime) {
      setStartTime(0);
      setElapsed('');
    }
  }, [current, startTime, externalStartTime]);

  // 实时计时器
  useEffect(() => {
    if (startTime === 0 || isDone) return;
    const timer = setInterval(() => {
      setElapsed(formatElapsed(Date.now() - startTime));
    }, 1000);
    return () => clearInterval(timer);
  }, [startTime, isDone]);

  // 完成时定格耗时
  useEffect(() => {
    if (isDone && startTime > 0) {
      setElapsed(formatElapsed(Date.now() - startTime));
    }
  }, [isDone, startTime]);

  // Auto-scroll to bottom only if user is near bottom
  const logContainerRef = useRef<HTMLDivElement>(null);
  const isNearBottomRef = useRef(true);

  const handleScroll = () => {
    const el = logContainerRef.current;
    if (!el) return;
    isNearBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  };

  useEffect(() => {
    if (isNearBottomRef.current && logContainerRef.current) {
      logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight;
    }
  }, [logs.length, logs[logs.length - 1]?.dlPercent]);

  // 计算速度
  const getSpeed = () => {
    if (startTime === 0 || current <= 0) return '';
    const el = (Date.now() - startTime) / 1000;
    if (el < 0.5) return '';
    const speed = current / el;
    return speed >= 1 ? `${speed.toFixed(1)} it/s` : `${(1 / speed).toFixed(1)} s/it`;
  };

  const speed = getSpeed();

  // Cap logs to prevent unbounded memory/DOM growth
  const displayLogs = useMemo(() => {
    if (logs.length <= MAX_LOGS) return logs;
    return logs.slice(logs.length - MAX_LOGS);
  }, [logs]);
  const truncated = logs.length - displayLogs.length;

  const statusIcon = (status: LogEntry['status']) => {
    switch (status) {
      case 'success':
        return <CheckCircle2 className="log-entry-icon success" />;
      case 'error':
        return <XCircle className="log-entry-icon error" />;
      case 'processing':
        return <Loader2 className="log-entry-icon processing" />;
      case 'download':
        return <Download className="log-entry-icon info" style={{ animation: 'pulse 1.5s infinite' }} />;
      case 'warning':
        return <AlertTriangle className="log-entry-icon" style={{ color: '#fbbf24' }} />;
      case 'info':
      default:
        return <Info className="log-entry-icon info" />;
    }
  };

  return (
    <div className="progress-section">
      {/* Progress Bar */}
      <div className="progress-header">
        <span className="progress-label">
          {isDone ? t('progressLog.done') : t('progressLog.progress')}
        </span>
        <span className="progress-percent">
          {speed && <span style={{ marginRight: 8, fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 400 }}>{speed}</span>}
          {Math.round(progress)}%
        </span>
      </div>
      <div className="progress-bar-lg">
        <div
          className={`progress-fill-lg ${isDone ? (hasError ? 'has-error' : 'done') : ''}`}
          style={{ width: `${progress}%` }}
        />
      </div>
      <div className="progress-count">
        {current} / {total} {t('progressLog.files')}
      </div>

      {/* Log Panel */}
        <div className="log-panel" style={{ marginTop: 'var(--space-4)' }}>
          <div className="log-panel-header">
            <div className="log-panel-title">
              <ScrollText style={{ width: 14, height: 14 }} />
              {t('progressLog.logTitle')}
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
              {elapsed && (
                <span style={{ display: 'flex', alignItems: 'center', gap: 3, fontSize: 11, color: 'var(--color-text-tertiary)' }}>
                  <Timer style={{ width: 11, height: 11 }} />
                  {elapsed}
                </span>
              )}
              <span className="log-panel-count">{truncated > 0 ? `${displayLogs.length}/${logs.length}` : logs.length} {t('progressLog.entries')}</span>
              {onClearLogs && (
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={onClearLogs}
                  style={{ padding: '2px 6px' }}
                  title={t('progressLog.clearLogs')}
                >
                  <Trash2 style={{ width: 12, height: 12 }} />
                </button>
              )}
            </div>
          </div>
          <div className="log-content" ref={logContainerRef} onScroll={handleScroll}>
            {displayLogs.length === 0 ? (
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--color-text-tertiary)', fontSize: 12 }}>{t('progressLog.noLogs')}</div>
            ) : (<>
              {truncated > 0 && (
                <div style={{ padding: '4px 12px', fontSize: 10, color: 'var(--color-text-tertiary)', textAlign: 'center', borderBottom: '1px solid var(--color-border)', background: 'rgba(124,92,252,0.04)' }}>
                  ⋯ {truncated} {t('progressLog.entriesHidden')}
                </div>
              )}
              {displayLogs.map((log, i) => (
              log.status === 'download' && log.dlPercent != null ? (
                <div key={i} className={`log-entry ${i === displayLogs.length - 1 ? 'log-entry-new' : ''}`} style={{ flexDirection: 'column', alignItems: 'stretch', gap: 4 }}>
                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)' }}>
                    <span className="log-entry-time">{log.time}</span>
                    {statusIcon(log.status)}
                    <span className="log-entry-message info">{log.message}</span>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, paddingLeft: 90 }}>
                    <div style={{ flex: 1, height: 6, borderRadius: 3, background: 'var(--color-border)', overflow: 'hidden' }}>
                      <div style={{ height: '100%', borderRadius: 3, background: 'linear-gradient(90deg, #7c5cfc, #60a5fa)', width: `${log.dlPercent}%`, transition: 'width 0.3s ease' }} />
                    </div>
                    <span style={{ fontFamily: 'monospace', fontWeight: 700, color: '#60a5fa', fontSize: 10, minWidth: 36, textAlign: 'right', flexShrink: 0 }}>{log.dlPercent!.toFixed(1)}%</span>
                    {log.dlSpeed && <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', flexShrink: 0 }}>{log.dlSpeed}</span>}
                  </div>
                </div>
              ) : (
                <div key={i} className={`log-entry ${i === displayLogs.length - 1 ? 'log-entry-new' : ''}`}>
                  <span className="log-entry-time">{log.time}</span>
                  {statusIcon(log.status)}
                  <span className={`log-entry-message ${log.status}`}>{log.message}</span>
                </div>
              )
            ))}
            </>)}
            <div ref={logEndRef} />
          </div>
        </div>
    </div>
  );
}
