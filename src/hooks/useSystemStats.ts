import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { hasTauriRuntime } from '../utils/tauriRuntime';

export interface SystemStats {
  cpu_usage: number; cpu_name: string; cpu_cores: number;
  memory_used: number; memory_total: number; memory_percent: number;
  gpu_name: string; gpu_usage: number;
  vram_used: number; vram_total: number; vram_percent: number;
}

// 全应用只有一处轮询：顶栏和设置页的监控面板订阅同一份数据，间隔由设置写入
const listeners = new Set<() => void>();
let snapshot: SystemStats | null = null;
let interval = 0;
let generation = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let polling = false;
const active = () => interval > 0 && listeners.size > 0;

export const getSystemStats = () => snapshot;

function publish(value: SystemStats | null) {
  snapshot = value;
  listeners.forEach(listener => listener());
}

async function poll() {
  if (polling || !active()) return;
  polling = true;
  const current = generation;
  try {
    const stats = await invoke<SystemStats>('get_system_stats');
    if (current === generation) publish(stats);
  } catch {
    // 单次采样失败不影响后续监控
  } finally {
    polling = false;
    if (active()) timer = setTimeout(poll, interval);
  }
}

/** 立即采样一次再按当前间隔继续；有采样在途时由它收尾后接上 */
function restart() {
  clearTimeout(timer);
  void poll();
}

/** 设置里的检测间隔（毫秒）；0 为关闭，已显示的数据随之清空 */
export function setSystemStatsInterval(ms: number) {
  const next = hasTauriRuntime() && ms > 0 ? ms : 0;
  if (next === interval) return;
  interval = next;
  if (next > 0) {
    restart();
  } else {
    generation += 1;
    clearTimeout(timer);
    publish(null);
  }
}

// 没有订阅者时只停轮询、保留上一份数据，重新订阅时不会先闪一下空白
export function subscribeSystemStats(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) restart();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) clearTimeout(timer);
  };
}

export default function useSystemStats() {
  return useSyncExternalStore(subscribeSystemStats, getSystemStats, () => null);
}

export function getUsageColor(percent: number) {
  return percent < 50 ? '#4ade80' : percent < 80 ? '#fbbf24' : '#f87171';
}
