import { useCallback, useSyncExternalStore } from 'react';

const STORAGE_KEY = 'tagger_json_simplified';
const listeners = new Set<() => void>();
let current: boolean | null = null;

function read(): boolean {
  if (current === null) {
    try { current = localStorage.getItem(STORAGE_KEY) === 'true'; } catch { current = false; }
  }
  return current;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/**
 * Tagger 打标与 VLM 打标共用的「JSON 简化格式」选择。两个子页常驻挂载，
 * 一边改了另一边要立即跟着变，所以由这里统一保存并通知所有使用者。
 */
export function useTaggerJsonSimplified(): [boolean, (simplified: boolean) => void] {
  const simplified = useSyncExternalStore(subscribe, read);
  const setSimplified = useCallback((next: boolean) => {
    current = next;
    try { localStorage.setItem(STORAGE_KEY, String(next)); } catch { /* 配额满等，忽略 */ }
    listeners.forEach(listener => listener());
  }, []);
  return [simplified, setSimplified];
}
