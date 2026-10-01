import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

const requests = new Map<string, { promise: Promise<any[]>; users: number }>();

export function useDynamicItems(command?: string) {
  const [state, setState] = useState<{ items: any[]; loading: boolean }>({ items: [], loading: !!command });
  useEffect(() => {
    if (!command) {
      setState({ items: [], loading: false });
      return;
    }
    let entry = requests.get(command);
    if (!entry) {
      entry = { promise: invoke<any[]>(command), users: 0 };
      requests.set(command, entry);
    }
    entry.users++;
    let active = true;
    setState({ items: [], loading: true });
    entry.promise.then(items => {
      if (active) setState({ items, loading: false });
    }, () => {
      if (active) setState({ items: [], loading: false });
    });
    return () => {
      active = false;
      entry.users--;
      queueMicrotask(() => {
        if (entry.users === 0 && requests.get(command) === entry) requests.delete(command);
      });
    };
  }, [command]);
  return state;
}
