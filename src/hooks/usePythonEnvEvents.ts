import { useEffect, useLayoutEffect, useRef } from 'react';
import { listen } from '../utils/tauriRuntime';
import { useTranslation } from 'react-i18next';
import {
  SetLogs,
  UnifiedDownloadPayload,
  UnifiedProgressPayload,
  UnifiedTaskLogger,
  useUnifiedTaskLogs,
} from './useUnifiedTaskLogs';

/**
 * 翻译后端发送的 @key|arg1|arg2 格式消息
 * 如果不是 @ 开头，原样返回
 */
function translateMessage(t: (key: string, opts?: Record<string, unknown>) => string, message: string): string {
  if (!message.startsWith('@')) return message;
  const parts = message.slice(1).split('|');
  const key = parts[0];
  const args = parts.slice(1);

  // 按 key 映射参数名
  switch (key) {
    case 'pythonEnv.installingDep':
      return t(key, { dep: args[0] || '', current: args[1] || '', total: args[2] || '' });
    case 'pythonEnv.venvFailed':
      return t(key, { error: args[0] || '' });
    case 'pythonEnv.downloading':
      return t(key, { filename: args[0] || '' });
    default:
      return t(key);
  }
}

/**
 * 统一监听 Python 环境事件的 Hook
 *
 * 在需要 Python 环境的功能页面中使用，监听：
 * - python-env-progress: 文本日志（如 "✓ Python 环境已就绪"）
 * - python-env-download: 内联进度条（pip 安装进度）
 *
 * 后端发送 @key|arg1|arg2 格式的消息，前端自动翻译。
 *
 * @param processing - 当前功能是否正在处理中，只有处理中才接收事件（防止跨页面泄漏）
 * @param setLogs - 日志状态 setter；未传 logger 时用它创建内部 logger
 * @param logger - 页面自己的 logger；传入后事件写入这个 logger
 */
export function usePythonEnvEvents(
  processing: boolean,
  setLogs: SetLogs,
  logger?: UnifiedTaskLogger,
) {
  const { t } = useTranslation();
  const fallbackLogger = useUnifiedTaskLogs(setLogs);
  const activeLogger = logger ?? fallbackLogger;
  const processingRef = useRef(false);
  useLayoutEffect(() => { processingRef.current = processing; }, [processing]);

  useEffect(() => {
    let active = true;

    // 文本日志（"✓ Python 环境已就绪" 等）
    const u1 = listen<UnifiedProgressPayload>('python-env-progress', (e) => {
      if (!active || !processingRef.current) return;
      activeLogger.appendProgressLog(e.payload, { message: translateMessage(t, e.payload.message) });
    });

    // 内联进度条（pip 安装依赖）
    const u2 = listen<UnifiedDownloadPayload>('python-env-download', (e) => {
      if (!active || !processingRef.current) return;
      const d = e.payload;
      activeLogger.appendDownloadLog({ ...d, message: translateMessage(t, d.message) }, { appendDone: false });
    });

    return () => {
      active = false;
      u1.then(fn => fn());
      u2.then(fn => fn());
    };
  }, [activeLogger, t]);
}
