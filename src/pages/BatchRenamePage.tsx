import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import {
  TextCursorInput,
  Play,
  Loader2,
  Eye,
  Shuffle,
  ArrowRight,
  Hash,
  Type,
  ChevronLeft,
  ChevronRight,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import DedupRenameTab from '../components/DedupRenameTab';
import InputPathPickerButton from '../components/InputPathPickerButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface PreviewItem { original: string; renamed: string; }

export default function BatchRenamePage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<'number' | 'dedup'>('number');
  const [inputPath, setInputPath] = useState('');
  const [prefix, setPrefix] = useState('img_');
  const [startNumber, setStartNumber] = useState(1);
  const [digitCount, setDigitCount] = useState(4);
  const [shuffleOrder, setShuffleOrder] = useState(false);
  const [renameTags, setRenameTags] = useState(true);
  const [previewPage, setPreviewPage] = useState(0);
  const PREVIEW_PER_PAGE = 15;
  const [previews, setPreviews] = useState<PreviewItem[]>([]);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('rename-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) setProgress((d.current / d.total) * 100);
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs((prev) => [...prev, { time: getTimeStr(), message: d.message, status: d.status === 'done' ? 'info' : d.status as LogEntry['status'] }]);
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, []);

  // 数字字段兜底：输入中途可能为 ""（空串），提交前规整为合法值
  const sanitizeRenameNums = () => {
    const sn = Number.isFinite(startNumber) && startNumber >= 0 ? startNumber : 0;
    const dc = Number.isFinite(digitCount) && digitCount >= 1 ? Math.min(digitCount, 10) : 1;
    if (sn !== startNumber) setStartNumber(sn);
    if (dc !== digitCount) setDigitCount(dc);
    return { sn, dc };
  };

  const handlePreview = async () => {
    if (!inputPath) return;
    const { sn, dc } = sanitizeRenameNums();
    setPreviewLoading(true);
    try {
      const result = await invoke<PreviewItem[]>('preview_rename', {
        options: { input_path: inputPath, prefix, start_number: sn, digit_count: dc, shuffle: shuffleOrder, rename_tags: renameTags },
      });
      setPreviews(result);
    } catch (e: any) {
      setPreviews([]);
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('batchRename.previewFailed')}: ${String(e)}`, status: 'error' }]);
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleShuffle = async () => {
    if (!inputPath) return;
    const { sn, dc } = sanitizeRenameNums();
    setPreviewLoading(true);
    try {
      const result = await invoke<PreviewItem[]>('preview_rename', {
        options: { input_path: inputPath, prefix, start_number: sn, digit_count: dc, shuffle: true, rename_tags: renameTags },
      });
      setPreviews(result);
      setShuffleOrder(true);
    } catch (e: any) {
      setPreviews([]);
    } finally {
      setPreviewLoading(false);
    }
  };

  const { addTask, updateTask } = useTaskQueue();

  const handleExecute = async () => {
    if (!inputPath || previews.length === 0) return;
    const { sn, dc } = sanitizeRenameNums();
    setProcessing(true);
    addTask('rename', t('batchRename.taskName'));
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: t('batchRename.startMsg', { prefix, start: sn, digits: dc }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('execute_rename', {
        options: { input_path: inputPath, prefix, start_number: sn, digit_count: dc, shuffle: shuffleOrder, rename_tags: renameTags },
      });
      setPreviews([]);
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      updateTask('rename', { status: /已取消|cancel/i.test(String(e)) ? 'cancelled' : 'error', message: String(e) });
      setHasError(true); setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);

  // Example preview string
  const exampleNum = String(startNumber).padStart(digitCount, '0');
  const exampleName = `${prefix}${exampleNum}.png`;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <TextCursorInput style={{ width: 28, height: 28, color: '#38bdf8' }} />
          <h1 className="page-title">{t('batchRename.title')}</h1>
        </div>
        <p className="page-subtitle">{t('batchRename.subtitle')}</p>
      </div>

      {/* Tabs */}
      <div style={{
        display: 'flex', gap: 2, marginBottom: 'var(--space-4)',
        background: 'var(--color-bg-card)', borderRadius: 'var(--radius-lg)',
        padding: 3, border: '1px solid var(--color-border)',
        width: 'fit-content',
      }}>
        {[
          { id: 'number' as const, label: t('dedupRename.tabNumber') },
          { id: 'dedup' as const, label: t('dedupRename.tabDedup') },
        ].map(tab => (
          <button key={tab.id} onClick={() => setActiveTab(tab.id)} style={{
            padding: '8px 20px', borderRadius: 'var(--radius-md)', border: 'none',
            cursor: 'pointer', fontSize: 'var(--font-size-sm)', fontWeight: 600,
            transition: 'all 0.2s', fontFamily: 'inherit',
            background: activeTab === tab.id ? 'var(--color-accent-primary)' : 'transparent',
            color: activeTab === tab.id ? '#fff' : 'var(--color-text-tertiary)',
          }}>{tab.label}</button>
        ))}
      </div>

      {/* Tab: Number rename */}
      <div style={{ display: activeTab === 'number' ? 'block' : 'none' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 400px', gap: 'var(--space-6)' }}>
          {/* 左侧 */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
            {/* 路径 */}
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">{t('batchRename.imageFolder')}</span></div>
              <div className="form-group">
                <label className="form-label">{t('batchRename.folderDesc')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('batchRename.selectFolder')} value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <InputPathPickerButton onSelect={(path) => { setInputPath(path); setPreviews([]); }} />
                </div>
              </div>
            </div>

            {/* 命名规则 */}
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">{t('batchRename.namingRule')}</span></div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
                <div className="form-group">
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Type style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                    {t('batchRename.prefix')}
                  </label>
                  <input className="form-input" value={prefix} onChange={(e) => setPrefix(e.target.value)} placeholder={t('batchRename.prefixPlaceholder')} />
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                      {t('batchRename.startNum')}
                    </label>
                    <input className="form-input" type="number" value={startNumber} onChange={e => setStartNumber(e.target.value === "" ? "" as any : Number(e.target.value))} onBlur={e => { if (e.target.value === "") setStartNumber(0); }} min={0} />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                      {t('batchRename.digitCount')}
                    </label>
                    <input className="form-input" type="number" value={digitCount} onChange={e => setDigitCount(e.target.value === "" ? "" as any : Number(e.target.value))} onBlur={e => { if (e.target.value === "") setDigitCount(1); }} min={1} max={10} />
                  </div>
                </div>

                {/* 同步重命名标签文件 */}
                <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }}>
                  <input type="checkbox" checked={renameTags} onChange={e => setRenameTags(e.target.checked)}
                    style={{ accentColor: '#38bdf8', width: 16, height: 16 }} />
                  {t('batchRename.renameTags')}
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>(.txt, .json)</span>
                </label>

                {/* 命名示例 */}
                <div style={{
                  padding: 'var(--space-3) var(--space-4)',
                  borderRadius: 'var(--radius-md)',
                  background: 'rgba(56, 189, 248, 0.06)',
                  border: '1px solid rgba(56, 189, 248, 0.12)',
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                }}>
                  <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>{t('batchRename.namingExample')}</span>
                  <span style={{ fontSize: 'var(--font-size-md)', fontWeight: 700, color: '#38bdf8', fontFamily: 'monospace' }}>{exampleName}</span>
                </div>
              </div>
            </div>

            {/* 操作按钮 */}
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={handlePreview} disabled={!inputPath || previewLoading}>
                {previewLoading ? <Loader2 style={{ width: 16, height: 16, animation: 'spin 1s linear infinite' }} /> : <Eye style={{ width: 16, height: 16 }} />}
                {t('batchRename.generatePreview')}
              </button>
              <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={handleShuffle} disabled={!inputPath || previewLoading}>
                <Shuffle style={{ width: 16, height: 16 }} />
                {t('batchRename.shufflePreview')}
              </button>
            </div>

            {/* 预览表格 */}
            {previews.length > 0 && (() => {
              const totalPages = Math.ceil(previews.length / PREVIEW_PER_PAGE);
              const pageItems = previews.slice(previewPage * PREVIEW_PER_PAGE, (previewPage + 1) * PREVIEW_PER_PAGE);
              const startIdx = previewPage * PREVIEW_PER_PAGE;
              return (
                <div className="tool-panel" style={{ padding: 0, overflow: 'hidden' }}>
                  <div className="tool-panel-header" style={{ padding: 'var(--space-3) var(--space-4)' }}>
                    <span className="tool-panel-title">{t('batchRename.previewTitle')}</span>
                    <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{t('batchRename.fileCount', { count: previews.length })}</span>
                  </div>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr style={{ borderBottom: '1px solid var(--color-border)' }}>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>#</th>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('batchRename.originalName')}</th>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'center', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', width: 30 }}></th>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('batchRename.newName')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {pageItems.map((item, localIdx) => {
                        const idx = startIdx + localIdx;
                        return (
                          <tr key={idx} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                            <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>{idx + 1}</td>
                            <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', fontFamily: 'monospace', wordBreak: 'break-all' }}>{item.original}</td>
                            <td style={{ padding: '6px 0', textAlign: 'center' }}><ArrowRight style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /></td>
                            <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: '#38bdf8', fontFamily: 'monospace', fontWeight: 600 }}>{item.renamed}</td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                  {totalPages > 1 && (
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8, padding: '8px 0', borderTop: '1px solid var(--color-border)' }}>
                      <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 28 }}
                        disabled={previewPage === 0} onClick={() => setPreviewPage(p => p - 1)}>
                        <ChevronLeft style={{ width: 14, height: 14 }} />
                      </button>
                      <span style={{ fontSize: 12, color: 'var(--color-text-secondary)', fontWeight: 600, minWidth: 60, textAlign: 'center' }}>
                        {previewPage + 1} / {totalPages}
                      </span>
                      <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 28 }}
                        disabled={previewPage >= totalPages - 1} onClick={() => setPreviewPage(p => p + 1)}>
                        <ChevronRight style={{ width: 14, height: 14 }} />
                      </button>
                    </div>
                  )}
                </div>
              );
            })()}
          </div>

          {/* 右侧 */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
            <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleExecute}
              disabled={processing || !inputPath || previews.length === 0}>
              {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> {t('batchRename.executing')}</> : <><Play style={{ width: 18, height: 18 }} /> {t('batchRename.executeRename')}</>}
            </button>
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
          </div>
        </div>
      </div>

      {/* Tab: Dedup rename */}
      <div style={{ display: activeTab === 'dedup' ? 'block' : 'none' }}>
        <DedupRenameTab />
      </div>
    </div>
  );
}
