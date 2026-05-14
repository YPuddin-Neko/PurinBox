import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { LogEntry, getTimeStr } from '../components/ProgressLog';

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
 * @param setLogs - 日志状态 setter
 */
interface ProgressPayload {
  current: number;
  total: number;
  filename: string;
  status: string;
  message: string;
}

interface DownloadPayload {
  filename: string;
  downloaded: number;
  total: number;
  percent: number;
  speed_mbps: number;
  status: string;
  message: string;
}

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

export function usePythonEnvEvents(
  processing: boolean,
  setLogs: React.Dispatch<React.SetStateAction<LogEntry[]>>,
) {
  const { t } = useTranslation();
  const processingRef = useRef(false);
  useEffect(() => { processingRef.current = processing; }, [processing]);

  useEffect(() => {
    let active = true;

    // 文本日志（"✓ Python 环境已就绪" 等）
    const u1 = listen<ProgressPayload>('python-env-progress', (e) => {
      if (!active || !processingRef.current) return;
      const d = e.payload;
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: translateMessage(t, d.message),
        status: d.status as LogEntry['status'],
      }]);
    });

    // 内联进度条（pip 安装依赖）
    const u2 = listen<DownloadPayload>('python-env-download', (e) => {
      if (!active || !processingRef.current) return;
      const d = e.payload;
      const msg = translateMessage(t, d.message);
      if (d.status === 'done' || d.status === 'cancelled') {
        // 下载/安装结束，移除进度条
        setLogs(prev => prev.filter(l => l.status !== 'download'));
      } else if (d.status === 'error') {
        setLogs(prev => [...prev.filter(l => l.status !== 'download'), {
          time: getTimeStr(), message: msg, status: 'error',
        }]);
      } else {
        // 更新或插入内联进度条
        const avgSpeed = d.speed_mbps > 0 ? `${d.speed_mbps.toFixed(1)} MB/s` : '';
        setLogs(prev => {
          const idx = prev.findIndex(l => l.status === 'download');
          const entry: LogEntry = {
            time: getTimeStr(),
            message: msg,
            status: 'download',
            dlPercent: d.percent,
            dlSpeed: avgSpeed,
          };
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = entry;
            return next;
          }
          return [...prev, entry];
        });
      }
    });

    return () => {
      active = false;
      u1.then(fn => fn());
      u2.then(fn => fn());
    };
  }, [setLogs, t]);
}
