import { useEffect, useLayoutEffect, useRef } from 'react';
import i18n from '../i18n';
import { listen } from '../utils/tauriRuntime';
import type { UnifiedDownloadPayload, UnifiedProgressPayload, UnifiedTaskLogger } from './useUnifiedTaskLogs';

/** 写 Python 环境事件只用到这两个方法 */
export type PythonEnvLogger = Pick<UnifiedTaskLogger, 'appendProgressLog' | 'appendDownloadLog'>;

/**
 * 翻译后端发送的 @key|arg1|arg2 格式消息
 * 如果不是 @ 开头，原样返回
 */
function translateMessage(message: string): string {
  if (!message.startsWith('@')) return message;
  const parts = message.slice(1).split('|');
  const key = parts[0];
  const args = parts.slice(1);

  // 按 key 映射参数名
  switch (key) {
    case 'pythonEnv.installingDep':
      return i18n.t(key, { dep: args[0] || '', current: args[1] || '', total: args[2] || '' });
    case 'pythonEnv.venvFailed':
    case 'pythonEnv.gpuRuntimeFallback':
      return i18n.t(key, { error: args[0] || '' });
    case 'pythonEnv.downloading':
      return i18n.t(key, { filename: args[0] || '' });
    default:
      return i18n.t(key);
  }
}

/**
 * 统一监听 Python 环境事件的 Hook
 *
 * 在需要 Python 环境的功能页面中使用，监听：
 * - python-env-progress: 文本日志（如 "✓ Python 环境已就绪"）
 * - python-env-download: 内联进度条（pip 安装进度）
 *
 * 后端发送 @key|arg1|arg2 格式的消息，按当前语言翻译。
 *
 * 只在挂载时订阅一次：logger 每次渲染都可以是新对象（如 useTaskLog 的返回值），
 * 事件到达时取最新的那个，不会因为日志变化而退订重订、漏掉进度事件。
 *
 * @param active - 只有为 true 时才写日志（本页正在处理，防止别的页面的环境事件写进来）
 * @param logger - 本页的日志方法，例如 `usePythonEnvEvents(processing, taskLogs)`
 */
export function usePythonEnvEvents(active: boolean, logger: PythonEnvLogger) {
  const activeRef = useRef(false);
  const loggerRef = useRef(logger);
  useLayoutEffect(() => {
    activeRef.current = active;
    loggerRef.current = logger;
  });

  useEffect(() => {
    let mounted = true;

    // 文本日志（"✓ Python 环境已就绪" 等）
    const u1 = listen<UnifiedProgressPayload>('python-env-progress', (e) => {
      if (!mounted || !activeRef.current) return;
      loggerRef.current.appendProgressLog(e.payload, { message: translateMessage(e.payload.message) });
    });

    // 内联进度条（pip 安装依赖）
    const u2 = listen<UnifiedDownloadPayload>('python-env-download', (e) => {
      if (!mounted || !activeRef.current) return;
      const d = e.payload;
      loggerRef.current.appendDownloadLog({ ...d, message: translateMessage(d.message) }, { appendDone: false });
    });

    return () => {
      mounted = false;
      u1.then(fn => fn());
      u2.then(fn => fn());
    };
  }, []);
}
