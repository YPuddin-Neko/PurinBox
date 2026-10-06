import { Dispatch, SetStateAction, useCallback, useMemo, useRef } from 'react';
import i18n from '../i18n';
import { LogEntry, getTimeStr, useLogState } from '../components/ProgressLog';

export interface UnifiedProgressPayload {
  current: number;
  total: number;
  filename?: string;
  status: string;
  message: string;
  i18n_key?: string;
  i18n_params?: Record<string, unknown>;
  /** 后端 begin_run 分配的运行 ID，同一事件名下递增；不经 begin_run 的事件没有 */
  run_id?: number;
  /** 被用户取消的那一轮，终态 done 带 true */
  cancelled?: boolean;
  /** 已处理但没有写入标签的警告 */
  unwritten?: boolean;
}

/**
 * 按事件通道丢弃上一轮迟到的进度事件。一个事件名用一个实例。
 *
 * 通道上的每条事件都要先过 accept（包括不归本页处理的轮次，如工作流跑同一命令），
 * begin 记下的下限才能覆盖别处跑过的轮次。
 */
export class RunIdGate {
  private seen = 0;
  private floor = 0;

  /** 新一轮开始：此前见过的 run_id 都算旧轮 */
  begin(): void {
    this.floor = this.seen;
  }

  /** 是否接收这条事件；没有 run_id 的事件照常接收 */
  accept(runId: number | undefined): boolean {
    if (typeof runId !== 'number' || !Number.isFinite(runId)) return true;
    if (runId > this.seen) this.seen = runId;
    return runId > this.floor;
  }
}

/** 被用户取消的那一轮的终态事件 */
export function isCancelledDone(payload: Pick<UnifiedProgressPayload, 'status' | 'cancelled'>): boolean {
  return payload.status === 'done' && payload.cancelled === true;
}

export interface UnifiedDownloadPayload {
  filename?: string;
  downloaded?: number;
  total?: number;
  percent: number;
  speed_mbps: number;
  status: string;
  message: string;
}

export type LogStatus = LogEntry['status'];
export type SetLogs = Dispatch<SetStateAction<LogEntry[]>>;

export function resolveProgressMessage(payload: UnifiedProgressPayload): string {
  if (!payload.i18n_key) return payload.message;
  const translated = i18n.t(payload.i18n_key, payload.i18n_params || {});
  return translated !== payload.i18n_key ? translated : payload.message;
}

function normalizeLogStatus(status: string): LogStatus {
  if (status === 'success' || status === 'error' || status === 'download' || status === 'warning' || status === 'info') {
    return status;
  }
  return 'info';
}

/** 两段文本互相包含：后端事件里的消息与命令返回的 Err 文本常是同一句的长短版本 */
export function textsOverlap(logged: string | null | undefined, text: string): boolean {
  return !!logged && (logged.includes(text) || text.includes(logged));
}

interface ProgressLogOptions {
  /** 覆盖显示文本（默认按 i18n_key 翻译，失败时用 message） */
  message?: string;
  /** 覆盖日志状态（默认由事件 status 归一化） */
  status?: LogStatus;
}

interface DownloadLogOptions {
  appendDone?: boolean;
  errorPrefix?: string;
}

interface InitialLogOptions {
  /** true 时保留已有日志，开始日志追加在后面 */
  keep?: boolean;
}

export function useUnifiedTaskLogs(setLogs: SetLogs) {
  // invoke 可能返回原文，而事件已按当前语言翻译；两者都用于同一次错误去重。
  const lastBackendErrorRef = useRef<string[]>([]);

  const appendLog = useCallback((message: string, status: LogStatus, extra?: Partial<LogEntry>) => {
    setLogs(prev => [...prev, { time: getTimeStr(), message, status, ...extra }]);
  }, [setLogs]);

  /**
   * 一轮任务开始：清掉上一轮的错误去重记录，并按 keep 清空或保留已有日志；
   * 不传 message 时只做前两步。
   */
  const setInitialLog = useCallback((message?: string, status: LogStatus = 'info', options: InitialLogOptions = {}) => {
    lastBackendErrorRef.current = [];
    const entries: LogEntry[] = message === undefined ? [] : [{ time: getTimeStr(), message, status }];
    if (options.keep) {
      if (entries.length > 0) setLogs(prev => [...prev, ...entries]);
    } else {
      setLogs(entries);
    }
  }, [setLogs]);

  const appendProgressLog = useCallback((payload: UnifiedProgressPayload, options: ProgressLogOptions = {}) => {
    const message = options.message ?? resolveProgressMessage(payload);
    if (payload.status === 'error') lastBackendErrorRef.current = [payload.message, message];
    appendLog(message, options.status ?? normalizeLogStatus(payload.status));
  }, [appendLog]);

  const appendDownloadLog = useCallback((payload: UnifiedDownloadPayload, options: DownloadLogOptions = {}) => {
    const appendDone = options.appendDone ?? true;
    if (payload.status === 'done' || payload.status === 'cancelled') {
      setLogs(prev => {
        const next = prev.filter(log => log.status !== 'download');
        if (!appendDone) return next;
        return [...next, { time: getTimeStr(), message: payload.message, status: payload.status === 'done' ? 'success' : 'info' }];
      });
      return;
    }

    if (payload.status === 'error') {
      lastBackendErrorRef.current = [payload.message];
      const message = options.errorPrefix ? `${options.errorPrefix}: ${payload.message}` : payload.message;
      setLogs(prev => [...prev.filter(log => log.status !== 'download'), {
        time: getTimeStr(),
        message,
        status: 'error',
      }]);
      return;
    }

    const avgSpeed = payload.speed_mbps > 0 ? `${payload.speed_mbps.toFixed(1)} MB/s` : '';
    setLogs(prev => {
      const idx = prev.findIndex(log => log.status === 'download');
      const entry: LogEntry = {
        time: getTimeStr(),
        message: payload.message,
        status: 'download',
        dlPercent: payload.percent,
        dlSpeed: avgSpeed,
      };
      if (idx >= 0) {
        const next = [...prev];
        next[idx] = entry;
        return next;
      }
      return [...prev, entry];
    });
  }, [setLogs]);

  const appendCatchError = useCallback((error: unknown, prefix: string) => {
    const errorText = String(error);
    if (!lastBackendErrorRef.current.some(message => textsOverlap(message, errorText))) {
      appendLog(`${prefix}: ${errorText}`, 'error');
    }
    return errorText;
  }, [appendLog]);

  return useMemo(() => ({
    appendCatchError,
    appendDownloadLog,
    appendLog,
    appendProgressLog,
    setInitialLog,
  }), [
    appendCatchError,
    appendDownloadLog,
    appendLog,
    appendProgressLog,
    setInitialLog,
  ]);
}

export type UnifiedTaskLogger = ReturnType<typeof useUnifiedTaskLogs>;

export type TaskLog = UnifiedTaskLogger & {
  logs: LogEntry[];
  setLogs: SetLogs;
};

/**
 * 日志 state（useLogState，带 500 条上限）与追加语义（useUnifiedTaskLogs）的组合。
 * 返回对象随 logs 变化；要放进 effect 依赖时取其中的方法（各方法引用稳定）。
 */
export function useTaskLog(): TaskLog {
  const [logs, setLogs] = useLogState();
  const logger = useUnifiedTaskLogs(setLogs);
  return useMemo(() => ({ ...logger, logs, setLogs }), [logger, logs, setLogs]);
}
