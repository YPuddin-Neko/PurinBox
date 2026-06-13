import { useState, useEffect, useCallback } from 'react';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import {
  FileCode2, FolderOpen, Loader2, Eye, Download,
  ChevronLeft, ChevronRight, ImageUp, X, Clipboard, Check,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';

interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface SdImageMeta { path: string; filename: string; positive: string; negative: string; params: string; artist: string; source: string; }
interface ScanResult {
  items: SdImageMeta[]; total_images: number; has_meta_count: number; no_meta_count: number;
  no_meta_files: string[]; source_counts: Record<string, number>; scan_time_ms: number;
}

const PER_PAGE = 15;

const panel: React.CSSProperties = {
  background: 'var(--color-bg-card)', border: '1px solid var(--color-border)',
  borderRadius: 'var(--radius-lg)',
};

const SOURCE_COLORS: Record<string, string> = {
  a1111: '#7c5cfc', comfyui: '#38bdf8', novelai: '#f59e0b', unknown: '#6b7280',
};

export default function SdMetadataPage() {
  const { t } = useTranslation();
  const { addTask, updateTask } = useTaskQueue();
  const [inputPath, setInputPath] = useState('');
  const [scanning, setScanning] = useState(false);
  const [items, setItems] = useState<SdImageMeta[]>([]);
  const [totalImages, setTotalImages] = useState(0);
  const [hasMeta, setHasMeta] = useState(0);
  const [noMeta, setNoMeta] = useState(0);
  const [sourceCounts, setSourceCounts] = useState<Record<string, number>>({});
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [page, setPage] = useState(0);
  const [modalItem, setModalItem] = useState<SdImageMeta | null>(null);
  const [showNoMeta, setShowNoMeta] = useState(false);
  const [noMetaFiles, setNoMetaFiles] = useState<string[]>([]);
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const [exportMode, setExportMode] = useState<'same' | 'custom'>('same');
  const [destFolder, setDestFolder] = useState('');
  const [exporting, setExporting] = useState(false);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('sd-metadata-progress', (e) => {
      if (!active) return;
      const d = e.payload;
      setPCur(d.current); setPTot(d.total);
      if (d.total > 0) setProgress(Math.round((d.current / d.total) * 100));
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs(prev => [...prev, { time: getTimeStr(), message: d.message, status: d.status === 'done' ? 'info' : d.status as LogEntry['status'] }]);
      }
    });
    return () => { active = false; p.then(u => u()); };
  }, []);

  // Tauri native drag-drop (gives real file paths)
  useEffect(() => {
    let active = true;
    const unlisten = getCurrentWebview().onDragDropEvent(async (e) => {
      if (!active) return;
      if (e.payload.type === 'over') {
        setDragging(true);
      } else if (e.payload.type === 'leave') {
        setDragging(false);
      } else if (e.payload.type === 'drop') {
        setDragging(false);
        const paths = e.payload.paths;
        if (!paths || paths.length === 0) return;
        const filePath = paths[0];
        // Check if PNG
        if (!filePath.toLowerCase().endsWith('.png')) {
          setLogs(prev => [...prev, { time: getTimeStr(), message: t('sdMetadata.dropNoMeta'), status: 'warning' }]);
          return;
        }
        try {
          const result = await invoke<SdImageMeta | null>('read_single_sd_metadata', { filePath });
          if (result) {
            setModalItem(result);
            setLogs(prev => [...prev, { time: getTimeStr(), message: t('sdMetadata.dropFound', { name: result.filename, source: result.source }), status: 'success' }]);
          } else {
            setLogs(prev => [...prev, { time: getTimeStr(), message: t('sdMetadata.dropNoMeta'), status: 'warning' }]);
          }
        } catch (err: any) {
          setLogs(prev => [...prev, { time: getTimeStr(), message: String(err), status: 'error' }]);
        }
      }
    });
    return () => { active = false; unlisten.then(u => u()); };
  }, [t]);

  const pickFolder = useCallback(async (setter: (v: string) => void) => {
    const sel = await open({ directory: true, title: t('pages.selectInputTitle') });
    if (sel) setter(sel as string);
  }, [t]);

  const handleScan = async () => {
    if (!inputPath) return;
    setScanning(true); setProgress(0); setPCur(0); setPTot(0);
    setIsDone(false); setHasError(false); setPage(0);
    setItems([]); setTotalImages(0); setHasMeta(0); setNoMeta(0); setSourceCounts({}); setNoMetaFiles([]);
    setLogs([{ time: getTimeStr(), message: t('sdMetadata.scanStart'), status: 'info' }]);
    addTask('sd-metadata', t('sidebar.sdMetadata'));
    try {
      const result = await invoke<ScanResult>('scan_sd_metadata', { inputPath });
      setItems(result.items);
      setTotalImages(result.total_images);
      setHasMeta(result.has_meta_count);
      setNoMeta(result.no_meta_count);
      setNoMetaFiles(result.no_meta_files || []);
      setSourceCounts(result.source_counts);
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: t('sdMetadata.scanDone', { total: result.total_images, meta: result.has_meta_count, time: (result.scan_time_ms / 1000).toFixed(1) }),
        status: 'success',
      }]);
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: String(e), status: 'error' }]);
      updateTask('sd-metadata', { status: /已取消|cancel/i.test(String(e)) ? 'cancelled' : 'error', message: String(e) });
      setHasError(true);
    } finally { setIsDone(true); setScanning(false); }
  };

  const copyText = useCallback(async (text: string, field: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedField(field);
      setTimeout(() => setCopiedField(null), 1500);
    } catch { /* ignore */ }
  }, []);

  const handleExport = async () => {
    if (items.length === 0) return;
    setExporting(true);
    setLogs(prev => [...prev, { time: getTimeStr(), message: t('sdMetadata.exportStart', { count: items.length }), status: 'info' }]);
    addTask('sd-metadata', t('sdMetadata.exporting'));
    try {
      const result = await invoke<{ success_count: number; fail_count: number; skip_count: number; errors: string[] }>('export_sd_tags', {
        options: {
          mode: exportMode,
          dest_folder: exportMode === 'custom' ? destFolder : null,
          items: items.map(i => ({ source_path: i.path, positive: i.positive })),
        },
      });
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: t('sdMetadata.exportDone', { success: result.success_count, fail: result.fail_count, skip: result.skip_count }),
        status: result.fail_count > 0 ? 'warning' : 'success',
      }]);
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: `${t('sdMetadata.exportFail')}: ${String(e)}`, status: 'error' }]);
      updateTask('sd-metadata', { status: /已取消|cancel/i.test(String(e)) ? 'cancelled' : 'error', message: String(e) });
    } finally { setExporting(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);

  const totalPages = Math.ceil(items.length / PER_PAGE);
  const pageItems = items.slice(page * PER_PAGE, (page + 1) * PER_PAGE);
  const startIdx = page * PER_PAGE;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <FileCode2 style={{ width: 28, height: 28, color: '#a78bfa' }} />
          <h1 className="page-title">{t('sdMetadata.title')}</h1>
        </div>
        <p className="page-subtitle">{t('sdMetadata.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gap: 20, minHeight: 'calc(100vh - 260px)' }}>
        {/* Left panel */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* Folder */}
          <div className="tool-panel">
            <div className="form-group">
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <FolderOpen style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                {t('sdMetadata.inputFolder')}
              </label>
              <div style={{ display: 'flex', gap: 8 }}>
                <input className="form-input" value={inputPath} readOnly placeholder={t('sdMetadata.selectFolder')} style={{ flex: 1 }} />
                <button className="btn btn-secondary" onClick={() => pickFolder(setInputPath)} style={{ flexShrink: 0 }}>
                  <FolderOpen style={{ width: 14, height: 14 }} />
                </button>
              </div>
            </div>
          </div>

          {/* Scan */}
          <button className="btn btn-primary" style={{ width: '100%', height: 44 }}
            onClick={handleScan} disabled={!inputPath || scanning}>
            {scanning ? <><Loader2 style={{ width: 16, height: 16, animation: 'spin 1s linear infinite' }} /> {t('sdMetadata.scanning')}</>
              : <><Eye style={{ width: 16, height: 16 }} /> {t('sdMetadata.scan')}</>}
          </button>

          {/* Export settings */}
          {items.length > 0 && (
            <div className="tool-panel" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">{t('sdMetadata.exportMode')}</label>
                <div style={{ display: 'flex', gap: 6 }}>
                  {(['same', 'custom'] as const).map(m => (
                    <button key={m} className={`btn ${exportMode === m ? 'btn-primary' : 'btn-secondary'}`}
                      onClick={() => setExportMode(m)} style={{ flex: 1, fontSize: 12, height: 32 }}>
                      {t(`sdMetadata.exportMode_${m}`)}
                    </button>
                  ))}
                </div>
              </div>
              {exportMode === 'custom' && (
                <div className="form-group">
                  <label className="form-label">{t('sdMetadata.destFolder')}</label>
                  <div style={{ display: 'flex', gap: 8 }}>
                    <input className="form-input" value={destFolder} readOnly placeholder={t('sdMetadata.selectFolder')} style={{ flex: 1 }} />
                    <button className="btn btn-secondary" onClick={() => pickFolder(setDestFolder)} style={{ flexShrink: 0 }}>
                      <FolderOpen style={{ width: 14, height: 14 }} />
                    </button>
                  </div>
                </div>
              )}
              <button className="btn btn-primary" style={{ width: '100%', height: 40, marginTop: 'var(--space-2)' }}
                onClick={handleExport} disabled={exporting || items.length === 0 || (exportMode === 'custom' && !destFolder)}>
                {exporting ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('sdMetadata.exporting')}</>
                  : <><Download style={{ width: 14, height: 14 }} /> {t('sdMetadata.exportTags')} ({items.length})</>}
              </button>
            </div>
          )}

          {/* Progress log */}
          <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>

        {/* Right panel */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minHeight: 300, overflow: 'hidden' }}>
          {/* Stats */}
          {items.length > 0 && (
            <div style={{ display: 'flex', gap: 8, flexShrink: 0 }}>
              {[
                { label: t('sdMetadata.statTotal'), value: totalImages, color: '#60a5fa', clickable: false },
                { label: t('sdMetadata.statHasMeta'), value: hasMeta, color: '#4ade80', clickable: false },
                { label: t('sdMetadata.statNoMeta'), value: noMeta, color: '#ef4444', clickable: noMeta > 0 },
                ...Object.entries(sourceCounts).map(([src, cnt]) => ({
                  label: src.toUpperCase(), value: cnt, color: SOURCE_COLORS[src] || '#6b7280', clickable: false,
                })),
              ].map(s => (
                <div key={s.label}
                  onClick={s.clickable ? () => setShowNoMeta(true) : undefined}
                  style={{
                    ...panel, flex: 1, padding: '8px 10px', display: 'flex', alignItems: 'center', gap: 6,
                    cursor: s.clickable ? 'pointer' : 'default',
                    transition: 'all 0.15s',
                    ...(s.clickable ? { borderColor: 'rgba(239,68,68,0.3)' } : {}),
                  }}>
                  <span style={{ fontSize: 16, fontWeight: 800, color: s.color, fontFamily: 'monospace' }}>{s.value}</span>
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', fontWeight: 600, lineHeight: 1.2 }}>
                    {s.label}{s.clickable ? ' ▸' : ''}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* Empty / scanning / drop zone */}
          {items.length === 0 ? (
            <div
              style={{
                ...panel, flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
                flexDirection: 'column', gap: 12, color: 'var(--color-text-tertiary)',
                border: dragging ? '2px dashed var(--color-accent-primary)' : '1px solid var(--color-border)',
                background: dragging ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-card)',
                transition: 'all 0.2s',
              }}>
              {dragging ? (
                <>
                  <ImageUp style={{ width: 48, height: 48, opacity: 0.4, color: 'var(--color-accent-primary)' }} />
                  <span style={{ fontSize: 13, color: 'var(--color-accent-primary)' }}>{t('sdMetadata.dropRelease')}</span>
                </>
              ) : (
                <>
                  <FileCode2 style={{ width: 48, height: 48, opacity: 0.15 }} />
                  <span style={{ fontSize: 13 }}>{scanning ? t('sdMetadata.scanning') : isDone ? t('sdMetadata.noMeta') : t('sdMetadata.hint')}</span>
                  <span style={{ fontSize: 11, opacity: 0.5, marginTop: -4 }}>{t('sdMetadata.dropHint')}</span>
                </>
              )}
            </div>
          ) : (
            <>
              {/* Table */}
              <div style={{ ...panel, flex: 1, padding: 0, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
                <div style={{ padding: '10px 16px', borderBottom: '1px solid var(--color-border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <span style={{ fontSize: 13, fontWeight: 700 }}>{t('sdMetadata.metaList')}</span>
                  <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{items.length} {t('sdMetadata.items')}</span>
                </div>
                <div style={{ flex: 1, overflowY: 'auto' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr style={{ borderBottom: '1px solid var(--color-border)' }}>
                        <th style={{ padding: '8px 12px', textAlign: 'left', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600, width: 30 }}>#</th>
                        <th style={{ padding: '8px 12px', textAlign: 'left', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('sdMetadata.filename')}</th>
                        <th style={{ padding: '8px 12px', textAlign: 'center', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600, width: 70 }}>{t('sdMetadata.source')}</th>
                        <th style={{ padding: '8px 12px', textAlign: 'left', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('sdMetadata.positivePreview')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {pageItems.map((item, localIdx) => {
                        const idx = startIdx + localIdx;
                        return (
                          <tr key={idx} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)', cursor: 'pointer', transition: 'background 0.15s' }}
                            onClick={() => setModalItem(item)}
                            onMouseEnter={e => (e.currentTarget.style.background = 'rgba(124,92,252,0.05)')}
                            onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}>
                            <td style={{ padding: '6px 12px', fontSize: 11, color: 'var(--color-text-tertiary)' }}>{idx + 1}</td>
                            <td style={{ padding: '6px 12px' }}>
                              <div style={{ fontSize: 12, fontFamily: 'monospace', color: 'var(--color-text-secondary)', wordBreak: 'break-all' }}>{item.filename}</div>
                            </td>
                            <td style={{ padding: '6px 8px', textAlign: 'center' }}>
                              <span style={{
                                fontSize: 9, fontWeight: 700, padding: '2px 6px', borderRadius: 4,
                                background: `${SOURCE_COLORS[item.source] || '#6b7280'}22`,
                                color: SOURCE_COLORS[item.source] || '#6b7280',
                              }}>{item.source.toUpperCase()}</span>
                            </td>
                            <td style={{ padding: '6px 12px', fontSize: 11, color: 'var(--color-text-tertiary)', maxWidth: 300, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                              {item.positive.slice(0, 80)}{item.positive.length > 80 ? '...' : ''}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
                {/* Pagination */}
                {totalPages > 1 && (
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8, padding: '8px 0', borderTop: '1px solid var(--color-border)' }}>
                    <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 28 }}
                      disabled={page === 0} onClick={() => { setPage(p => p - 1); }}>
                      <ChevronLeft style={{ width: 14, height: 14 }} />
                    </button>
                    <span style={{ fontSize: 12, color: 'var(--color-text-secondary)', fontWeight: 600, minWidth: 60, textAlign: 'center' }}>
                      {page + 1} / {totalPages}
                    </span>
                    <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 28 }}
                      disabled={page >= totalPages - 1} onClick={() => { setPage(p => p + 1); }}>
                      <ChevronRight style={{ width: 14, height: 14 }} />
                    </button>
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>

      {/* ─── Detail Modal ─── */}
      {modalItem && (
        <div style={{
          position: 'fixed', inset: 0, zIndex: 999,
          background: 'rgba(0,0,0,0.65)', backdropFilter: 'blur(6px)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          animation: 'fadeIn 0.15s ease',
        }} onClick={() => setModalItem(null)}>
          <div style={{
            background: 'var(--color-bg-card)', border: '1px solid var(--color-border)',
            borderRadius: 'var(--radius-lg)', width: '80vw', maxWidth: 960, maxHeight: '85vh',
            display: 'flex', flexDirection: 'column', overflow: 'hidden',
            boxShadow: '0 24px 64px rgba(0,0,0,0.5)',
          }} onClick={e => e.stopPropagation()}>
            {/* Modal header */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '16px 20px', borderBottom: '1px solid var(--color-border)' }}>
              <span style={{ fontSize: 14, fontWeight: 700, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{modalItem.filename}</span>
              <span style={{
                fontSize: 9, fontWeight: 700, padding: '2px 8px', borderRadius: 4,
                background: `${SOURCE_COLORS[modalItem.source] || '#6b7280'}22`,
                color: SOURCE_COLORS[modalItem.source] || '#6b7280',
              }}>{modalItem.source.toUpperCase()}</span>
              <button className="btn btn-ghost" style={{ padding: 4, minWidth: 28, height: 28 }}
                onClick={() => setModalItem(null)}>
                <X style={{ width: 16, height: 16 }} />
              </button>
            </div>
            {/* Modal body */}
            <div style={{ flex: 1, overflow: 'auto', padding: 20, display: 'flex', gap: 20 }}>
              {/* Image preview */}
              <div style={{ flex: '0 0 280px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{
                  borderRadius: 'var(--radius-md)', overflow: 'hidden',
                  border: '1px solid var(--color-border)', background: 'rgba(0,0,0,0.3)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center', minHeight: 200,
                }}>
                  <img src={convertFileSrc(modalItem.path)}
                    alt={modalItem.filename}
                    style={{ maxWidth: '100%', maxHeight: 360, objectFit: 'contain' }}
                    onError={e => { (e.target as HTMLImageElement).style.display = 'none'; }} />
                </div>
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', wordBreak: 'break-all', fontFamily: 'monospace' }}>
                  {modalItem.path}
                </div>
              </div>
              {/* Metadata */}
              <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
                <div>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <span style={{ fontSize: 11, fontWeight: 700, color: '#4ade80', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{t('sdMetadata.positive')}</span>
                    {modalItem.positive && (
                      <button className="btn btn-ghost" style={{ padding: '2px 8px', fontSize: 10, gap: 4, height: 22 }}
                        onClick={() => copyText(modalItem.positive, 'positive')}>
                        {copiedField === 'positive' ? <Check style={{ width: 12, height: 12, color: '#4ade80' }} /> : <Clipboard style={{ width: 12, height: 12 }} />}
                        {copiedField === 'positive' ? t('sdMetadata.copied') : ''}
                      </button>
                    )}
                  </div>
                  <div style={{ fontSize: 12, color: 'var(--color-text-secondary)', lineHeight: 1.7, marginTop: 6, padding: '12px 14px', background: 'rgba(74,222,128,0.06)', borderRadius: 8, wordBreak: 'break-all', userSelect: 'text', maxHeight: 240, overflowY: 'auto' }}>
                    {modalItem.positive || '-'}
                  </div>
                </div>
                <div>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <span style={{ fontSize: 11, fontWeight: 700, color: '#ef4444', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{t('sdMetadata.negative')}</span>
                    {modalItem.negative && (
                      <button className="btn btn-ghost" style={{ padding: '2px 8px', fontSize: 10, gap: 4, height: 22 }}
                        onClick={() => copyText(modalItem.negative, 'negative')}>
                        {copiedField === 'negative' ? <Check style={{ width: 12, height: 12, color: '#4ade80' }} /> : <Clipboard style={{ width: 12, height: 12 }} />}
                        {copiedField === 'negative' ? t('sdMetadata.copied') : ''}
                      </button>
                    )}
                  </div>
                  <div style={{ fontSize: 12, color: 'var(--color-text-tertiary)', lineHeight: 1.7, marginTop: 6, padding: '12px 14px', background: 'rgba(239,68,68,0.06)', borderRadius: 8, wordBreak: 'break-all', userSelect: 'text', maxHeight: 160, overflowY: 'auto' }}>
                    {modalItem.negative || '-'}
                  </div>
                </div>
                {modalItem.params && (
                  <div>
                    <span style={{ fontSize: 11, fontWeight: 700, color: '#60a5fa', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{t('sdMetadata.params')}</span>
                    <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 6, fontFamily: 'monospace', lineHeight: 1.6, userSelect: 'text', padding: '10px 14px', background: 'rgba(96,165,250,0.06)', borderRadius: 8, wordBreak: 'break-all' }}>
                      {modalItem.params}
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* ─── No-Meta Files Modal ─── */}
      {showNoMeta && noMetaFiles.length > 0 && (
        <div style={{
          position: 'fixed', inset: 0, zIndex: 999,
          background: 'rgba(0,0,0,0.65)', backdropFilter: 'blur(6px)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          animation: 'fadeIn 0.15s ease',
        }} onClick={() => setShowNoMeta(false)}>
          <div style={{
            background: 'var(--color-bg-card)', border: '1px solid var(--color-border)',
            borderRadius: 'var(--radius-lg)', width: 520, maxHeight: '70vh',
            display: 'flex', flexDirection: 'column', overflow: 'hidden',
            boxShadow: '0 24px 64px rgba(0,0,0,0.5)',
          }} onClick={e => e.stopPropagation()}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '16px 20px', borderBottom: '1px solid var(--color-border)' }}>
              <span style={{ fontSize: 14, fontWeight: 700, flex: 1 }}>{t('sdMetadata.noMetaFilesTitle')}</span>
              <button className="btn btn-ghost" style={{ padding: 4, minWidth: 28, height: 28 }}
                onClick={() => setShowNoMeta(false)}>
                <X style={{ width: 16, height: 16 }} />
              </button>
            </div>
            <div style={{ padding: '12px 20px', fontSize: 12, color: 'var(--color-text-tertiary)', borderBottom: '1px solid var(--color-border)' }}>
              {t('sdMetadata.noMetaFilesDesc', { count: noMetaFiles.length })}
            </div>
            <div style={{ flex: 1, overflowY: 'auto', padding: '8px 0' }}>
              {noMetaFiles.map((f, i) => (
                <div key={i} style={{
                  padding: '6px 20px', fontSize: 12, fontFamily: 'monospace',
                  color: 'var(--color-text-secondary)',
                  borderBottom: '1px solid rgba(255,255,255,0.02)',
                }}>
                  {i + 1}. {f}
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
