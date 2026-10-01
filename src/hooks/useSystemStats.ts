import { useCallback, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppSettings } from '../components/ThemeProvider';
import { hasTauriRuntime } from '../utils/tauriRuntime';

export interface SystemStats {
  cpu_usage: number; cpu_name: string; cpu_cores: number;
  memory_used: number; memory_total: number; memory_percent: number;
  gpu_name: string; gpu_usage: number;
  vram_used: number; vram_total: number; vram_percent: number;
}

const subscribers = new Map<() => void, number>();
let snapshot: SystemStats | null = null;
let interval = 0;
let generation = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let polling = false;
const getSnapshot = () => snapshot;

function publish(value: SystemStats | null) {
  snapshot = value;
  subscribers.forEach((_, listener) => listener());
}

async function poll() {
  if (polling || interval <= 0) return;
  polling = true;
  const current = generation;
  try {
    const stats = await invoke<SystemStats>('get_system_stats');
    if (current === generation && interval > 0) publish(stats);
  } catch {
    // A failed sample must not stop subsequent monitoring.
  } finally {
    polling = false;
    if (interval > 0) timer = setTimeout(poll, interval);
  }
}

function reconcile() {
  const enabled = [...subscribers.values()].filter(value => value > 0);
  const next = hasTauriRuntime() && enabled.length ? Math.min(...enabled) : 0;
  if (next === interval) return;
  interval = next;
  generation += 1;
  clearTimeout(timer);
  if (interval > 0) void poll();
  else publish(null);
}

export default function useSystemStats() {
  const { monitorInterval } = useAppSettings();
  const subscribe = useCallback((listener: () => void) => {
    subscribers.set(listener, monitorInterval);
    reconcile();
    return () => {
      subscribers.delete(listener);
      reconcile();
    };
  }, [monitorInterval]);
  return useSyncExternalStore(subscribe, getSnapshot, () => null);
}

export function getUsageColor(percent: number) {
  return percent < 50 ? '#4ade80' : percent < 80 ? '#fbbf24' : '#f87171';
}
