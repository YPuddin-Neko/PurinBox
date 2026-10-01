import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import i18n from '../i18n';
import { listen } from '../utils/tauriRuntime';
import { useLogState, type LogEntry } from '../components/ProgressLog';
import { isCancelMessage, useTaskQueue, type TaskStatus } from '../components/TaskContext';
import { usePythonEnvEvents } from './usePythonEnvEvents';
import {
  useUnifiedTaskLogs,
  type LogStatus,
  type UnifiedProgressPayload,
  type UnifiedTaskLogger,
} from './useUnifiedTaskLogs';

/** 批处理命令的通用返回值（Rust commands::ProcessResult） */
export interface ProcessResult {
  success_count: number;
  fail_count: number;
  total: number;
  errors: string[];
}

export interface BatchTaskOptions {
  /** 后端进度事件名，如 'crop-progress' */
  event: string;
  /** 任务面板 ID（appRegistry 中登记的 task id）；不传则任何一轮都不进任务面板 */
  taskId?: string;
  /** processing 事件是否写日志；默认都不写 */
  logProcessing?: (payload: UnifiedProgressPayload) => boolean;
  /** done 事件是否写日志；默认 true。完成汇总由页面自己写时关掉 */
  logDone?: boolean;
  /** 覆盖单条事件的日志状态；返回 undefined 按事件 status 归一化 */
  logStatus?: (payload: UnifiedProgressPayload) => LogStatus | undefined;
  /** 每条被接收的事件在内置处理之后回调（计数、收集文件名等）；任务面板由 TaskContext 统一维护，不要在这里写 */
  onEvent?: (payload: UnifiedProgressPayload) => void;
  /** 运行期间把 python-env-progress / python-env-download 写进本页日志 */
  pythonEnv?: boolean;
}

export interface BatchRunRequest<T> {
  /** 任务面板显示名；hook 配了 taskId 且这里有值时，这一轮才进任务面板 */
  taskName?: string;
  /** 开始日志；不传则只清空日志 */
  startLog?: string;
  /** 执行动作，可以串多个 invoke；reject 视为失败（文本含“已取消”视为取消） */
  exec: () => Promise<T>;
  /** true：保留已有日志，开始日志追加在后面 */
  keepLogs?: boolean;
  /** false：这一轮不能用 ProcessButton 取消，buttonProps.processing 保持 false */
  cancellable?: boolean;
}

/** 直接展开给 <ProgressLog> */
export interface ProgressLogBindings {
  current: number;
  total: number;
  logs: LogEntry[];
  isDone: boolean;
  hasError: boolean;
  onClearLogs: () => void;
  externalStartTime: number;
}

/** 直接展开给 <ProcessButton>（onStart、cancelCommand 等由页面传） */
export interface ProcessButtonBindings {
  processing: boolean;
  onCancelLog: (message: string) => void;
}

export interface BatchTask {
  /** 有一轮正在执行（含 cancellable: false 的一轮） */
  processing: boolean;
  /** 页面自己的日志（预览、下载、删除、导出、汇总等） */
  logger: UnifiedTaskLogger;
  /** 已有一轮在执行时直接返回 undefined；exec 失败时错误已写入日志，同样返回 undefined */
  run: <T = ProcessResult>(request: BatchRunRequest<T>) => Promise<T | undefined>;
  progressLogProps: ProgressLogBindings;
  buttonProps: ProcessButtonBindings;
}

/** 命令正常返回后继续接收迟到事件的最长时间（收到 done 事件立即停止）；也是失败后复核任务面板状态的延迟 */
const LATE_EVENT_WINDOW_MS = 2000;

interface RunRecord {
  /** 进任务面板时的任务 ID */
  taskId: string | null;
  /** 本轮点过 ProcessButton 的取消 */
  cancelRequested: boolean;
  /** 本轮收到的 done 事件消息 */
  doneMessage: string | null;
  outcome: 'pending' | 'resolved' | 'rejected';
  /** outcome 为 rejected 时 catch 判定的任务状态 */
  failStatus: TaskStatus;
  /** 已停止接收事件 */
  closed: boolean;
}

function isProcessResultLike(value: unknown): value is Pick<ProcessResult, 'fail_count'> {
  return typeof value === 'object' && value !== null && 'fail_count' in value && typeof value.fail_count === 'number';
}

