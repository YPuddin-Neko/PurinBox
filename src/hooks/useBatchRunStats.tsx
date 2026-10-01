import { useCallback, useRef, useState } from 'react';
import { CheckCircle2, XCircle, AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { UnifiedProgressPayload, UnifiedTaskLogger } from './useUnifiedTaskLogs';

export function useBatchRunStats() {
  const { t } = useTranslation();
  const empty = () => ({ success: 0, failed: 0, warnings: 0, errors: [] as string[], warningFiles: [] as string[] });
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
    if (!p.filename || !['success', 'warning', 'error'].includes(p.status)) return;
    const next = { ...current.current };
    if (p.status === 'success' || p.status === 'warning') next.success++;
    if (p.status === 'error') { next.failed++; next.errors = [...next.errors, p.filename]; }
    if (p.status === 'warning') { next.warnings++; next.warningFiles = [...next.warningFiles, p.filename]; }
    current.current = next;
    setStats(next);
    // IPC can resolve before the final progress events arrive.
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
  const headerExtra = <span style={{ display: 'flex', gap: 10, alignItems: 'center', fontSize: 12 }}>
    <span title={t('common.success')} style={{ color: 'var(--color-success)', display: 'flex', gap: 4 }}><CheckCircle2 size={13} />{stats.success}</span>
    <span title={t('common.failed')} style={{ color: 'var(--color-error)', display: 'flex', gap: 4 }}><XCircle size={13} />{stats.failed}</span>
    {stats.warnings > 0 && <span title={t('common.warning')} style={{ color: 'var(--color-warning)', display: 'flex', gap: 4 }}><AlertTriangle size={13} />{stats.warnings}</span>}
  </span>;
  return { reset, onEvent, summarize, headerExtra };
}
