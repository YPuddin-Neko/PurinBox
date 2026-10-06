import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import i18n from '../i18n';
import { errorText, listen } from '../utils/tauriRuntime';
import type { ProcessResult } from '../api/commandOptions';
import { useLogState, type LogEntry } from '../components/ProgressLog';
import { isCancelMessage, useTaskQueue, type TaskStatus } from '../components/TaskContext';
import { usePythonEnvEvents } from './usePythonEnvEvents';
import {
  RunIdGate,
  isCancelledDone,
  textsOverlap,
  useUnifiedTaskLogs,
  type UnifiedDownloadPayload,
  type UnifiedProgressPayload,
  type UnifiedTaskLogger,
} from './useUnifiedTaskLogs';

export type { ProcessResult };

/** 模型、引擎等下载事件的写法 */
export interface BatchDownloadOptions {
  /** 下载事件名，如 'aesthetic-download'、'tagger-download' */
  event: string;
  /** done 时追加完成消息，默认 true；后端在进度通道也报了完成时（美学、打标）传 false，免得同一句记两遍 */
  appendDone?: boolean;
  /** error 消息的前缀，如 t('aiTagger.downloadFail') */
  errorPrefix?: string;
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
  /** 每条被接收的事件在内置处理之后回调（计数、收集文件名等）；任务面板由 TaskContext 统一维护，不要在这里写 */
  onEvent?: (payload: UnifiedProgressPayload) => void;
  /** 运行期间把 python-env-progress / python-env-download 写进本页日志 */
  pythonEnv?: boolean;
  /**
   * 运行期间（以及 trackDownload 包住的下载期间）把下载事件写进本页日志：
   * 进度条原地更新；完成或取消时移除进度条，各只记一条日志。
   * 运行中收到 cancelled 即按用户取消收尾。
   */
  download?: BatchDownloadOptions;
}

/** 交给 exec 的本轮状态 */
export interface BatchRunContext {
  /**
   * 本轮点过取消。exec 串了多条命令时，在下一条之前查它：后一条命令开头若会复位取消标志，
   * 两条之间点的取消就会被抹掉。查到为 true 时直接返回（不必再调命令），这一轮按已取消收尾
   */
  cancelRequested: () => boolean;
}

export interface BatchRunRequest<T> {
  /** 任务面板显示名；hook 配了 taskId 且这里有值时，这一轮才进任务面板 */
  taskName?: string;
  /** 开始日志；不传则只清空日志 */
  startLog?: string;
  /**
   * 执行动作，可以串多个 invoke。reject 视为失败；后端已报告取消（done 带 cancelled、下载事件 cancelled），
   * 或用户点过取消且文本以「已取消」开头时视为取消
   */
  exec: (run: BatchRunContext) => Promise<T>;
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
  /**
   * 在 run 之外发起下载（如单独的「下载模型」按钮）时用它包住 invoke，
   * 期间 download.event 的事件照样写进本页日志；run 里的下载不用包。失败照常 reject，由页面处理。
   */
  trackDownload: <T>(exec: () => Promise<T>) => Promise<T>;
  progressLogProps: ProgressLogBindings;
  buttonProps: ProcessButtonBindings;
}

/** 命令正常返回后继续接收迟到事件的最长时间（收到 done 事件立即停止） */
const LATE_EVENT_WINDOW_MS = 2000;

/** 一轮的结果中决定任务面板终态的部分 */
export interface RunOutcome {
  /** 后端确认本轮被取消 */
  cancelled: boolean;
  /** 本轮点过 ProcessButton 的取消 */
  cancelRequested: boolean;
  /** 收到过本轮的 done 事件 */
  doneSeen: boolean;
  /** 本轮有文件错误或警告 */
  failed: boolean;
}

interface RunRecord extends RunOutcome {
  /** 进任务面板时的任务 ID */
  taskId: string | null;
  /** 已写进日志的取消消息；Err 文本与它重叠时不再重复写 */
  cancelNotice: string | null;
  outcome: 'pending' | 'resolved' | 'rejected';
  /** 已停止接收事件 */
  closed: boolean;
}

/** 命令正常返回（没有 reject）的一轮在任务面板上的终态 */
export function settledTaskStatus(run: RunOutcome): TaskStatus {
  // 取消后命令仍可能正常返回且不发 done，此时按用户的取消收尾
  if (run.cancelled || (run.cancelRequested && !run.doneSeen)) return 'cancelled';
  return run.failed ? 'warning' : 'done';
}

export function processResultHasIssues(value: unknown): boolean {
  if (typeof value !== 'object' || value === null) return false;
  const result = value as Partial<ProcessResult>;
  return (typeof result.fail_count === 'number' && result.fail_count > 0)
    || (typeof result.warning_count === 'number' && result.warning_count > 0);
}

/**
 * 批处理页面的任务样板：运行状态、进度事件监听、开始/收尾、任务面板与两层日志。
 *
 * 事件只在本页发起的一轮里接收（命令返回后再留一个短窗口给迟到事件），
 * 工作流等其它来源运行同名命令时不会写进本页的进度和日志；
 * 上一轮晚到的事件按 run_id 丢弃。
 */
