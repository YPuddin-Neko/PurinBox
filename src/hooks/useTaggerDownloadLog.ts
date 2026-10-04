import { useEffect, useLayoutEffect, useRef } from 'react';
import i18n from '../i18n';
import { listen } from '../utils/tauriRuntime';
import type { BatchDownloadOptions } from './useBatchTask';
import type { UnifiedDownloadPayload, UnifiedTaskLogger } from './useUnifiedTaskLogs';

/** 打标模型下载写进日志的方式；下载完成由 tagger-progress 报告，这里不再记一遍 */
export function taggerDownloadLog(): BatchDownloadOptions {
  return { event: 'tagger-download', appendDone: false, errorPrefix: i18n.t('aiTagger.downloadFail') };
}

/** 不经 useBatchTask 的打标流程（辅助打标）用：active() 为 true 时把模型下载写进本页日志 */
export function useTaggerDownloadLog(active: () => boolean, logger: Pick<UnifiedTaskLogger, 'appendDownloadLog'>) {
  const activeRef = useRef(active);
  const loggerRef = useRef(logger);
  useLayoutEffect(() => {
    activeRef.current = active;
    loggerRef.current = logger;
  });
  useEffect(() => {
    let mounted = true;
    const { event } = taggerDownloadLog();
    const unlisten = listen<UnifiedDownloadPayload>(event, ({ payload }) => {
      if (!mounted || !activeRef.current()) return;
      const { appendDone, errorPrefix } = taggerDownloadLog();
      loggerRef.current.appendDownloadLog(payload, { appendDone, errorPrefix });
    });
    return () => {
      mounted = false;
      void unlisten.then(off => off());
    };
  }, []);
}
