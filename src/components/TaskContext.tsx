import { createContext, useContext, useState, useEffect, useCallback, ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';

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
}

const TaskContext = createContext<TaskContextType>({
  tasks: [],
  addTask: () => {},
  removeTask: () => {},
  updateTask: () => {},
});

export function useTaskQueue() {
  return useContext(TaskContext);
}

export function TaskProvider({ children }: { children: ReactNode }) {
  const [tasks, setTasks] = useState<TaskInfo[]>([]);

  const addTask = useCallback((id: string, name: string) => {
    setTasks(prev => {
      // 替换同 ID 的旧任务
      const filtered = prev.filter(t => t.id !== id);
      return [...filtered, { id, name, status: 'running', progress: 0, current: 0, total: 0, message: '准备中...', startTime: Date.now() }];
    });
  }, []);

  const removeTask = useCallback((id: string) => {
    setTasks(prev => prev.filter(t => t.id !== id));
  }, []);

  const updateTask = useCallback((id: string, updates: Partial<TaskInfo>) => {
    setTasks(prev => prev.map(t => t.id === id ? { ...t, ...updates } : t));
  }, []);

  // 监听 tagger-progress 事件，自动更新对应任务
  useEffect(() => {
    let active = true;
    const unlistenPromise = listen<{
      current: number; total: number; filename: string; status: string; message: string;
    }>('tagger-progress', (e) => {
      if (!active) return;
      const p = e.payload;
      setTasks(prev => {
        const task = prev.find(t => t.id === 'tagger');
        if (!task) return prev;
        const progress = p.total > 0 ? (p.current / p.total) * 100 : 0;
        const status: TaskInfo['status'] = p.status === 'done' ? 'done' : p.status === 'error' && p.message.includes('已取消') ? 'cancelled' : task.status;
        return prev.map(t => t.id === 'tagger' ? { ...t, progress, current: p.current, total: p.total, message: p.message, status } : t);
      });
    });
    return () => { active = false; unlistenPromise.then(fn => fn()); };
  }, []);

  return (
    <TaskContext.Provider value={{ tasks, addTask, removeTask, updateTask }}>
      {children}
    </TaskContext.Provider>
  );
}
