import { createContext, useContext, useState, useEffect, useCallback, useMemo, useRef, ReactNode } from 'react';
import { listen } from '../utils/tauriRuntime';
import i18next from 'i18next';
import { EVENT_TASK_MAP } from '../appRegistry';
import { resolveProgressMessage } from '../hooks/useUnifiedTaskLogs';

export type TaskStatus = 'running' | 'done' | 'error' | 'cancelled';

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

const CANCEL_PATTERN = /已取消|cancel/i;

/** 后端取消时的 done 事件消息和 Err 文本都带“已取消” */
export function isCancelMessage(text: string): boolean {
  return CANCEL_PATTERN.test(text);
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

interface TaskProgressPayload {
  current: number;
  total: number;
  status: string;
  message: string;
  i18n_key?: string;
  i18n_params?: Record<string, unknown>;
}

function statusFromEvent(payload: TaskProgressPayload, previous: TaskStatus): TaskStatus {
  switch (payload.status) {
    case 'done':
      return isCancelMessage(payload.message) ? 'cancelled' : 'done';
    case 'error':
      return isCancelMessage(payload.message) ? 'cancelled' : 'error';
    // 后续仍有进度事件说明任务还在跑（中途的单文件 error 不应让任务永远停在 error）
    case 'processing':
    case 'success':
      return 'running';
    default:
      return previous;
  }
}

export function TaskProvider({ children }: { children: ReactNode }) {
  const [tasks, setTasks] = useState<TaskInfo[]>([]);
  const activeTasks = useRef(new Set<string>());

  const addTask = useCallback((id: string, name: string) => {
    activeTasks.current.add(id);
    setTasks(prev => {
      const filtered = prev.filter(t => t.id !== id);
      return [...filtered, { id, name, status: 'running', current: 0, total: 0, message: i18next.t('common.preparing') }];
    });
  }, []);

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
      const unlistenPromise = listen<TaskProgressPayload>(eventName, (e) => {
        if (!active || !activeTasks.current.has(taskId)) return;
        const p = e.payload;
        // 终态后同名命令可能由工作流启动，不能重开已完成的页面任务。
        if (p.status === 'done') activeTasks.current.delete(taskId);
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
            status: statusFromEvent(p, task.status),
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
  }, []);

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