export function useBatchTask(options: BatchTaskOptions): BatchTask {
  const { event, pythonEnv = false } = options;
  const downloadEvent = options.download?.event;
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
  /** trackDownload 包住的下载数 */
  const downloadsRef = useRef(0);
  const closeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const gateRef = useRef(new RunIdGate());

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current !== null) {
      clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);

  useEffect(() => () => clearCloseTimer(), [clearCloseTimer]);

  // 返回值里的 fail_count 只有这里知道，所以正常返回的一轮由这里落定终态
  const settleTask = useCallback((record: RunRecord) => {
    if (record.taskId === null || record.outcome !== 'resolved') return;
    updateTask(record.taskId, { status: settledTaskStatus(record) });
  }, [updateTask]);

  // 停止接收事件并落定任务面板
  const closeRun = useCallback((record: RunRecord) => {
    if (record.closed) return;
    clearCloseTimer();
    record.closed = true;
    settleTask(record);
  }, [clearCloseTimer, settleTask]);

  useEffect(() => {
    let active = true;
    const gate = gateRef.current;
    const unlisten = listen<UnifiedProgressPayload>(event, ({ payload }) => {
      // 先过 run_id：不归本页处理的轮次也要记下，下一轮才能把它们当旧轮丢弃
      if (!active || !gate.accept(payload.run_id)) return;
      const record = runRef.current;
      if (!record || record.closed) return;
      const { logProcessing, logDone = true, onEvent } = optionsRef.current;
      if (payload.total > 0) {
        setCurrent(payload.current);
        setTotal(payload.total);
      }
      if (payload.status === 'error' || (payload.status === 'warning' && !!payload.filename)) {
        record.failed = true;
        setHasError(true);
      }
      const done = payload.status === 'done';
      const shouldLog = payload.status === 'processing'
        ? (logProcessing?.(payload) ?? false)
        : (!done || logDone);
      if (shouldLog) logger.appendProgressLog(payload);
      onEvent?.(payload);
      if (done) {
        setIsDone(true);
        record.doneSeen = true;
        if (isCancelledDone(payload) || (record.cancelRequested && isCancelMessage(payload.message))) {
          record.cancelled = true;
          if (shouldLog) record.cancelNotice = payload.message;
        }
        if (record.outcome === 'resolved') closeRun(record);
      }
    });
    return () => {
      active = false;
      unlisten.then(fn => fn());
    };
  }, [event, logger, closeRun]);

  useEffect(() => {
    if (!downloadEvent) return;
    let active = true;
    const unlisten = listen<UnifiedDownloadPayload>(downloadEvent, ({ payload }) => {
      if (!active) return;
      const record = busyRef.current ? runRef.current : null;
      if (!record && downloadsRef.current === 0) return;
      const download = optionsRef.current.download;
      if (payload.status !== 'cancelled') {
        logger.appendDownloadLog(payload, { appendDone: download?.appendDone ?? true, errorPrefix: download?.errorPrefix });
        return;
      }
      logger.appendDownloadLog(payload, { appendDone: false });
      if (record) record.cancelled = true;
      if (!textsOverlap(record?.cancelNotice, payload.message)) {
        logger.appendLog(payload.message, 'warning');
        if (record) record.cancelNotice = payload.message;
      }
    });
    return () => {
      active = false;
      unlisten.then(fn => fn());
    };
  }, [downloadEvent, logger]);

  usePythonEnvEvents(pythonEnv && runState !== null, logger);

  const run = useCallback(async <T>(request: BatchRunRequest<T>): Promise<T | undefined> => {
    if (busyRef.current) return undefined;
    busyRef.current = true;
    const previous = runRef.current;
    if (previous && closeTimerRef.current !== null) closeRun(previous);

    const { taskId } = optionsRef.current;
    gateRef.current.begin();
    const tracked = taskId !== undefined && request.taskName !== undefined
      ? { id: taskId, name: request.taskName }
      : null;
    const record: RunRecord = {
      taskId: tracked ? tracked.id : null,
      cancelRequested: false,
      cancelled: false,
      cancelNotice: null,
      doneSeen: false,
      failed: false,
      outcome: 'pending',
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
      const result = await request.exec({ cancelRequested: () => record.cancelRequested });
      record.outcome = 'resolved';
      if (processResultHasIssues(result)) {
        record.failed = true;
        setHasError(true);
      }
      return result;
    } catch (error) {
      record.outcome = 'rejected';
      const text = errorText(error);
      const cancelled = record.cancelled || (record.cancelRequested && isCancelMessage(text));
      if (cancelled) {
        record.cancelled = true;
        if (!textsOverlap(record.cancelNotice, text)) logger.appendLog(text, 'warning');
      } else {
        logger.appendCatchError(text, i18n.t('pages.errorPrefix'));
        setHasError(true);
      }
      if (record.taskId !== null) updateTask(record.taskId, { status: cancelled ? 'cancelled' : 'error', message: text });
      return undefined;
    } finally {
      busyRef.current = false;
      setRunState(null);
      setIsDone(true);
      if (record.outcome === 'rejected') {
        // 失败时立即停收：命令可能因工作流正占用同一命令被拒，之后到达的事件不属于本页
        record.closed = true;
      } else if (record.doneSeen) {
        closeRun(record);
      } else {
        clearCloseTimer();
        closeTimerRef.current = setTimeout(() => closeRun(record), LATE_EVENT_WINDOW_MS);
      }
    }
  }, [addTask, updateTask, logger, closeRun, clearCloseTimer]);

  const trackDownload = useCallback(async <T>(exec: () => Promise<T>): Promise<T> => {
    downloadsRef.current += 1;
    try {
      return await exec();
    } finally {
      downloadsRef.current -= 1;
    }
  }, []);

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
    trackDownload,
    progressLogProps,
    buttonProps,
  };
}
