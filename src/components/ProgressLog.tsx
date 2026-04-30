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

  // Auto-scroll to bottom when new logs appear
  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs.length]);

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
        <span className="progress-percent">{Math.round(progress)}%</span>
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
      {logs.length > 0 && (
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
          <div className="log-content">
            {logs.map((log, i) => (
              <div key={i} className={`log-entry ${i === logs.length - 1 ? 'log-entry-new' : ''}`}>
                <span className="log-entry-time">{log.time}</span>
                {statusIcon(log.status)}
                <span className={`log-entry-message ${log.status}`}>{log.message}</span>
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
        </div>
      )}
    </div>
  );
}
