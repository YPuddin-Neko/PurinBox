import { useCallback, useRef, useState } from 'react';
import { CheckCircle2, XCircle, AlertTriangle, SkipForward } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { isSkippedProgressEvent, type UnifiedProgressPayload, type UnifiedTaskLogger } from './useUnifiedTaskLogs';

export function useBatchRunStats() {
  const { t } = useTranslation();
  const empty = () => ({ success: 0, skipped: 0, failed: 0, warnings: 0, errors: [] as string[], warningFiles: [] as string[] });
  const current = useRef(empty());
  const [stats, setStats] = useState(empty);
  const summarized = useRef(false);
  const summaryLogger = useRef<UnifiedTaskLogger | null>(null);
  const reset = useCallback(() => {
    current.current = empty();
    summarized.current = false;
    summaryLogger.current = null;
    setStats(current.current);
  }, []);
  const onEvent = useCallback((p: UnifiedProgressPayload) => {
    if (!p.filename || !['success', 'warning', 'error', 'skipped'].includes(p.status)) return;
    const next = { ...current.current };
    if (isSkippedProgressEvent(p)) {
      next.skipped++;
    } else {
      if (p.status === 'success' || (p.status === 'warning' && !p.unwritten)) next.success++;
      if (p.status === 'error') { next.failed++; next.errors = [...next.errors, p.filename]; }
      if (p.status === 'warning') { next.warnings++; next.warningFiles = [...next.warningFiles, p.filename]; }
    }
    current.current = next;
    setStats(next);
    // 命令可能先于最后几条进度事件返回：汇总写完后才到的失败、警告逐条补记
    if (summaryLogger.current && (p.status === 'error' || p.status === 'warning')) {
      summaryLogger.current.appendLog(`${t(p.status === 'error' ? 'tagSort.failedFiles' : 'tagSort.warnFiles')}: ${p.filename}`, p.status);
    }
  }, [t]);
  const summarize = useCallback((logger: UnifiedTaskLogger) => {
    if (summarized.current) return;
    summarized.current = true;
    summaryLogger.current = logger;
    const { errors, warningFiles } = current.current;
    if (errors.length) logger.appendLog(`${t('tagSort.failedFiles')}: ${errors.join(', ')}`, 'error');
    if (warningFiles.length) logger.appendLog(`${t('tagSort.warnFiles')}: ${warningFiles.join(', ')}`, 'warning');
  }, [t]);
  const hasStats = stats.success > 0 || stats.skipped > 0 || stats.warnings > 0 || stats.failed > 0;
  const headerExtra = hasStats ? (
    <span className="log-stat-badges">
      {stats.success > 0 && <span className="log-stat-badge is-ok" title={t('common.success')}><CheckCircle2 size={12} />{stats.success}</span>}
      {stats.skipped > 0 && <span className="log-stat-badge is-skip" title={t('common.skipped')}><SkipForward size={12} />{stats.skipped}</span>}
      {stats.warnings > 0 && <span className="log-stat-badge is-warn" title={t('common.warning')}><AlertTriangle size={12} />{stats.warnings}</span>}
      {stats.failed > 0 && <span className="log-stat-badge is-err" title={t('common.failed')}><XCircle size={12} />{stats.failed}</span>}
    </span>
  ) : null;
  return { reset, onEvent, summarize, headerExtra };
}
