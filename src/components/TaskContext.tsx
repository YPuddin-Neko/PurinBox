import { createContext, useContext, useState, useEffect, useCallback, useMemo, useRef, ReactNode } from 'react';
import { listen } from '../utils/tauriRuntime';
import i18next from 'i18next';
import { EVENT_TASK_MAP } from '../appRegistry';
import { RunIdGate, resolveProgressMessage, type UnifiedProgressPayload } from '../hooks/useUnifiedTaskLogs';

/** warning：跑完了但有文件失败 */
export type TaskStatus = 'running' | 'done' | 'warning' | 'error' | 'cancelled';

export interface TaskInfo {
  id: string;
  name: string;
  status: TaskStatus;
  current: number;
  total: number;
  message: string;
}

export type TaskPatch = Partial<Omit<TaskInfo, 'id'>>;

/** 对象：直接合并；函数：按当前任务算出补丁，返回 null 表示不改 */
export type TaskUpdate = TaskPatch | ((task: TaskInfo) => TaskPatch | null);

export interface TaskActions {
  addTask: (id: string, name: string) => void;
  updateTask: (id: string, update: TaskUpdate) => void;
  clearCompleted: () => void;
}

/**
 * 后端以 Err 结束被取消的运行时，文本以「已取消」开头。
 * 只在用户点过取消时据此判定，否则报错文本恰好以它开头也会被当成取消。
 */
export function isCancelMessage(text: string): boolean {
  return text.trimStart().startsWith('已取消');
}

/**
 * 进度事件对应的任务状态。终态只有 done：带 cancelled 为已取消，本轮出现过失败为警告；
 * 其余事件（含逐文件的 error、warning、skipped、info）都说明任务还在跑。
 */
export function taskStatusFromEvent(payload: Pick<UnifiedProgressPayload, 'status' | 'cancelled'>, hadError: boolean): TaskStatus {
  if (payload.status !== 'done') return 'running';
  if (payload.cancelled === true) return 'cancelled';
  return hadError ? 'warning' : 'done';
}

const noop = () => {};

const TaskActionsContext = createContext<TaskActions>({
  addTask: noop,
  updateTask: noop,
  clearCompleted: noop,
});

const TaskListContext = createContext<TaskInfo[]>([]);

/** 任务操作（引用稳定：任务列表变化不会让调用方重渲染） */
export function useTaskQueue(): TaskActions {
  return useContext(TaskActionsContext);
}

/** 任务列表（每个进度事件都会变化，只给需要展示列表的组件用） */
export function useTaskList(): TaskInfo[] {
  return useContext(TaskListContext);
}

/** 任务 ID → 它的进度事件名 */
const TASK_EVENTS: Record<string, string[]> = {};
for (const [eventName, taskId] of Object.entries(EVENT_TASK_MAP)) {
  (TASK_EVENTS[taskId] ??= []).push(eventName);
}

export function TaskProvider({ children }: { children: ReactNode }) {
  const [tasks, setTasks] = useState<TaskInfo[]>([]);
  const activeTasks = useRef(new Set<string>());
  /** 本轮收到过 error 事件的任务 */
  const erroredTasks = useRef(new Set<string>());
  const gates = useRef(new Map<string, RunIdGate>());

  const gateFor = useCallback((eventName: string) => {
    let gate = gates.current.get(eventName);
    if (!gate) {
      gate = new RunIdGate();
      gates.current.set(eventName, gate);
    }
    return gate;
  }, []);

  const addTask = useCallback((id: string, name: string) => {
    activeTasks.current.add(id);
    erroredTasks.current.delete(id);
    for (const eventName of TASK_EVENTS[id] ?? []) gateFor(eventName).begin();
    setTasks(prev => {
      const filtered = prev.filter(t => t.id !== id);
      return [...filtered, { id, name, status: 'running', current: 0, total: 0, message: i18next.t('common.preparing') }];
    });
  }, [gateFor]);

  const updateTask = useCallback((id: string, update: TaskUpdate) => {
    const trackStatus = (status?: TaskStatus) => {
      if (!status) return;
      if (status === 'running') activeTasks.current.add(id);
      else activeTasks.current.delete(id);
    };
    // 显式终态在下一次 React 提交前就停止接收事件。
    if (typeof update !== 'function') trackStatus(update.status);
    setTasks(prev => {
      const index = prev.findIndex(t => t.id === id);
      if (index < 0) return prev;
      const patch = typeof update === 'function' ? update(prev[index]) : update;
      if (!patch) return prev;
      if (typeof update === 'function') trackStatus(patch.status);
      const next = [...prev];
      next[index] = { ...prev[index], ...patch };
      return next;
    });
  }, []);

  const clearCompleted = useCallback(() => {
    setTasks(prev => prev.filter(t => {
      if (t.status === 'running') return true;
      activeTasks.current.delete(t.id);
      return false;
    }));
  }, []);

  // 集中监听所有功能的进度事件
  useEffect(() => {
    let active = true;
    const unlisteners: Promise<() => void>[] = [];

    for (const [eventName, taskId] of Object.entries(EVENT_TASK_MAP)) {
      const unlistenPromise = listen<UnifiedProgressPayload>(eventName, (e) => {
        if (!active) return;
        const p = e.payload;
        if (!gateFor(eventName).accept(p.run_id) || !activeTasks.current.has(taskId)) return;
        if (p.status === 'error') erroredTasks.current.add(taskId);
        const status = taskStatusFromEvent(p, erroredTasks.current.has(taskId));
        // 终态后同名命令可能由工作流启动，不能重开已结束的页面任务。
        if (status !== 'running') activeTasks.current.delete(taskId);
        const message = resolveProgressMessage(p);
        setTasks(prev => {
          const index = prev.findIndex(t => t.id === taskId);
          if (index < 0) return prev;
          const task = prev[index];
          const next = [...prev];
          next[index] = {
            ...task,
            // total 为 0 的日志类事件不清空计数
            ...(p.total > 0 ? { current: p.current, total: p.total } : {}),
            message,
            status,
          };
          return next;
        });
      });
      unlisteners.push(unlistenPromise);
    }

    return () => {
      active = false;
      unlisteners.forEach(p => p.then(fn => fn()));
    };
  }, [gateFor]);

  const actions = useMemo<TaskActions>(
    () => ({ addTask, updateTask, clearCompleted }),
    [addTask, updateTask, clearCompleted],
  );

  return (
    <TaskActionsContext.Provider value={actions}>
      <TaskListContext.Provider value={tasks}>
        {children}
      </TaskListContext.Provider>
    </TaskActionsContext.Provider>
  );
}
