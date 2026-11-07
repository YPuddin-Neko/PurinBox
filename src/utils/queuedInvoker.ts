import type { TaskActions, TaskStatus } from '../components/TaskContext';
import type { QueuedCommand } from '../queuedCommands';
import { errorText } from './tauriRuntime';

type Progress = { status?: string; message?: string; current?: number; total?: number; percent?: number; cancelled?: boolean; run_id?: number };
type Invoke = <T>(command: string, args?: any, options?: any) => Promise<T>;
interface Dependencies {
  invoke: Invoke;
  listen: (event: string, handler: (event: { payload: Progress }) => void) => Promise<() => void>;
  actions: Pick<TaskActions, 'addTask' | 'updateTask'>;
  commands: Record<string, QueuedCommand>;
  name: (command: string) => string;
}
// Shared across page mounts so a second entry cannot replace an active task.
const seenRuns = new Map<string, number>();
const running = new Map<string, { cancelRequested: boolean }>();

export function resultHasFailures(result: unknown): boolean {
  if (!result || typeof result !== 'object') return false;
  const value = result as Record<string, unknown>;
  return ['failed', 'failed_files', 'fail_count', 'warning_count'].some(key => {
    const count = value[key];
    return Array.isArray(count) ? count.length > 0 : typeof count === 'number' && count > 0;
  });
}

export function createQueuedInvoker(deps: Dependencies): Invoke {
  return async <T>(command: string, args?: any, options?: any): Promise<T> => {
    for (const [id, state] of running) {
      if (deps.commands[id]?.cancel === command) state.cancelRequested = true;
    }
    const definition = deps.commands[command];
    if (!definition) return deps.invoke<T>(command, args, options);
    if (running.has(command)) throw new Error(`${deps.name(command)}：任务正在进行`);
    const state = { cancelRequested: false };
    running.set(command, state);
    deps.actions.addTask(command, deps.name(command));
    let active = true;
    let cancelled = false;
    let warning = false;
    let message = '';
    const unlisteners: (() => void)[] = [];
    try {
      for (const event of definition.events ?? []) {
        const floor = seenRuns.get(event) ?? 0;
        unlisteners.push(await deps.listen(event, ({ payload }) => {
          if (!active) return;
          if (typeof payload.run_id === 'number') {
            if (payload.run_id <= floor) return;
            seenRuns.set(event, Math.max(payload.run_id, seenRuns.get(event) ?? 0));
          }
          cancelled ||= payload.cancelled === true || payload.status === 'cancelled';
          warning ||= payload.status === 'error' || payload.status === 'warning';
          message = payload.message ?? message;
          const total = payload.percent !== undefined ? 100 : payload.total ?? 0;
          deps.actions.updateTask(command, {
            message,
            ...(total > 0 ? { current: payload.percent ?? payload.current ?? 0, total } : {}),
          });
        }));
      }
      if (state.cancelRequested) throw new Error('已取消');
      const result = await deps.invoke<T>(command, args, options);
      const status: TaskStatus = cancelled ? 'cancelled' : warning || resultHasFailures(result) ? 'warning' : 'done';
      deps.actions.updateTask(command, { status, message: typeof result === 'string' ? result : message });
      return result;
    } catch (error) {
      const text = errorText(error);
      deps.actions.updateTask(command, {
        status: cancelled || (state.cancelRequested && text.trimStart().startsWith('已取消')) ? 'cancelled' : 'error',
        message: text,
      });
      throw error;
    } finally {
      active = false;
      for (const unlisten of unlisteners) unlisten();
      running.delete(command);
    }
  };
}
