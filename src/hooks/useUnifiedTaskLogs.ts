import { Dispatch, SetStateAction, useCallback, useMemo, useRef } from 'react';
import i18n from '../i18n';
import { LogEntry, getTimeStr } from '../components/ProgressLog';

export interface UnifiedProgressPayload {
  current: number;
  total: number;
  filename?: string;
  status: string;
  message: string;
  i18n_key?: string;
  i18n_params?: Record<string, unknown>;
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

export function normalizeLogStatus(status: string): LogStatus {
  if (status === 'done' || status === 'processing') return 'info';
  if (status === 'success' || status === 'error' || status === 'download' || status === 'warning' || status === 'info') {
    return status;
  }
  return 'info';
}

export function isDuplicateBackendError(errorText: string, lastBackendError: string): boolean {
  return !!lastBackendError
    && (errorText.includes(lastBackendError) || lastBackendError.includes(errorText));
}

interface ProgressLogOptions {
  message?: string;
  status?: LogStatus;
}

interface DownloadLogOptions {
  appendDone?: boolean;
  doneStatus?: LogStatus | ((payload: UnifiedDownloadPayload) => LogStatus);
  errorPrefix?: string;
}

export function useUnifiedTaskLogs(setLogs: SetLogs) {
  const lastBackendErrorRef = useRef('');

  const markBackendError = useCallback((message: string) => {
    lastBackendErrorRef.current = message;
  }, []);

  const clearBackendError = useCallback(() => {
    lastBackendErrorRef.current = '';
  }, []);

  const appendLog = useCallback((message: string, status: LogStatus, extra?: Partial<LogEntry>) => {
    setLogs(prev => [...prev, { time: getTimeStr(), message, status, ...extra }]);
  }, [setLogs]);

  const setInitialLog = useCallback((message: string, status: LogStatus = 'info') => {
    clearBackendError();
    setLogs([{ time: getTimeStr(), message, status }]);
  }, [clearBackendError, setLogs]);

  const appendProgressLog = useCallback((payload: UnifiedProgressPayload, options: ProgressLogOptions = {}) => {
    const message = options.message ?? resolveProgressMessage(payload);
    if (payload.status === 'error') markBackendError(message);
    appendLog(message, options.status ?? normalizeLogStatus(payload.status));
  }, [appendLog, markBackendError]);

  const appendDownloadLog = useCallback((payload: UnifiedDownloadPayload, options: DownloadLogOptions = {}) => {
    const appendDone = options.appendDone ?? true;
    if (payload.status === 'done' || payload.status === 'cancelled') {
      setLogs(prev => {
        const next = prev.filter(log => log.status !== 'download');
        if (!appendDone) return next;
        const doneStatus = typeof options.doneStatus === 'function'
          ? options.doneStatus(payload)
          : options.doneStatus ?? (payload.status === 'done' ? 'success' : 'info');
        return [...next, { time: getTimeStr(), message: payload.message, status: doneStatus }];
      });
      return;
    }

    if (payload.status === 'error') {
      markBackendError(payload.message);
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
  }, [markBackendError, setLogs]);

  const appendCatchError = useCallback((error: unknown, prefix: string) => {
    const errorText = String(error);
    if (!isDuplicateBackendError(errorText, lastBackendErrorRef.current)) {
      appendLog(`${prefix}: ${errorText}`, 'error');
    }
    return errorText;
  }, [appendLog]);

  return useMemo(() => ({
    appendCatchError,
    appendDownloadLog,
    appendLog,
    appendProgressLog,
    clearBackendError,
    markBackendError,
    setInitialLog,
  }), [
    appendCatchError,
    appendDownloadLog,
    appendLog,
    appendProgressLog,
    clearBackendError,
    markBackendError,
    setInitialLog,
  ]);
}

export type UnifiedTaskLogger = ReturnType<typeof useUnifiedTaskLogs>;
