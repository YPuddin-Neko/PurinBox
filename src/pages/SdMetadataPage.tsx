import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { hasTauriRuntime } from '../utils/tauriRuntime';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import ThumbImage from '../components/ThumbImage';
import {
  FileCode2, FolderOpen, Loader2, Eye, Download,
  ImageUp, Clipboard, Check,
} from 'lucide-react';
import ProgressLog from '../components/ProgressLog';
import { useBatchTask } from '../hooks/useBatchTask';
import { Modal } from '../components/Modal';
import PageHeader from '../components/ui/PageHeader';
import Pager from '../components/ui/Pager';
import { useTranslation } from 'react-i18next';
import '../styles/metadata.css';
import RecursiveScanToggle from '../components/RecursiveScanToggle';

interface SdImageMeta { path: string; filename: string; positive: string; negative: string; params: string; source: string; }
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

const sourceColor = (source: string) => SOURCE_COLORS[source] || '#6b7280';

function PromptBlock({ label, text, color, copied, onCopy, monospace = false }: {
  label: string; text: string; color: string; copied: boolean; onCopy: () => void; monospace?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 11, fontWeight: 700, color }}>{label}</span>
        {text && <button className="btn btn-ghost btn-sm" onClick={onCopy} title={t('common.copy')} style={{ padding: '2px 8px', fontSize: 10, gap: 4, height: 22 }}>
          {copied ? <Check size={12} /> : <Clipboard size={12} />}
          {copied ? t('sdMetadata.copied') : null}
        </button>}
      </div>
      <div style={{ fontSize: monospace ? 11 : 12, fontFamily: monospace ? 'monospace' : undefined, lineHeight: monospace ? 1.6 : 1.7, marginTop: 6, padding: '12px 14px', background: color + '0f',
        borderRadius: 8, overflowWrap: 'anywhere', userSelect: 'text', maxHeight: 240, overflowY: 'auto' }}>
        {text || '-'}
      </div>
    </div>
  );
}

