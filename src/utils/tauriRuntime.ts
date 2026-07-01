type ListenEvent<T> = {
  event?: string;
  id?: number;
  payload: T;
};

export function hasTauriRuntime() {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export async function listen<T>(
  eventName: string,
  handler: (event: ListenEvent<T>) => void,
): Promise<() => void> {
  if (!hasTauriRuntime()) return () => {};

  try {
    const eventApi = await import('@tauri-apps/api/event');
    return await eventApi.listen<T>(eventName, handler as Parameters<typeof eventApi.listen<T>>[1]);
  } catch {
    return () => {};
  }
}
