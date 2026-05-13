import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  Grid3X3,
  FolderOpen,
  Play,
  Loader2,
  Download,
  ImageIcon,
  ChevronLeft,
  ChevronRight,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface BucketImageInfo {
  path: string;
  name: string;
  orig_width: number;
  orig_height: number;
}

interface BucketGroup {
  index: number;
  bucket_width: number;
  bucket_height: number;
  image_count: number;
  total_count: number;
  aspect_ratio: number;
  images: BucketImageInfo[];
}

interface BucketAnalysis {
  total_images: number;
  total_count: number;
  bucket_count: number;
  skipped: [string, string][];

  buckets: BucketGroup[];
}

interface ScanProgress {
  current: number;
  total: number;
  status: string;
  message: string;
}

function bucketColor(ratio: number): string {
  const hue = ((ratio - 0.3) / 2.5) * 300;
  return `hsl(${Math.round(hue % 360)}, 65%, 55%)`;
}

export default function BucketPreviewPage() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [resWidth, setResWidth] = useState(1024);
  const [resHeight, setResHeight] = useState(1024);
  const [steps, setSteps] = useState(32);
  const [noUpscale, setNoUpscale] = useState(true);
  const [repeats, setRepeats] = useState(1);

  const stepsError = steps < 32 || (steps > 32 && steps % 64 !== 0);


  const [analyzing, setAnalyzing] = useState(false);
  const [analysis, setAnalysis] = useState<BucketAnalysis | null>(null);
  const [scanMsg, setScanMsg] = useState('');
  const [scanProgress, setScanProgress] = useState(0);

  const [enableExport, setEnableExport] = useState(false);
  const [exportPath, setExportPath] = useState('');
  const [exporting, setExporting] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: 'success' | 'error' } | null>(null);

  const [expandedBuckets, setExpandedBuckets] = useState<Set<number>>(new Set());
  const [bucketPage, setBucketPage] = useState(0);
  const BUCKETS_PER_PAGE = 3;

  useEffect(() => {
    let active = true;
    const p1 = listen<ScanProgress>('bucket-progress', (e) => {
      if (!active) return;
      setScanMsg(e.payload.message);
      if (e.payload.total > 0) setScanProgress((e.payload.current / e.payload.total) * 100);
    });
    const p2 = listen<ScanProgress>('bucket-export-progress', (e) => {
      if (!active) return;
      setToast({ msg: e.payload.message, type: 'success' });
      setTimeout(() => setToast(null), 3000);
    });
    return () => { active = false; p1.then(fn => fn()); p2.then(fn => fn()); };
  }, []);

  const selectInputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('bucketPreview.selectDataset') });
    if (selected) setInputPath(selected as string);
  };

  const selectExportFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('bucketPreview.selectExport') });
    if (selected) setExportPath(selected as string);
  };

  const handleAnalyze = async () => {
    if (!inputPath) return;
    setAnalyzing(true);
    setAnalysis(null);
    setScanProgress(0);
    setScanMsg(t('bucketPreview.scanning'));
    setExpandedBuckets(new Set());
    setBucketPage(0);
    try {
      const result = await invoke<BucketAnalysis>('analyze_buckets', {
        options: { input_path: inputPath, res_width: resWidth, res_height: resHeight, steps, no_upscale: noUpscale, repeats },
      });
      setAnalysis(result);
    } catch (e: any) {
      setScanMsg(`${t('pages.errorPrefix')}: ${String(e)}`);
    } finally {
      setAnalyzing(false);
    }
  };

  const showToast = (msg: string, type: 'success' | 'error') => {
    setToast({ msg, type });
    setTimeout(() => setToast(null), 3000);
  };

  const handleExport = async () => {
    if (!analysis || !exportPath) return;
    setExporting(true);
    try {
      const msg = await invoke<string>('export_buckets', { analysis, outputPath: exportPath, repeats });
      showToast(msg, 'success');
    } catch (e: any) {
      showToast(`${t('bucketPreview.exportFailed')}: ${String(e)}`, 'error');
    } finally {
      setExporting(false);
    }
  };

  const toggleBucket = (idx: number) => {
    setExpandedBuckets(prev => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx); else next.add(idx);
      return next;
    });
  };

  return (
    <div className="page" style={{ height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden', position: 'relative' }}>
      {/* Toast */}
      {toast && (
        <div style={{
          position: 'absolute', top: 24, left: '50%', transform: 'translateX(-50%)',
          zIndex: 999, padding: '10px 24px', borderRadius: 'var(--radius-lg)',
          background: toast.type === 'success' ? 'rgba(34,197,94,0.95)' : 'rgba(239,68,68,0.95)',
          color: '#fff', fontSize: 13, fontWeight: 600, whiteSpace: 'nowrap',
          boxShadow: '0 4px 20px rgba(0,0,0,0.25)',
          animation: 'toast-in 0.3s ease',
          pointerEvents: 'none',
        }}>
          {toast.type === 'success' ? '✓' : '✕'} {toast.msg}
        </div>
      )}
      <style>{`@keyframes toast-in { from { opacity: 0; transform: translateX(-50%) translateY(-10px); } to { opacity: 1; transform: translateX(-50%) translateY(0); } }`}</style>
      {/* Header */}
      <div className="page-header" style={{ flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Grid3X3 style={{ width: 28, height: 28, color: '#f59e0b' }} />
          <h1 className="page-title">{t('bucketPreview.title')}</h1>
        </div>
        <p className="page-subtitle">{t('bucketPreview.subtitle')}</p>
      </div>

      {/* Params */}
      <div className="tool-panel" style={{ flexShrink: 0 }}>
        <div className="tool-panel-header"><span className="tool-panel-title">{t('bucketPreview.paramSettings')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <div className="form-group">
            <label className="form-label">{t('bucketPreview.datasetFolder')}</label>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              <input className="form-input" placeholder={t('bucketPreview.datasetPlaceholder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
              <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
            </div>
          </div>

          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr) auto', gap: 8, alignItems: 'end' }}>
            <div>
              <label className="form-label" style={{ fontSize: 10 }}>{t('bucketPreview.resWidth')}</label>
              <input className="form-input" type="number" value={resWidth} onChange={e => setResWidth(Number(e.target.value))} min={64} step={64} style={{ height: 32 }} />
            </div>
            <div>
              <label className="form-label" style={{ fontSize: 10 }}>{t('bucketPreview.resHeight')}</label>
              <input className="form-input" type="number" value={resHeight} onChange={e => setResHeight(Number(e.target.value))} min={64} step={64} style={{ height: 32 }} />
            </div>
            <div style={{ position: 'relative' }}>
              <label className="form-label" style={{ fontSize: 10, color: stepsError ? '#ef4444' : undefined }}>{t('bucketPreview.stepsLabel')}</label>
              <input className="form-input" type="number" value={steps} onChange={e => setSteps(Number(e.target.value))} min={32} step={32} style={{
                height: 32,
                borderColor: stepsError ? '#ef4444' : undefined,
                boxShadow: stepsError ? '0 0 0 1px #ef4444' : undefined,
              }} />
              {stepsError && <div style={{
                position: 'absolute', top: '100%', left: 0, marginTop: 2,
                fontSize: 9, color: '#ef4444', whiteSpace: 'nowrap',
              }}>{t('bucketPreview.stepsError')}</div>}
            </div>
            <div>
              <label className="form-label" style={{ fontSize: 10 }}>Repeats</label>
              <input className="form-input" type="number" value={repeats} onChange={e => setRepeats(Number(e.target.value))} min={1} style={{ height: 32 }} />
            </div>

            {/* 不使用桶放大 */}
            <div onClick={() => setNoUpscale(!noUpscale)} style={{
              display: 'flex', alignItems: 'center', gap: 8, height: 32,
              padding: '0 10px', borderRadius: 'var(--radius-md)',
              border: `1px solid ${noUpscale ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
              background: noUpscale ? 'rgba(124,58,237,0.06)' : 'var(--color-bg-secondary)',
              cursor: 'pointer', userSelect: 'none', flexShrink: 0,
              transition: 'all 0.2s',
            }}>
              <span style={{ fontSize: 10, color: 'var(--color-text-secondary)', whiteSpace: 'nowrap' }}>{t('bucketPreview.noUpscale')}</span>
              <div style={{
                width: 30, height: 16, borderRadius: 8, transition: 'all 0.2s',
                background: noUpscale ? 'var(--color-accent-primary)' : 'var(--color-border)',
                position: 'relative', flexShrink: 0,
              }}>
                <div style={{ width: 12, height: 12, borderRadius: '50%', background: '#fff', position: 'absolute', top: 2, left: noUpscale ? 16 : 2, transition: 'left 0.2s' }} />
              </div>
            </div>
          </div>

          <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
            {analyzing && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', whiteSpace: 'nowrap', flexShrink: 0 }}>{scanMsg}</span>}
            {analyzing && <div style={{ flex: 1, height: 4, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}>
              <div style={{ height: '100%', width: `${scanProgress}%`, background: 'var(--color-accent-primary)', borderRadius: 2, transition: 'width 0.3s' }} />
            </div>}
            {!analyzing && <div style={{ flex: 1 }} />}
            <button className="btn btn-primary" style={{ height: 34, padding: '0 20px', flexShrink: 0 }} onClick={handleAnalyze} disabled={analyzing || !inputPath || stepsError}>
              {analyzing ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('bucketPreview.previewing')}</> : <><Play style={{ width: 14, height: 14 }} /> {t('bucketPreview.startPreview')}</>}
            </button>
          </div>
        </div>
      </div>

      {/* Results */}
      {analysis && (
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', marginTop: 'var(--space-4)' }}>
          {/* Stats */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 16, flexShrink: 0, marginBottom: 'var(--space-3)' }}>
            <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: '#4ade80' }} />
              <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('bucketPreview.nBuckets', { n: analysis.bucket_count })}</span>
            </div>
            <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.nImages', { n: analysis.total_images })}</span>
            <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.totalCount', { n: analysis.total_count })}</span>
            {analysis.skipped.length > 0 && <span style={{ fontSize: 11, color: '#f87171' }}>{t('bucketPreview.readFail', { n: analysis.skipped.length })}</span>}

          </div>

          {/* Bucket grid — fixed 3 columns, paginated */}
          {(() => {
            const totalBucketPages = Math.ceil(analysis.buckets.length / BUCKETS_PER_PAGE);
            const pageBuckets = analysis.buckets.slice(bucketPage * BUCKETS_PER_PAGE, (bucketPage + 1) * BUCKETS_PER_PAGE);
            return (
              <>
          <div style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10, alignItems: 'start' }}>
              {pageBuckets.map(bucket => {
                const color = bucketColor(bucket.aspect_ratio);
                const isExpanded = expandedBuckets.has(bucket.index);
                const isLandscape = bucket.bucket_width > bucket.bucket_height;
                const isPortrait = bucket.bucket_height > bucket.bucket_width;
                const orientLabel = isLandscape ? t('bucketPreview.orientLandscape') : isPortrait ? t('bucketPreview.orientPortrait') : t('bucketPreview.orientSquare');
                const maxSide = 34;
                const ratio = bucket.bucket_width / bucket.bucket_height;
                const pw = ratio >= 1 ? maxSide : Math.round(maxSide * ratio);
                const ph = ratio >= 1 ? Math.round(maxSide / ratio) : maxSide;
                const pct = Math.min(100, (bucket.image_count / analysis.total_images) * 100);

                return (
                  <div key={bucket.index} onClick={() => toggleBucket(bucket.index)} style={{
                    aspectRatio: '1', borderRadius: 10, cursor: 'pointer',
                    border: `2px solid ${isExpanded ? color : 'var(--color-border)'}`,
                    background: isExpanded ? `${color}08` : 'var(--color-bg-secondary)',
                    transition: 'border-color 0.3s, background 0.3s, box-shadow 0.3s',
                    boxShadow: isExpanded ? `0 0 0 1px ${color}30` : 'none',
                    overflow: 'hidden',
                    display: 'flex', flexDirection: 'column',
                    position: 'relative',
                  }}>
                    {/* Top-left: index */}
                    <div style={{ position: 'absolute', top: 6, left: 8, fontSize: 9, fontWeight: 700, color: 'var(--color-text-tertiary)', zIndex: 1 }}>#{bucket.index}</div>
                    {/* Top-right: orientation badge */}
                    <div style={{
                      position: 'absolute', top: 6, right: 8, zIndex: 1,
                      width: 20, height: 20, borderRadius: 5, background: color, opacity: 0.85,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      fontSize: 10, fontWeight: 800, color: '#fff',
                    }}>{orientLabel}</div>

                    {/* Top spacer — centers content when collapsed, collapses when expanded */}
                    <div style={{ flex: isExpanded ? 0 : 1, transition: 'flex 0.35s ease' }} />

                    {/* Info section */}
                    <div style={{
                      display: 'flex', flexDirection: 'column', alignItems: 'center',
                      gap: isExpanded ? 1 : 4,
                      padding: isExpanded ? '4px 8px 3px' : '0 10px',
                      transition: 'gap 0.35s ease, padding 0.35s ease',
                      flexShrink: 0,
                    }}>
                      {/* Aspect ratio preview — shrinks when expanded */}
                      <div style={{
                        width: pw,
                        height: isExpanded ? 0 : ph,
                        borderRadius: 3,
                        border: isExpanded ? '0px solid transparent' : `2px solid ${color}`,
                        background: `${color}15`,
                        transition: 'height 0.3s ease, border-width 0.3s ease, opacity 0.25s ease',
                        opacity: isExpanded ? 0 : 1,
                        overflow: 'hidden',
                      }} />
                      {/* Resolution */}
                      <div style={{
                        fontSize: isExpanded ? 11 : 13,
                        fontWeight: 700, color: 'var(--color-text-primary)', lineHeight: 1.2,
                        transition: 'font-size 0.3s ease',
                      }}>
                        {bucket.bucket_width}×{bucket.bucket_height}
                      </div>
                      {/* Count info */}
                      <div style={{
                        fontSize: isExpanded ? 8 : 10,
                        color: 'var(--color-text-tertiary)', textAlign: 'center', lineHeight: 1.3,
                        transition: 'font-size 0.3s ease',
                      }}>
                        {bucket.image_count} {t('bucketPreview.nImagesShort', { n: '' }).trim()} · count {bucket.total_count}
                      </div>
                      {/* Mini bar — hides when expanded */}
                      <div style={{
                        width: '65%',
                        height: isExpanded ? 0 : 3,
                        borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden',
                        transition: 'height 0.25s ease, opacity 0.2s ease',
                        opacity: isExpanded ? 0 : 1,
                      }}>
                        <div style={{ height: '100%', width: `${pct}%`, background: color, borderRadius: 2 }} />
                      </div>
                    </div>

                    {/* Bottom spacer — centers content when collapsed */}
                    <div style={{ flex: isExpanded ? 0 : 1, transition: 'flex 0.35s ease' }} />

                    {/* Image grid — fills remaining space when expanded */}
                    <div onClick={e => e.stopPropagation()} style={{
                      flex: isExpanded ? 1 : 0,
                      opacity: isExpanded ? 1 : 0,
                      transition: 'flex 0.35s ease, opacity 0.3s ease 0.1s',
                      overflow: 'hidden',
                      padding: isExpanded ? '4px 6px 6px' : '0 6px',
                      minHeight: 0,
                    }}>
                      <div style={{
                        display: 'grid',
                        gridTemplateColumns: 'repeat(3, 1fr)',
                        gridAutoRows: 'calc((100% - 8px) / 3)',
                        gap: 4,
                        height: '100%',
                        overflowY: 'auto',
                        overflowX: 'hidden',
                        alignContent: 'start',
                      }}>
                        {bucket.images.map((img, i) => (
                          <div key={i} style={{
                            borderRadius: 4, overflow: 'hidden',
                            border: '1px solid var(--color-border)',
                            background: '#0a0a0a',
                            display: 'flex', flexDirection: 'column',
                          }}>
                            <div style={{
                              flex: 1, overflow: 'hidden', minHeight: 0,
                              display: 'flex', alignItems: 'center', justifyContent: 'center',
                            }}>
                              <img src={convertFileSrc(img.path)} alt={img.name}
                                draggable={false}
                                onDragStart={e => e.preventDefault()}
                                style={{
                                  maxWidth: '100%', maxHeight: '100%', objectFit: 'contain',
                                  userSelect: 'none', pointerEvents: 'none',
                                }}
                                loading="lazy" />
                            </div>
                            <div style={{
                              padding: '2px 4px', background: 'var(--color-bg-secondary)',
                              fontSize: 7, fontWeight: 600, color: 'var(--color-text-secondary)',
                              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                              textAlign: 'center', flexShrink: 0,
                            }} title={img.name}>{img.name}</div>
                          </div>
                        ))}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Pagination */}
          {totalBucketPages > 1 && (
            <div style={{ flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10, padding: '6px 0' }}>
              <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 30 }}
                disabled={bucketPage === 0} onClick={() => setBucketPage(p => p - 1)}>
                <ChevronLeft style={{ width: 14, height: 14 }} />
              </button>
              <span style={{ fontSize: 12, color: 'var(--color-text-secondary)', fontWeight: 600, minWidth: 80, textAlign: 'center' }}>
                {bucketPage + 1} / {totalBucketPages}
              </span>
              <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 30 }}
                disabled={bucketPage >= totalBucketPages - 1} onClick={() => setBucketPage(p => p + 1)}>
                <ChevronRight style={{ width: 14, height: 14 }} />
              </button>
              <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>
                {t('bucketPreview.allBuckets', { n: analysis.buckets.length })}
              </span>
            </div>
          )}
              </>
            );
          })()}

          {/* Export */}
          <div style={{
            flexShrink: 0, marginTop: 'var(--space-3)', padding: '10px 16px',
            borderRadius: 'var(--radius-md)', border: '1px solid var(--color-border)',
            background: 'var(--color-bg-secondary)',
            display: 'flex', alignItems: 'center', gap: 12,
          }}>
            <div onClick={() => setEnableExport(!enableExport)} style={{
              width: 18, height: 18, borderRadius: 4, cursor: 'pointer',
              border: `2px solid ${enableExport ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
              background: enableExport ? 'var(--color-accent-primary)' : 'transparent',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              transition: 'all 0.2s', flexShrink: 0,
            }}>
              {enableExport && <svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 5L4 7L8 3" stroke="#fff" strokeWidth="2" fill="none" strokeLinecap="round" strokeLinejoin="round"/></svg>}
            </div>
            <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)', whiteSpace: 'nowrap' }}>{t('bucketPreview.exportResult')}</span>
            {enableExport && (
              <>
                <div style={{ flex: 1, display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('bucketPreview.exportPlaceholder')} value={exportPath} onChange={e => setExportPath(e.target.value)} style={{ flex: 1, height: 32, fontSize: 12 }} />
                  <button className="btn btn-secondary" onClick={selectExportFolder} style={{ height: 32 }}><FolderOpen style={{ width: 14, height: 14 }} /></button>
                </div>
                <button className="btn btn-primary" style={{ height: 32, padding: '0 16px', fontSize: 12, whiteSpace: 'nowrap' }} onClick={handleExport} disabled={exporting || !exportPath}>
                  {exporting ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('bucketPreview.exporting')}</> : <><Download style={{ width: 14, height: 14 }} /> {t('bucketPreview.export')}</>}
                </button>
              </>
            )}

          </div>
        </div>
      )}

      {/* Empty state */}
      {!analysis && !analyzing && (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, opacity: 0.5 }}>
          <ImageIcon style={{ width: 48, height: 48, color: 'var(--color-text-tertiary)' }} />
          <span style={{ fontSize: 14, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.emptyHint')}</span>
        </div>
      )}
    </div>
  );
}