export default function SdMetadataPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'sd-metadata-progress', taskId: 'sd-metadata', logDone: false });
  const { logger } = task;
  const { isDone } = task.progressLogProps;
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [items, setItems] = useState<SdImageMeta[]>([]);
  const [sourceCounts, setSourceCounts] = useState<Record<string, number>>({});
  const [page, setPage] = useState(0);
  const [modalItem, setModalItem] = useState<SdImageMeta | null>(null);
  const [showNoMeta, setShowNoMeta] = useState(false);
  const [noMetaFiles, setNoMetaFiles] = useState<string[]>([]);
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const [exportMode, setExportMode] = useState<'same' | 'custom'>('same');
  const [destFolder, setDestFolder] = useState('');
  const [exporting, setExporting] = useState(false);
  const [dragging, setDragging] = useState(false);


  // Tauri native drag-drop (gives real file paths)
  useEffect(() => {
    if (!hasTauriRuntime()) return;
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
        try {
          const result = await invoke<SdImageMeta | null>('read_single_sd_metadata', { filePath });
          if (result) {
            setModalItem(result);
            logger.appendLog(t('sdMetadata.dropFound', { name: result.filename, source: result.source }), 'success');
          } else {
            logger.appendLog(t('sdMetadata.dropNoMeta'), 'warning');
          }
        } catch (err: any) {
          logger.appendLog(String(err), 'error');
        }
      }
    });
    return () => {
      active = false;
      unlisten.then(u => u()).catch(() => {});
    };
  }, [t, logger]);

  const pickFolder = useCallback(async (setter: (v: string) => void, output = false) => {
    const sel = await open({ directory: true, title: t(output ? 'pages.selectOutputTitle' : 'pages.selectInputTitle') });
    if (sel) setter(sel as string);
  }, [t]);

  const handleScan = async () => {
    if (!inputPath || task.processing) return;
    setScanning(true); setPage(0);
    setItems([]); setSourceCounts({}); setNoMetaFiles([]);
    try {
      const result = await task.run({
        taskName: t('sidebar.sdMetadata'), startLog: t('sdMetadata.scanStart'), cancellable: false,
        exec: () => invoke<ScanResult>('scan_sd_metadata', { inputPath, recursive }),
      });
      if (result) {
        setItems(result.items);
        setNoMetaFiles(result.no_meta_files || []);
        setSourceCounts(result.source_counts);
        logger.appendLog(t('sdMetadata.scanDone', {
          total: result.items.length + result.no_meta_files.length, meta: result.items.length,
          time: (result.scan_time_ms / 1000).toFixed(1),
        }), 'success');
      }
    } finally { setScanning(false); }
  };

  const copyText = useCallback(async (text: string, field: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedField(field);
      setTimeout(() => setCopiedField(null), 1500);
    } catch { /* ignore */ }
  }, []);

  const handleExport = async () => {
    if (items.length === 0 || task.processing) return;
    setExporting(true);
    try {
      const result = await task.run({
        taskName: t('sdMetadata.exporting'), startLog: t('sdMetadata.exportStart', { count: items.length }),
        keepLogs: true, cancellable: false,
        exec: () => invoke<{ success_count: number; fail_count: number; skip_count: number; errors: string[] }>('export_sd_tags', {
          options: {
            mode: exportMode, dest_folder: exportMode === 'custom' ? destFolder : null, input_root: inputPath,
            items: items.map(i => ({ source_path: i.path, positive: i.positive })),
          },
        }),
      });
      if (result) logger.appendLog(t('sdMetadata.exportDone', {
        success: result.success_count, fail: result.fail_count, skip: result.skip_count,
      }), result.fail_count > 0 ? 'warning' : 'success');
    } finally { setExporting(false); }
  };

  // 后端每个文件只会落入 items 或 no_meta_files 之一，计数直接由两者长度得出
  const hasMeta = items.length;
  const noMeta = noMetaFiles.length;
  const totalImages = hasMeta + noMeta;
  const totalPages = Math.ceil(items.length / PER_PAGE);
  const pageItems = items.slice(page * PER_PAGE, (page + 1) * PER_PAGE);
  const startIdx = page * PER_PAGE;

  return (
    <div className="page" style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <PageHeader icon={FileCode2} color="#a78bfa" title={t('sdMetadata.title')} subtitle={t('sdMetadata.subtitle')} />

      {/* 网格占满页面剩余高度：结果表在右栏内滚动，翻页栏始终可见 */}
      <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gridTemplateRows: 'minmax(0, 1fr)', gap: 20, flex: 1, minHeight: 420 }}>
        {/* Left panel */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)', minHeight: 0, overflowY: 'auto' }}>
          {/* Folder */}
          <div className="tool-panel">
            <div className="form-group">
              <div className="form-label-row">
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOpen style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                  {t('sdMetadata.inputFolder')}
                </label>
                <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
              </div>
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
            onClick={handleScan} disabled={!inputPath || task.processing}>
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
                    <button className="btn btn-secondary" onClick={() => pickFolder(setDestFolder, true)} style={{ flexShrink: 0 }}>
                      <FolderOpen style={{ width: 14, height: 14 }} />
                    </button>
                  </div>
                </div>
              )}
              <button className="btn btn-primary" style={{ width: '100%', height: 40, marginTop: 'var(--space-2)' }}
                onClick={handleExport} disabled={task.processing || items.length === 0 || (exportMode === 'custom' && !destFolder)}>
                {exporting ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('sdMetadata.exporting')}</>
                  : <><Download style={{ width: 14, height: 14 }} /> {t('sdMetadata.exportTags')} ({items.length})</>}
              </button>
            </div>
          )}

          {/* Progress log */}
          <ProgressLog {...task.progressLogProps} />
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
                  label: src.toUpperCase(), value: cnt, color: sourceColor(src), clickable: false,
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
                          <tr key={idx} className="metadata-row" style={{ borderBottom: '1px solid rgba(255,255,255,0.03)', cursor: 'pointer', transition: 'background 0.15s' }}
                            onClick={() => setModalItem(item)}>
                            <td style={{ padding: '6px 12px', fontSize: 11, color: 'var(--color-text-tertiary)' }}>{idx + 1}</td>
                            <td style={{ padding: '6px 12px' }}>
                              <div style={{ fontSize: 12, fontFamily: 'monospace', color: 'var(--color-text-secondary)', wordBreak: 'break-all' }}>{item.filename}</div>
                            </td>
                            <td style={{ padding: '6px 8px', textAlign: 'center' }}>
                              <span style={{
                                fontSize: 9, fontWeight: 700, padding: '2px 6px', borderRadius: 4,
                                background: `${sourceColor(item.source)}22`,
                                color: sourceColor(item.source),
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
                {totalPages > 1 && <Pager page={page} pages={totalPages} onChange={setPage} footer />}
              </div>
            </>
          )}
        </div>
      </div>

      {modalItem && (
        <Modal open onClose={() => setModalItem(null)} title={modalItem.filename} maxWidth={960} className="metadata-detail-modal" headerIcon={false} bodyStyle={{ padding: 20 }}
          headerExtra={<span style={{ fontSize: 9, fontWeight: 700, padding: '2px 8px', borderRadius: 4, background: `${sourceColor(modalItem.source)}22`, color: sourceColor(modalItem.source) }}>{modalItem.source.toUpperCase()}</span>}>
          <div className="metadata-detail-content">
            <div className="metadata-detail-preview">
              <div style={{ borderRadius: 'var(--radius-md)', overflow: 'hidden', border: '1px solid var(--color-border)', background: 'rgba(0,0,0,0.3)', display: 'flex', alignItems: 'center', justifyContent: 'center', minHeight: 200 }}>
              <ThumbImage path={modalItem.path} maxEdge={1024} alt={modalItem.filename}
                style={{ maxWidth: '100%', maxHeight: 360, objectFit: 'contain' }} />
              </div>
              <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', overflowWrap: 'anywhere', fontFamily: 'monospace' }}>{modalItem.path}</div>
            </div>
            <div style={{ flex: '2 1 280px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: 16 }}>
              <PromptBlock label={t('sdMetadata.positive')} text={modalItem.positive} color="#4ade80"
                copied={copiedField === 'positive'} onCopy={() => copyText(modalItem.positive, 'positive')} />
              <PromptBlock label={t('sdMetadata.negative')} text={modalItem.negative} color="#ef4444"
                copied={copiedField === 'negative'} onCopy={() => copyText(modalItem.negative, 'negative')} />
              {modalItem.params && <PromptBlock label={t('sdMetadata.params')} text={modalItem.params} color="#60a5fa" monospace
                copied={copiedField === 'params'} onCopy={() => copyText(modalItem.params, 'params')} />}
            </div>
          </div>
        </Modal>
      )}
      <Modal open={showNoMeta && noMetaFiles.length > 0} onClose={() => setShowNoMeta(false)}
        title={t('sdMetadata.noMetaFilesTitle')} maxWidth={520}>
        {noMetaFiles.map((file, i) => (
          <div key={file} style={{ padding: '6px 0', fontSize: 12, overflowWrap: 'anywhere', fontFamily: 'monospace' }}>
            {i + 1}. {file}
          </div>
        ))}
      </Modal>
    </div>
  );
}
