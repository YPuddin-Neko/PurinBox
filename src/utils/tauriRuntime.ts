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

/** 命令被拒绝时的文本：Rust 命令拒绝时给的是字符串，其余取 Error 的 message，避免显示成 "Error: …" */
export function errorText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
}

/** 在系统默认浏览器中打开外部链接（Tauri 环境走 opener 插件，浏览器预览回退到 window.open） */
export async function openExternalUrl(url: string): Promise<void> {
  if (!url) return;
  if (hasTauriRuntime()) {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('plugin:opener|open_url', { url });
      return;
    } catch {
      // 回退到 window.open
    }
  }
  if (typeof window !== 'undefined') {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
}
