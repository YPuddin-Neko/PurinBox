import { createContext, useContext, useState, useEffect, useCallback, ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import i18next from 'i18next';

export interface TaskInfo {
  id: string;
  name: string;
  status: 'running' | 'done' | 'error' | 'cancelled';
  progress: number; // 0-100
  current: number;
  total: number;
  message: string;
  startTime: number;
}

interface TaskContextType {
  tasks: TaskInfo[];
  addTask: (id: string, name: string) => void;
  removeTask: (id: string) => void;
  updateTask: (id: string, updates: Partial<TaskInfo>) => void;
  clearCompleted: () => void;
}

const TaskContext = createContext<TaskContextType>({
  tasks: [],
  addTask: () => {},
  removeTask: () => {},
  updateTask: () => {},
  clearCompleted: () => {},
});

export function useTaskQueue() {
  return useContext(TaskContext);
}

// 事件名 → 任务 ID 映射
const EVENT_TASK_MAP: Record<string, string> = {
  'scale-progress': 'scale',
  'flip-progress': 'flip',
  'filter-progress': 'filter',
  'keeper-progress': 'keeper',
  'convert-progress': 'convert',
  'alpha-progress': 'alpha',
  'rename-progress': 'rename',
  'crop-progress': 'crop',
  'person-crop-progress': 'person-crop',
  'tagger-progress': 'tagger',
  'python-env-progress': 'tagger',
  'llm-tagger-progress': 'llm-tagger',
  'tag-sort-progress': 'tag-sort',
  'perspective-progress': 'perspective',
  'blur-noise-progress': 'blur-noise',
  'upscale-progress': 'upscale',
  'cluster-progress': 'image-cluster',
};

export function TaskProvider({ children }: { children: ReactNode }) {
  const [tasks, setTasks] = useState<TaskInfo[]>([]);

  const addTask = useCallback((id: string, name: string) => {
    setTasks(prev => {
      const filtered = prev.filter(t => t.id !== id);
      return [...filtered, { id, name, status: 'running', progress: 0, current: 0, total: 0, message: i18next.t('common.preparing'), startTime: Date.now() }];
    });
  }, []);

  const removeTask = useCallback((id: string) => {
    setTasks(prev => prev.filter(t => t.id !== id));
  }, []);

  const updateTask = useCallback((id: string, updates: Partial<TaskInfo>) => {
    setTasks(prev => prev.map(t => t.id === id ? { ...t, ...updates } : t));
  }, []);

  const clearCompleted = useCallback(() => {
    setTasks(prev => prev.filter(t => t.status === 'running'));
  }, []);

  // 集中监听所有功能的进度事件
  useEffect(() => {
    let active = true;
    const unlisteners: Promise<() => void>[] = [];

    for (const [eventName, taskId] of Object.entries(EVENT_TASK_MAP)) {
      const unlistenPromise = listen<{
        current: number; total: number; filename: string; status: string; message: string;
      }>(eventName, (e) => {
        if (!active) return;
        const p = e.payload;
        setTasks(prev => {
          const task = prev.find(t => t.id === taskId);
          if (!task) return prev;
          const progress = p.total > 0 ? (p.current / p.total) * 100 : 0;
          let status: TaskInfo['status'] = task.status;
          if (p.status === 'done') status = 'done';
          else if (p.status === 'error' && (p.message.includes('已取消') || p.message.includes('cancelled'))) status = 'cancelled';
          return prev.map(t => t.id === taskId ? { ...t, progress, current: p.current, total: p.total, message: p.message, status } : t);
        });
      });
      unlisteners.push(unlistenPromise);
    }

    return () => {
      active = false;
      unlisteners.forEach(p => p.then(fn => fn()));
    };
  }, []);

  return (
    <TaskContext.Provider value={{ tasks, addTask, removeTask, updateTask, clearCompleted }}>
      {children}
    </TaskContext.Provider>
  );
}

