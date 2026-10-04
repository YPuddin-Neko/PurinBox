import { useState, useEffect, useRef, useCallback, Dispatch, SetStateAction, ReactNode } from 'react';
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
  current: number;
  total: number;
  /** 应传 useLogState 的结果：条数上限在 state 层，这里按原样全部渲染 */
  logs: LogEntry[];
  isDone: boolean;
  hasError: boolean;
  onClearLogs?: () => void;
  /**
   * 耗时的起点（如点击开始的时间戳）；不传或为 0 时从第一个计数开始计时。
   * 速度总是从第一个计数开始算，不含环境部署、模型下载等准备时间。
   */
  externalStartTime?: number;
  /** 日志面板标题栏右侧、计时之后的附加内容（如成功/失败/警告计数） */
  headerExtra?: ReactNode;
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

/**
 * 带上限的日志 state：超过 MAX_LOGS 时丢弃最旧条目。
 * 上限放在 state 而不只在显示层：state 不封顶的话，长任务（几万张图）里数组无限增长，
 * 且每次追加的展开拷贝成本随长度线性上涨。所有日志页应使用此 hook 而非裸 useState。
 */
export function useLogState(): [LogEntry[], Dispatch<SetStateAction<LogEntry[]>>] {
  const [logs, setLogsRaw] = useState<LogEntry[]>([]);
  const setLogs = useCallback((action: SetStateAction<LogEntry[]>) => {
    setLogsRaw(prev => {
      const next = typeof action === 'function' ? action(prev) : action;
      return next.length > MAX_LOGS ? next.slice(next.length - MAX_LOGS) : next;
    });
  }, []);
  return [logs, setLogs];
}

/**
 * 速度的采样：本轮第一个计数和最近一次计数变化的时间。
 * 只用这两次之间完成的条数和时间：不含第一个条目之前的准备时间，任务结束后读数保持不变。
 */
export interface SpeedSample {
  /** 所属一轮的 externalStartTime，换轮时作废 */
  run: number;
  firstTime: number;
  firstCount: number;
  lastTime: number;
  lastCount: number;
}

export function nextSpeedSample(prev: SpeedSample | null, current: number, run: number, now: number): SpeedSample | null {
  if (current < 1) return null;
  if (!prev || prev.run !== run) return { run, firstTime: now, firstCount: current, lastTime: now, lastCount: current };
  if (current === prev.lastCount) return prev;
  return { ...prev, lastTime: now, lastCount: current };
}

/** 采样不足（还没有第二次计数，或间隔不到 0.5 秒）时返回空串 */
export function formatSpeed(sample: SpeedSample | null): string {
  if (!sample || sample.lastCount <= sample.firstCount) return '';
  const seconds = (sample.lastTime - sample.firstTime) / 1000;
  if (seconds < 0.5) return '';
  const speed = (sample.lastCount - sample.firstCount) / seconds;
  return speed >= 1 ? `${speed.toFixed(1)} it/s` : `${(1 / speed).toFixed(1)} s/it`;
}

export default function ProgressLog({ current, total, logs, isDone, hasError, onClearLogs, externalStartTime, headerExtra }: ProgressLogProps) {
  const { t } = useTranslation();
  const [sample, setSample] = useState<SpeedSample | null>(null);
  const [elapsed, setElapsed] = useState('');

  const progress = total > 0 ? Math.min(100, Math.max(0, (current / total) * 100)) : 0;
  const run = externalStartTime ?? 0;

  // 第一个计数到达时开始取样，计数归零或换轮时复位
  useEffect(() => {
    setSample(prev => nextSpeedSample(prev, current, run, Date.now()));
  }, [current, run]);

  const startTime = run > 0 ? run : (sample?.firstTime ?? 0);

  useEffect(() => {
    if (startTime === 0) {
      setElapsed('');
      return;
    }
    const update = () => setElapsed(formatElapsed(Date.now() - startTime));
    update();
    if (isDone) return;
    const timer = setInterval(update, 1000);
    return () => clearInterval(timer);
  }, [startTime, isDone]);

  // 停在底部附近时才自动滚到底，往上翻看日志时不打断
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

  const speed = formatSpeed(sample);

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
        return <AlertTriangle className="log-entry-icon warning" />;
      case 'info':
      default:
        return <Info className="log-entry-icon info" />;
    }
  };

  return (
    <div className="progress-section">
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
            {headerExtra}
            <span className="log-panel-count">{logs.length} {t('progressLog.entries')}</span>
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
          {logs.length === 0 ? (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--color-text-tertiary)', fontSize: 12 }}>{t('progressLog.noLogs')}</div>
          ) : logs.map((log, i) => (
            log.status === 'download' && log.dlPercent != null ? (
              <div key={i} className={`log-entry ${i === logs.length - 1 ? 'log-entry-new' : ''}`} style={{ flexDirection: 'column', alignItems: 'stretch', gap: 4 }}>
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
              <div key={i} className={`log-entry ${i === logs.length - 1 ? 'log-entry-new' : ''}`}>
                <span className="log-entry-time">{log.time}</span>
                {statusIcon(log.status)}
                <span className={`log-entry-message ${log.status}`}>{log.message}</span>
              </div>
            )
          ))}
        </div>
      </div>
    </div>
  );
}
