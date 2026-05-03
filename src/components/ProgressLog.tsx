import { useEffect, useRef } from 'react';
import { CheckCircle2, XCircle, Loader2, Info, ScrollText, Trash2 } from 'lucide-react';
import '../styles/progress.css';

export interface LogEntry {
  time: string;
  message: string;
  status: 'success' | 'error' | 'processing' | 'info';
}

interface ProgressLogProps {
  progress: number; // 0 - 100
  current: number;
  total: number;
  logs: LogEntry[];
  isDone: boolean;
  hasError: boolean;
  onClearLogs?: () => void;
}

function getTimeStr(): string {
  const now = new Date();
  return `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}`;
}

export { getTimeStr };

export default function ProgressLog({ progress, current, total, logs, isDone, hasError, onClearLogs }: ProgressLogProps) {
  const logEndRef = useRef<HTMLDivElement>(null);
  const startTimeRef = useRef<number | null>(null);

  // 记录开始时间
  useEffect(() => {
    if (current === 1 && !startTimeRef.current) {
      startTimeRef.current = Date.now();
    }
    if (current === 0) {
      startTimeRef.current = null;
    }
  }, [current]);

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
  }, [logs.length]);

  // 计算速度
  const getSpeed = () => {
    if (!startTimeRef.current || current <= 0) return '';
    const elapsed = (Date.now() - startTimeRef.current) / 1000;
    if (elapsed < 0.5) return '';
    const speed = current / elapsed;
    return speed >= 1 ? `${speed.toFixed(1)} it/s` : `${(1 / speed).toFixed(1)} s/it`;
  };

  const speed = getSpeed();

  const statusIcon = (status: LogEntry['status']) => {
    switch (status) {
      case 'success':
        return <CheckCircle2 className="log-entry-icon success" />;
      case 'error':
        return <XCircle className="log-entry-icon error" />;
      case 'processing':
        return <Loader2 className="log-entry-icon processing" />;
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
          {isDone ? '处理完成' : '处理进度'}
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
        {current} / {total} 个文件
      </div>

      {/* Log Panel */}
        <div className="log-panel" style={{ marginTop: 'var(--space-4)' }}>
          <div className="log-panel-header">
            <div className="log-panel-title">
              <ScrollText style={{ width: 14, height: 14 }} />
              处理日志
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
              <span className="log-panel-count">{logs.length} 条</span>
              {onClearLogs && (
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={onClearLogs}
                  style={{ padding: '2px 6px' }}
                  title="清空日志"
                >
                  <Trash2 style={{ width: 12, height: 12 }} />
                </button>
              )}
            </div>
          </div>
          <div className="log-content" ref={logContainerRef} onScroll={handleScroll}>
            {logs.length === 0 ? (
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--color-text-tertiary)', fontSize: 12 }}>暂无日志</div>
            ) : logs.map((log, i) => (
              <div key={i} className={`log-entry ${i === logs.length - 1 ? 'log-entry-new' : ''}`}>
                <span className="log-entry-time">{log.time}</span>
                {statusIcon(log.status)}
                <span className={`log-entry-message ${log.status}`}>{log.message}</span>
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
        </div>
    </div>
  );
}