function errorText(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * 批处理页面的任务样板：运行状态、进度事件监听、开始/收尾、任务面板与两层日志。
 *
 * 事件只在本页发起的一轮里接收（命令返回后再留一个短窗口给迟到事件），
 * 工作流等其它来源运行同名命令时不会写进本页的进度和日志。
 */
export function useBatchTask(options: BatchTaskOptions): BatchTask {
  const { event, pythonEnv = false } = options;
  const { addTask, updateTask } = useTaskQueue();
  const [logs, setLogs] = useLogState();
  const logger = useUnifiedTaskLogs(setLogs);

  const [runState, setRunState] = useState<'cancellable' | 'locked' | null>(null);
  const [current, setCurrent] = useState(0);
  const [total, setTotal] = useState(0);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [startTime, setStartTime] = useState(0);

  const optionsRef = useRef(options);
  useLayoutEffect(() => {
    optionsRef.current = options;
  });

  const runRef = useRef<RunRecord | null>(null);
  const busyRef = useRef(false);
  const closeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current !== null) {
      clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);

  useEffect(() => () => clearCloseTimer(), [clearCloseTimer]);

  // 任务面板还停在运行中（done 事件丢失、被迟到事件改回运行中）时按本轮结果落定
  const settleTask = useCallback((record: RunRecord) => {
    if (record.taskId === null || record.outcome === 'pending') return;
    if (record.outcome === 'rejected') {
      const status = record.failStatus;
      updateTask(record.taskId, task => (task.status === 'running' ? { status } : null));
      return;
    }
    const status: TaskStatus = record.doneMessage !== null
      ? (isCancelMessage(record.doneMessage) ? 'cancelled' : 'done')
      : (record.cancelRequested ? 'cancelled' : 'done');
    // 命令已正常返回：此时的 error 只可能来自中途的单文件失败事件
    updateTask(record.taskId, task => (task.status === 'running' || task.status === 'error' ? { status } : null));
  }, [updateTask]);

  // 停止接收事件并落定任务面板
  const closeRun = useCallback((record: RunRecord) => {
    clearCloseTimer();
    record.closed = true;
    settleTask(record);
  }, [clearCloseTimer, settleTask]);

  useEffect(() => {
    let active = true;
    const unlisten = listen<UnifiedProgressPayload>(event, ({ payload }) => {
      const record = runRef.current;
      if (!active || !record || record.closed) return;
      const { logProcessing, logDone = true, logStatus, onEvent } = optionsRef.current;
      if (payload.total > 0) {
        setCurrent(payload.current);
        setTotal(payload.total);
      }
      if (payload.status === 'error') setHasError(true);
      const shouldLog = payload.status === 'processing'
        ? (logProcessing?.(payload) ?? false)
        : (payload.status !== 'done' || logDone);
      if (shouldLog) logger.appendProgressLog(payload, { status: logStatus?.(payload) });
      onEvent?.(payload);
      if (payload.status === 'done') {
        setIsDone(true);
        record.doneMessage = payload.message;
        if (record.outcome === 'resolved') closeRun(record);
      }
    });
    return () => {
      active = false;
      unlisten.then(fn => fn());
    };
  }, [event, logger, closeRun]);

  usePythonEnvEvents(pythonEnv && runState !== null, setLogs, logger);

  const run = useCallback(async <T>(request: BatchRunRequest<T>): Promise<T | undefined> => {
    if (busyRef.current) return undefined;
    busyRef.current = true;
    const previous = runRef.current;
    if (previous && closeTimerRef.current !== null) closeRun(previous);

    const { taskId } = optionsRef.current;
    const tracked = taskId !== undefined && request.taskName !== undefined
      ? { id: taskId, name: request.taskName }
      : null;
    const record: RunRecord = {
      taskId: tracked ? tracked.id : null,
      cancelRequested: false,
      doneMessage: null,
      outcome: 'pending',
      failStatus: 'error',
      closed: false,
    };
    runRef.current = record;

    setRunState(request.cancellable === false ? 'locked' : 'cancellable');
    setCurrent(0);
    setTotal(0);
    setIsDone(false);
    setHasError(false);
    setStartTime(Date.now());
    logger.setInitialLog(request.startLog, 'info', { keep: request.keepLogs });
    if (tracked) addTask(tracked.id, tracked.name);

    try {
      const result = await request.exec();
      record.outcome = 'resolved';
      if (isProcessResultLike(result) && result.fail_count > 0) setHasError(true);
      return result;
    } catch (error) {
      record.outcome = 'rejected';
      const text = errorText(error);
      if (isCancelMessage(text)) {
        record.failStatus = 'cancelled';
        // 后端取消时常先发一条“…已取消”的 done 事件再返回 Err，已写过就不再重复
        const seen = record.doneMessage;
        const logged = seen !== null && (optionsRef.current.logDone ?? true) && (seen.includes(text) || text.includes(seen));
        if (!logged) logger.appendLog(text, 'warning');
      } else {
        record.failStatus = 'error';
        logger.appendCatchError(text, i18n.t('pages.errorPrefix'));
        setHasError(true);
      }
      if (record.taskId !== null) updateTask(record.taskId, { status: record.failStatus, message: text });
      return undefined;
    } finally {
      busyRef.current = false;
      setRunState(null);
      setIsDone(true);
      if (record.outcome === 'resolved' && record.doneMessage !== null) {
        closeRun(record);
      } else {
        // 失败时立即停收：命令可能因工作流正占用同一命令被拒，之后到达的事件不属于本页；
        // 但任务面板仍要等迟到事件过去再复核一次
        if (record.outcome === 'rejected') record.closed = true;
        clearCloseTimer();
        closeTimerRef.current = setTimeout(() => closeRun(record), LATE_EVENT_WINDOW_MS);
      }
    }
  }, [addTask, updateTask, logger, closeRun, clearCloseTimer]);

  const onCancelLog = useCallback((message: string) => {
    const record = runRef.current;
    if (record && record.outcome === 'pending') record.cancelRequested = true;
    logger.appendLog(message, 'warning');
  }, [logger]);

  const clearLogs = useCallback(() => {
    logger.setInitialLog();
    setCurrent(0);
    setTotal(0);
    setIsDone(false);
    setHasError(false);
    setStartTime(0);
  }, [logger]);

  const progressLogProps = useMemo<ProgressLogBindings>(() => ({
    current,
    total,
    logs,
    isDone,
    hasError,
    onClearLogs: clearLogs,
    externalStartTime: startTime,
  }), [current, total, logs, isDone, hasError, clearLogs, startTime]);

  const buttonProps = useMemo<ProcessButtonBindings>(() => ({
    processing: runState === 'cancellable',
    onCancelLog,
  }), [runState, onCancelLog]);

  return {
    processing: runState !== null,
    logger,
    run,
    progressLogProps,
    buttonProps,
  };
}
