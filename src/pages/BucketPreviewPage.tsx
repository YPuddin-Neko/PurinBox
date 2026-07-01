import { useState, useEffect, useMemo, useRef, type WheelEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
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
  ChevronDown,
  Check,
  SlidersHorizontal,
  Sparkles,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import RecursiveScanToggle from '../components/RecursiveScanToggle';

type BucketEngine = 'sd' | 'diffusion_pipe';
type SdBucketMode = 'legacy' | 'nearest_only';

interface BucketImageInfo {
  path: string;
  name: string;
  orig_width: number;
  orig_height: number;
  repeats: number;
}

interface BucketGroup {
  index: number;
  bucket_width: number;
  bucket_height: number;
  image_count: number;
  total_count: number;
  effective_count: number;
  dropped_count: number;
  batch_count: number;
  short_batch_count: number;
  aspect_ratio: number;
  mean_ar_error: number;
  images: BucketImageInfo[];
}

interface BucketAnalysis {
  total_images: number;
  total_count: number;
  effective_count: number;
  dropped_count: number;
  batch_count: number;
  short_batch_count: number;
  usable_rate: number;
  batch_size: number;
  drop_last: boolean;
  bucket_count: number;
  skipped: [string, string][];
  buckets: BucketGroup[];
  mean_ar_error: number;
  ar_error_metric: 'linear' | 'log';
}

interface BucketParamCandidate {
  res_width: number;
  res_height: number;
  steps: number;
  dp_min_ar: number;
  dp_max_ar: number;
  dp_num_ar_buckets: number;
  batch_size: number;
  active_bucket_count: number;
  total_count: number;
  effective_count: number;
  dropped_count: number;
  usable_rate: number;
  mean_ar_error: number;
}

interface BucketParamRecommendation {
  total_images: number;
  skipped_count: number;
  unique_sizes: number;
  unique_aspect_ratios: number;
  res_width: number;
  res_height: number;
  steps: number;
  dp_min_ar: number;
  dp_max_ar: number;
  dp_num_ar_buckets: number;
  min_bucket_reso: number;
  max_bucket_reso: number;
  batch_size: number;
  active_bucket_count: number;
  total_count: number;
  effective_count: number;
  dropped_count: number;
  usable_rate: number;
  candidates: BucketParamCandidate[];
}

interface DroppedMaterialItem extends BucketImageInfo {
  dropped_repeats: number;
}

interface DroppedBucketPreview {
  bucket: BucketGroup;
  items: DroppedMaterialItem[];
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

function containWheelScroll(event: WheelEvent<HTMLElement>) {
  const element = event.currentTarget;
  const canScrollY = element.scrollHeight > element.clientHeight + 1;
  if (!canScrollY) return;

  const atTop = element.scrollTop <= 0;
  const atBottom = element.scrollTop + element.clientHeight >= element.scrollHeight - 1;
  const scrollingUp = event.deltaY < 0;
  const scrollingDown = event.deltaY > 0;

  event.stopPropagation();
  if ((scrollingUp && atTop) || (scrollingDown && atBottom)) {
    event.preventDefault();
  }
}

export default function BucketPreviewPage() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [resolution, setResolution] = useState('1024,1024');
  const [bucketRange, setBucketRange] = useState('256,2048');
  const [steps, setSteps] = useState(32);
  const [noUpscale, setNoUpscale] = useState(true);
  const [bucketEngine, setBucketEngine] = useState<BucketEngine>('sd');
  const [bucketMode, setBucketMode] = useState<SdBucketMode>('legacy');
  const [modeMenuOpen, setModeMenuOpen] = useState(false);
  const [dpMinArInput, setDpMinArInput] = useState('0.5');
  const [dpMaxArInput, setDpMaxArInput] = useState('2.0');
  const [dpArBucketCount, setDpArBucketCount] = useState(7);
  const [batchSize, setBatchSize] = useState(1);
  const [recursive, setRecursive] = useState(false);
  const modeMenuRef = useRef<HTMLDivElement | null>(null);

  // 解析分辨率
  const parsePair = (s: string): [number, number] | null => {
    const parts = s.split(/[,xX×]/).map(p => parseInt(p.trim()));
    if (parts.length === 2 && parts[0] > 0 && parts[1] > 0) return [parts[0], parts[1]];
    return null;
  };
  const resPair = parsePair(resolution);
  const rangePair = parsePair(bucketRange);
  const isDpMode = bucketEngine === 'diffusion_pipe';
  const resWidth = resPair?.[0] ?? 1024;
  const resHeight = resPair?.[1] ?? 1024;
  const minBucketReso = rangePair?.[0] ?? 256;
  const maxBucketReso = rangePair?.[1] ?? 2048;
  const dpMinAr = Number.parseFloat(dpMinArInput);
  const dpMaxAr = Number.parseFloat(dpMaxArInput);

  const stepsError = steps < 32 || (steps > 32 && steps % 64 !== 0);
  const resError = !resPair;
  const dpArError = isDpMode && (!Number.isFinite(dpMinAr) || !Number.isFinite(dpMaxAr) || dpMinAr <= 0 || dpMaxAr <= dpMinAr);
  const dpBucketError = isDpMode && (!Number.isFinite(dpArBucketCount) || dpArBucketCount < 1);
  const batchSizeError = !Number.isFinite(batchSize) || batchSize < 1;


  const [analyzing, setAnalyzing] = useState(false);
  const [analysis, setAnalysis] = useState<BucketAnalysis | null>(null);
  const analysisIsDpMode = analysis?.ar_error_metric === 'log';
  const droppedMaterialPreview = useMemo<DroppedBucketPreview[]>(() => {
    if (!analysis || !analysis.drop_last || analysis.dropped_count <= 0) return [];
    return analysis.buckets
      .filter(bucket => bucket.dropped_count > 0)
      .map(bucket => {
        let remaining = bucket.dropped_count;
        const items: DroppedMaterialItem[] = [];

        for (let i = bucket.images.length - 1; i >= 0 && remaining > 0; i -= 1) {
          const image = bucket.images[i];
          const droppedRepeats = Math.min(image.repeats, remaining);
          if (droppedRepeats > 0) {
            items.unshift({ ...image, dropped_repeats: droppedRepeats });
            remaining -= droppedRepeats;
          }
        }

        return { bucket, items };
      });
  }, [analysis]);
  const [scanMsg, setScanMsg] = useState('');
  const [scanProgress, setScanProgress] = useState(0);
  const [recommending, setRecommending] = useState(false);
  const [recommendation, setRecommendation] = useState<BucketParamRecommendation | null>(null);
  const [recommendPage, setRecommendPage] = useState(0);

  const [enableExport, setEnableExport] = useState(false);
  const [exportPath, setExportPath] = useState('');
  const [exporting, setExporting] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: 'success' | 'error' } | null>(null);
  // toast 定时器：先 clear 再设，避免快速连续 toast 时旧定时器提前清掉新 toast
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [expandedBuckets, setExpandedBuckets] = useState<Set<number>>(new Set());
  const [bucketPage, setBucketPage] = useState(0);
  const BUCKETS_PER_PAGE = 3;
  const RECOMMENDATIONS_PER_PAGE = 4;
  // 展开桶的图片分批渲染（每批 60 张）
  const IMAGES_PER_BATCH = 60;
  const [bucketImgLimits, setBucketImgLimits] = useState<Record<number, number>>({});
  const bucketEngineOptions: { value: BucketEngine; label: string }[] = [
    { value: 'sd', label: t('bucketPreview.modeSd') },
    { value: 'diffusion_pipe', label: t('bucketPreview.modeDiffusionPipe') },
  ];
  const currentBucketEngineLabel = bucketEngineOptions.find(option => option.value === bucketEngine)?.label ?? t('bucketPreview.modeSd');

  useEffect(() => {
    if (!modeMenuOpen) return;
    const handlePointerDown = (event: MouseEvent) => {
      if (!modeMenuRef.current?.contains(event.target as Node)) {
        setModeMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, [modeMenuOpen]);

  useEffect(() => {
    setRecommendation(null);
    setRecommendPage(0);
    setAnalysis(null);
    setScanMsg('');
    setScanProgress(0);
    setExpandedBuckets(new Set());
    setBucketPage(0);
    setBucketImgLimits({});
  }, [inputPath, recursive, bucketEngine]);

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
      if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
      toastTimerRef.current = setTimeout(() => setToast(null), 3000);
    });
    return () => {
      active = false; p1.then(fn => fn()); p2.then(fn => fn());
      if (toastTimerRef.current) { clearTimeout(toastTimerRef.current); toastTimerRef.current = null; }
    };
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
    if (resError || stepsError || dpArError || dpBucketError || batchSizeError) return;
    // 数字字段兜底：输入中途可能为 ""（空串），提交前规整为合法值
    const stepsVal = Number.isFinite(steps) && steps >= 32 ? steps : 32;
    const dpArBucketCountVal = Number.isFinite(dpArBucketCount) && dpArBucketCount >= 1 ? Math.floor(dpArBucketCount) : 7;
    const batchSizeVal = Number.isFinite(batchSize) && batchSize >= 1 ? Math.floor(batchSize) : 1;
    if (stepsVal !== steps) setSteps(stepsVal);
    if (isDpMode && dpArBucketCountVal !== dpArBucketCount) setDpArBucketCount(dpArBucketCountVal);
    if (batchSizeVal !== batchSize) setBatchSize(batchSizeVal);
    setAnalyzing(true);
    setAnalysis(null);
    setScanProgress(0);
    setScanMsg(t('bucketPreview.scanning'));
    setExpandedBuckets(new Set());
    setBucketPage(0);
    setBucketImgLimits({});
    try {
      const result = await invoke<BucketAnalysis>('analyze_buckets', {
        options: {
          input_path: inputPath,
          res_width: resWidth,
          res_height: resHeight,
          steps: stepsVal,
          no_upscale: isDpMode ? true : noUpscale,
          min_bucket_reso: noUpscale || isDpMode ? null : minBucketReso,
          max_bucket_reso: noUpscale || isDpMode ? null : maxBucketReso,
          bucket_mode: isDpMode ? 'diffusion_pipe' : bucketMode,
          recursive,
          dp_min_ar: isDpMode ? dpMinAr : null,
          dp_max_ar: isDpMode ? dpMaxAr : null,
          dp_num_ar_buckets: isDpMode ? dpArBucketCountVal : null,
          batch_size: batchSizeVal,
        },
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
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    toastTimerRef.current = setTimeout(() => setToast(null), 3000);
  };

  const formatRecommendedAr = (value: number) => {
    const fixed = value.toFixed(3);
    return fixed.replace(/0+$/, '').replace(/\.$/, '');
  };

  const formatPercent = (value: number) => `${(value * 100).toFixed(1)}%`;

  const clearAnalysisResult = () => {
    setAnalysis(null);
    setScanMsg('');
    setScanProgress(0);
    setExpandedBuckets(new Set());
    setBucketPage(0);
    setBucketImgLimits({});
  };

  const applyRecommendation = (candidate: BucketParamCandidate | BucketParamRecommendation) => {
    setResolution(`${candidate.res_width},${candidate.res_height}`);
    setSteps(candidate.steps);
    setDpMinArInput(formatRecommendedAr(candidate.dp_min_ar));
    setDpMaxArInput(formatRecommendedAr(candidate.dp_max_ar));
    setDpArBucketCount(candidate.dp_num_ar_buckets);
    setBatchSize(candidate.batch_size);
    clearAnalysisResult();
  };

  const handleRecommend = async () => {
    if (!inputPath) return;
    setRecommending(true);
    try {
      const recommendation = await invoke<BucketParamRecommendation>('recommend_bucket_params', {
        options: {
          input_path: inputPath,
          recursive,
        },
      });
      setRecommendation(recommendation);
      setRecommendPage(0);
      applyRecommendation(recommendation);
      setBucketRange(`${recommendation.min_bucket_reso},${recommendation.max_bucket_reso}`);
      showToast(t('bucketPreview.recommendApplied', {
        n: recommendation.total_images,
        sizes: recommendation.unique_sizes,
      }), 'success');
    } catch (e: any) {
      showToast(`${t('bucketPreview.recommendFailed')}: ${String(e)}`, 'error');
    } finally {
      setRecommending(false);
    }
  };

  const handleExport = async () => {
    if (!analysis || !exportPath) return;
    setExporting(true);
    try {
      const msg = await invoke<string>('export_buckets', { analysis, outputPath: exportPath });
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
    // 重新展开时重置该桶的图片渲染批次
    setBucketImgLimits(prev => {
      if (!(idx in prev)) return prev;
      const next = { ...prev };
      delete next[idx];
      return next;
    });
  };

  return (
    <div className="page" style={{ minHeight: '100%', display: 'flex', flexDirection: 'column', overflow: 'visible', position: 'relative', paddingBottom: 'var(--space-6)' }}>
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
        <div className="tool-panel-header" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <span className="tool-panel-title" style={{ minHeight: 30, display: 'flex', alignItems: 'center' }}>{t('bucketPreview.paramSettings')}</span>
          <div ref={modeMenuRef} style={{ position: 'relative', flexShrink: 0 }}>
            <button
              className="btn btn-secondary"
              title={t('bucketPreview.engineMode')}
              onClick={() => setModeMenuOpen(open => !open)}
              style={{
                height: 30,
                padding: '0 10px',
                gap: 6,
                fontSize: 11,
                fontWeight: 700,
                borderColor: modeMenuOpen ? 'var(--color-border-active)' : 'var(--color-border)',
                background: modeMenuOpen ? 'rgba(124,92,252,0.08)' : undefined,
                color: modeMenuOpen ? 'var(--color-accent-primary)' : undefined,
              }}
            >
              <SlidersHorizontal style={{ width: 14, height: 14 }} />
              <span style={{ whiteSpace: 'nowrap' }}>{currentBucketEngineLabel}</span>
              <ChevronDown style={{ width: 13, height: 13, transform: modeMenuOpen ? 'rotate(180deg)' : 'rotate(0deg)', transition: 'transform 0.15s ease' }} />
            </button>
            {modeMenuOpen && (
              <div style={{
                position: 'absolute',
                right: 0,
                top: 36,
                width: 190,
                padding: 6,
                borderRadius: 'var(--radius-md)',
                border: '1px solid var(--color-border)',
                background: 'var(--color-bg-card)',
                boxShadow: '0 10px 28px rgba(15,23,42,0.16)',
                zIndex: 20,
              }}>
                {bucketEngineOptions.map(option => {
                  const selected = bucketEngine === option.value;
                  return (
                    <button
                      key={option.value}
                      onClick={() => {
                        setBucketEngine(option.value);
                        setModeMenuOpen(false);
                      }}
                      style={{
                        width: '100%',
                        height: 30,
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        gap: 8,
                        padding: '0 8px',
                        borderRadius: 'var(--radius-sm)',
                        border: 'none',
                        background: selected ? 'rgba(124,92,252,0.10)' : 'transparent',
                        color: selected ? 'var(--color-accent-primary)' : 'var(--color-text-secondary)',
                        fontSize: 12,
                        fontWeight: selected ? 700 : 600,
                        cursor: 'pointer',
                      }}
                    >
                      <span>{option.label}</span>
                      {selected && <Check style={{ width: 13, height: 13 }} />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <div className="form-group">
            <div className="form-label-row">
              <label className="form-label" style={{ margin: 0 }}>{t('bucketPreview.datasetFolder')}</label>
              <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              <input className="form-input" placeholder={t('bucketPreview.datasetPlaceholder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
              <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
            </div>
          </div>

          <div style={{ display: 'flex', gap: 8, alignItems: 'end', flexWrap: 'wrap' }}>
            {/* 训练分辨率 */}
            <div style={{ position: 'relative', flex: '1 1 120px', minWidth: 100 }}>
              <label className="form-label" style={{ fontSize: 10, color: resError ? '#ef4444' : undefined }}>{t('bucketPreview.resolution')}</label>
              <input className="form-input" placeholder={t('bucketPreview.resolutionPlaceholder')} value={resolution} onChange={e => setResolution(e.target.value)} style={{
                height: 32,
                borderColor: resError ? '#ef4444' : undefined,
                boxShadow: resError ? '0 0 0 1px #ef4444' : undefined,
              }} />
            </div>

            {isDpMode && (
              <>
                <div style={{ position: 'relative', width: 92 }}>
                  <label className="form-label" style={{ fontSize: 10, color: dpArError ? '#ef4444' : undefined }}>{t('bucketPreview.dpMinAr')}</label>
                  <input className="form-input" type="number" step="0.01" min="0.01" value={dpMinArInput} onChange={e => setDpMinArInput(e.target.value)} style={{
                    height: 32,
                    borderColor: dpArError ? '#ef4444' : undefined,
                    boxShadow: dpArError ? '0 0 0 1px #ef4444' : undefined,
                  }} />
                </div>

                <div style={{ position: 'relative', width: 92 }}>
                  <label className="form-label" style={{ fontSize: 10, color: dpArError ? '#ef4444' : undefined }}>{t('bucketPreview.dpMaxAr')}</label>
                  <input className="form-input" type="number" step="0.01" min="0.01" value={dpMaxArInput} onChange={e => setDpMaxArInput(e.target.value)} style={{
                    height: 32,
                    borderColor: dpArError ? '#ef4444' : undefined,
                    boxShadow: dpArError ? '0 0 0 1px #ef4444' : undefined,
                  }} />
                </div>
              </>
            )}

            {/* 桶分辨率范围 (仅 no_upscale=false 时可用) */}
            {!isDpMode && <div style={{ flex: '1 1 120px', minWidth: 100, opacity: noUpscale ? 0.35 : 1, pointerEvents: noUpscale ? 'none' : 'auto', transition: 'opacity 0.2s' }}>
              <label className="form-label" style={{ fontSize: 10 }}>{t('bucketPreview.bucketRange')}</label>
              <input className="form-input" placeholder={t('bucketPreview.bucketRangePlaceholder')} value={bucketRange} onChange={e => setBucketRange(e.target.value)} style={{ height: 32 }} />
            </div>}

            {/* 桶分辨率划分单位 */}
            <div style={{ position: 'relative', width: 90 }}>
              <label className="form-label" style={{ fontSize: 10, color: stepsError ? '#ef4444' : undefined }}>{t('bucketPreview.stepsLabel')}</label>
              <input className="form-input" type="number" value={steps} onChange={e => setSteps(e.target.value === "" ? "" as any : Number(e.target.value))} onBlur={e => { if (e.target.value === "") setSteps(32); }} min={32} step={32} style={{
                height: 32,
                borderColor: stepsError ? '#ef4444' : undefined,
                boxShadow: stepsError ? '0 0 0 1px #ef4444' : undefined,
              }} />
              {stepsError && <div style={{
                position: 'absolute', top: '100%', left: 0, marginTop: 2,
                fontSize: 9, color: '#ef4444', whiteSpace: 'nowrap',
              }}>{t('bucketPreview.stepsError')}</div>}
            </div>

            <div style={{ position: 'relative', width: 82 }}>
              <label className="form-label" style={{ fontSize: 10, color: batchSizeError ? '#ef4444' : undefined }}>{t('bucketPreview.batchSize')}</label>
              <input className="form-input" type="number" value={batchSize} onChange={e => setBatchSize(e.target.value === "" ? "" as any : Number(e.target.value))} onBlur={e => { if (e.target.value === "") setBatchSize(1); }} min={1} step={1} style={{
                height: 32,
                borderColor: batchSizeError ? '#ef4444' : undefined,
                boxShadow: batchSizeError ? '0 0 0 1px #ef4444' : undefined,
              }} />
            </div>

            {isDpMode && (
              <div title={t('bucketPreview.dpDropLastTip')} style={{
                display: 'flex', alignItems: 'center', gap: 8, height: 32,
                padding: '0 10px', borderRadius: 'var(--radius-md)',
                border: '1px solid rgba(248,113,113,0.55)',
                background: 'rgba(248,113,113,0.08)',
                userSelect: 'none', flexShrink: 0,
                transition: 'all 0.2s',
              }}>
                <span style={{ fontSize: 10, color: '#ef4444', whiteSpace: 'nowrap', fontWeight: 700 }}>
                  {t('bucketPreview.dropLast')}
                </span>
                <div style={{
                  width: 30, height: 16, borderRadius: 8, transition: 'all 0.2s',
                  background: '#ef4444',
                  position: 'relative', flexShrink: 0,
                }}>
                  <div style={{ width: 12, height: 12, borderRadius: '50%', background: '#fff', position: 'absolute', top: 2, left: 16, transition: 'left 0.2s' }} />
                </div>
              </div>
            )}

            {!isDpMode && (
              <div>
                <label className="form-label" style={{ fontSize: 10 }}>{t('bucketPreview.sdBucketMode')}</label>
                <div style={{ display: 'flex', gap: 2, height: 32, alignItems: 'center' }}>
                  {([['legacy', t('bucketPreview.modeLegacy')], ['nearest_only', t('bucketPreview.modeNearest')]] as const).map(([val, label]) => (
                    <button key={val} onClick={() => setBucketMode(val)} style={{
                      padding: '4px 10px',
                      borderRadius: 'var(--radius-sm)',
                      border: `1px solid ${bucketMode === val ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                      background: bucketMode === val ? 'rgba(124,92,252,0.08)' : 'transparent',
                      color: bucketMode === val ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
                      fontSize: 11,
                      fontWeight: 600,
                      cursor: 'pointer',
                      whiteSpace: 'nowrap',
                      transition: 'all 0.15s',
                    }}>{label}</button>
                  ))}
                </div>
              </div>
            )}

            {isDpMode && (
              <div style={{ position: 'relative', width: 84 }}>
                <label className="form-label" style={{ fontSize: 10, color: dpBucketError ? '#ef4444' : undefined }}>{t('bucketPreview.dpArBuckets')}</label>
                <input className="form-input" type="number" value={dpArBucketCount} min={1} step={1} onChange={e => setDpArBucketCount(e.target.value === "" ? "" as any : Number(e.target.value))} onBlur={e => { if (e.target.value === "") setDpArBucketCount(7); }} style={{
                  height: 32,
                  borderColor: dpBucketError ? '#ef4444' : undefined,
                  boxShadow: dpBucketError ? '0 0 0 1px #ef4444' : undefined,
                }} />
              </div>
            )}

            {/* 桶不放大图片 */}
            {!isDpMode && <div onClick={() => setNoUpscale(!noUpscale)} style={{
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
            </div>}
          </div>

          <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
            {analyzing && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', whiteSpace: 'nowrap', flexShrink: 0 }}>{scanMsg}</span>}
            {analyzing && <div style={{ flex: 1, height: 4, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}>
              <div style={{ height: '100%', width: `${scanProgress}%`, background: 'var(--color-accent-primary)', borderRadius: 2, transition: 'width 0.3s' }} />
            </div>}
            {!analyzing && <div style={{ flex: 1 }} />}
            {isDpMode && (
              <button className="btn btn-secondary" style={{ height: 34, padding: '0 16px', flexShrink: 0, gap: 6 }} onClick={handleRecommend} disabled={recommending || analyzing || !inputPath}>
                {recommending ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('bucketPreview.recommending')}</> : <><Sparkles style={{ width: 14, height: 14 }} /> {t('bucketPreview.recommendParams')}</>}
              </button>
            )}
            <button className="btn btn-primary" style={{ height: 34, padding: '0 20px', flexShrink: 0 }} onClick={handleAnalyze} disabled={analyzing || !inputPath || resError || stepsError || dpArError || dpBucketError || batchSizeError}>
              {analyzing ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('bucketPreview.previewing')}</> : <><Play style={{ width: 14, height: 14 }} /> {t('bucketPreview.startPreview')}</>}
            </button>
          </div>

          {isDpMode && recommendation && recommendation.candidates.length > 0 && (
            (() => {
              const totalRecommendPages = Math.ceil(recommendation.candidates.length / RECOMMENDATIONS_PER_PAGE);
              const currentRecommendPage = Math.min(recommendPage, Math.max(0, totalRecommendPages - 1));
              const pageCandidates = recommendation.candidates.slice(
                currentRecommendPage * RECOMMENDATIONS_PER_PAGE,
                (currentRecommendPage + 1) * RECOMMENDATIONS_PER_PAGE,
              );

              return (
                <div style={{
                  borderTop: '1px solid var(--color-border)',
                  paddingTop: 10,
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 8,
                }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
                    <span style={{ fontSize: 12, fontWeight: 800, color: 'var(--color-text-primary)' }}>{t('bucketPreview.recommendCandidates')}</span>
                    <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.recommendBatch', { n: recommendation.batch_size })}</span>
                    <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.activeBuckets', { n: recommendation.active_bucket_count })}</span>
                    <span style={{ fontSize: 11, color: recommendation.usable_rate >= 0.97 ? '#22c55e' : '#f59e0b', fontWeight: 700 }}>{t('bucketPreview.usableRate', { rate: formatPercent(recommendation.usable_rate) })}</span>
                    {totalRecommendPages > 1 && (
                      <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 6 }}>
                        <button className="btn btn-ghost" style={{ width: 26, height: 24, padding: 0 }}
                          disabled={currentRecommendPage === 0}
                          onClick={() => setRecommendPage(page => Math.max(0, page - 1))}
                          title={t('bucketPreview.previousRecommendPage')}>
                          <ChevronLeft style={{ width: 13, height: 13 }} />
                        </button>
                        <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', minWidth: 42, textAlign: 'center' }}>
                          {t('bucketPreview.recommendPage', { current: currentRecommendPage + 1, total: totalRecommendPages })}
                        </span>
                        <button className="btn btn-ghost" style={{ width: 26, height: 24, padding: 0 }}
                          disabled={currentRecommendPage >= totalRecommendPages - 1}
                          onClick={() => setRecommendPage(page => Math.min(totalRecommendPages - 1, page + 1))}
                          title={t('bucketPreview.nextRecommendPage')}>
                          <ChevronRight style={{ width: 13, height: 13 }} />
                        </button>
                      </div>
                    )}
                  </div>
                  <div style={{
                    display: 'flex',
                    flexWrap: 'wrap',
                    gap: 6,
                  }}>
                    {pageCandidates.map((candidate, idx) => {
                      const candidateNumber = currentRecommendPage * RECOMMENDATIONS_PER_PAGE + idx + 1;
                      return (
                        <button
                          key={`${candidate.res_width}-${candidate.steps}-${candidate.dp_min_ar}-${candidate.dp_max_ar}-${candidate.dp_num_ar_buckets}-${candidate.batch_size}`}
                          onClick={() => applyRecommendation(candidate)}
                          style={{
                            minHeight: 44,
                            width: 270,
                            maxWidth: '100%',
                            flex: '0 0 270px',
                            padding: '7px 9px',
                            borderRadius: 'var(--radius-sm)',
                            border: '1px solid var(--color-border)',
                            background: candidate.batch_size === batchSize
                              && candidate.dp_num_ar_buckets === dpArBucketCount
                              && candidate.res_width === resWidth
                              && formatRecommendedAr(candidate.dp_min_ar) === formatRecommendedAr(dpMinAr)
                              && formatRecommendedAr(candidate.dp_max_ar) === formatRecommendedAr(dpMaxAr)
                              ? 'rgba(124,92,252,0.08)'
                              : 'var(--color-bg-input)',
                            color: 'var(--color-text-secondary)',
                            cursor: 'pointer',
                            textAlign: 'left',
                            display: 'flex',
                            flexDirection: 'column',
                            gap: 4,
                          }}
                        >
                          <span style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 11, fontWeight: 800, color: 'var(--color-text-primary)' }}>
                            <span>{t('bucketPreview.candidateLabel', { n: candidateNumber })} · BS {candidate.batch_size}</span>
                            <span style={{ color: candidate.usable_rate >= 0.97 ? '#22c55e' : '#f59e0b' }}>{formatPercent(candidate.usable_rate)}</span>
                          </span>
                          <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', lineHeight: 1.4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                            {candidate.res_width} · AR {formatRecommendedAr(candidate.dp_min_ar)}-{formatRecommendedAr(candidate.dp_max_ar)} · {candidate.dp_num_ar_buckets} / {candidate.active_bucket_count} · {t('bucketPreview.droppedCount', { n: candidate.dropped_count })}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              );
            })()
          )}
        </div>
      </div>

      {/* Results */}
      {analysis && (
        <div style={{ display: 'flex', flexDirection: 'column', overflow: 'visible', marginTop: 'var(--space-4)' }}>
          {/* Stats */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 16, flexShrink: 0, marginBottom: 'var(--space-3)' }}>
            <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: '#4ade80' }} />
              <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('bucketPreview.nBuckets', { n: analysis.bucket_count })}</span>
            </div>
            <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.nImages', { n: analysis.total_images })}</span>
            <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.totalCount', { n: analysis.total_count })}</span>
            {analysisIsDpMode && (
              <span style={{ fontSize: 12, color: analysis.dropped_count > 0 ? '#f59e0b' : '#22c55e', fontWeight: 700 }}>
                {t('bucketPreview.effectiveCount', { n: analysis.effective_count })} · {t('bucketPreview.usableRate', { rate: formatPercent(analysis.usable_rate) })}
              </span>
            )}
            <span style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.batchCount', { n: analysis.batch_count })}</span>
            {analysis.short_batch_count > 0 && <span style={{ fontSize: 11, color: '#60a5fa' }}>{t('bucketPreview.shortBatchCount', { n: analysis.short_batch_count })}</span>}
            {analysisIsDpMode && analysis.dropped_count > 0 && <span style={{ fontSize: 11, color: '#ef4444', fontWeight: 700 }}>{t('bucketPreview.droppedCount', { n: analysis.dropped_count })}</span>}
            {analysis.skipped.length > 0 && <span style={{ fontSize: 11, color: '#f87171' }}>{t('bucketPreview.readFail', { n: analysis.skipped.length })}</span>}
            <span style={{ fontSize: 11, fontWeight: 600, fontFamily: '"SF Mono","Fira Code",Menlo,monospace', color: analysis.mean_ar_error < 0.01 ? '#4ade80' : analysis.mean_ar_error < 0.05 ? '#fbbf24' : '#f87171' }} title={`Mean ${analysis.ar_error_metric === 'log' ? 'log ' : ''}AR Error (without repeats): ${analysis.mean_ar_error}`}>
              {analysis.ar_error_metric === 'log' ? 'Log AR Error' : 'AR Error'}: {analysis.mean_ar_error.toFixed(16)}
            </span>

          </div>

          {droppedMaterialPreview.length > 0 && (
            <div style={{
              flexShrink: 0,
              marginBottom: 'var(--space-3)',
              padding: '10px 12px',
              borderRadius: 'var(--radius-md)',
              border: '1px solid rgba(239,68,68,0.22)',
              background: 'rgba(239,68,68,0.045)',
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
            }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, flexWrap: 'wrap' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <ImageIcon style={{ width: 14, height: 14, color: '#ef4444' }} />
                  <span style={{ fontSize: 12, fontWeight: 800, color: 'var(--color-text-primary)' }}>
                    {t('bucketPreview.droppedMaterialsPreview')}
                  </span>
                  <span style={{ fontSize: 11, color: '#ef4444', fontWeight: 700 }}>
                    {t('bucketPreview.droppedCount', { n: analysis.dropped_count })}
                  </span>
                </div>
                <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>
                  {t('bucketPreview.droppedMaterialsHint')}
                </span>
              </div>
              <div onWheel={containWheelScroll} style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fit, minmax(230px, 1fr))',
                gap: 8,
                maxHeight: 142,
                overflowY: 'auto',
                overscrollBehavior: 'contain',
                paddingRight: 2,
              }}>
                {droppedMaterialPreview.map(({ bucket, items }) => (
                  <div key={bucket.index} style={{
                    borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--color-border)',
                    background: 'var(--color-bg-input)',
                    padding: 8,
                    minWidth: 0,
                  }}>
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, marginBottom: 6 }}>
                      <span style={{ fontSize: 11, fontWeight: 800, color: 'var(--color-text-primary)' }}>
                        #{bucket.index} · {bucket.bucket_width}×{bucket.bucket_height}
                      </span>
                      <span style={{ fontSize: 10, color: '#ef4444', fontWeight: 700 }}>
                        {t('bucketPreview.droppedShort', { n: bucket.dropped_count })}
                      </span>
                    </div>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
                      {items.map(item => (
                        <div key={`${bucket.index}-${item.path}`} style={{ display: 'flex', alignItems: 'center', gap: 7, minWidth: 0 }}>
                          <div style={{
                            width: 26,
                            height: 26,
                            borderRadius: 5,
                            overflow: 'hidden',
                            border: '1px solid var(--color-border)',
                            background: '#0a0a0a',
                            flexShrink: 0,
                          }}>
                            <img src={convertFileSrc(item.path)} alt={item.name} loading="lazy" style={{ width: '100%', height: '100%', objectFit: 'cover', display: 'block' }} />
                          </div>
                          <div style={{ minWidth: 0, flex: 1 }}>
                            <div title={item.name} style={{
                              fontSize: 10,
                              fontWeight: 700,
                              color: 'var(--color-text-secondary)',
                              whiteSpace: 'nowrap',
                              overflow: 'hidden',
                              textOverflow: 'ellipsis',
                            }}>
                              {item.name}
                            </div>
                            <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>
                              {t('bucketPreview.droppedFromRepeats', { drop: item.dropped_repeats, repeats: item.repeats })}
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Bucket grid — fixed 3 columns, paginated */}
          {(() => {
            const totalBucketPages = Math.ceil(analysis.buckets.length / BUCKETS_PER_PAGE);
            const pageBuckets = analysis.buckets.slice(bucketPage * BUCKETS_PER_PAGE, (bucketPage + 1) * BUCKETS_PER_PAGE);
            return (
              <>
          <div className="image-grid-perf" style={{ overflow: 'visible' }}>
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
                      <div style={{
                        fontSize: isExpanded ? 8 : 9,
                        color: analysisIsDpMode
                          ? (bucket.dropped_count > 0 ? '#f59e0b' : '#22c55e')
                          : (bucket.short_batch_count > 0 ? '#60a5fa' : 'var(--color-text-tertiary)'),
                        textAlign: 'center',
                        lineHeight: 1.25,
                        fontWeight: 700,
                        transition: 'font-size 0.3s ease',
                      }}>
                        {analysisIsDpMode ? (
                          <>
                            {t('bucketPreview.effectiveShort', { n: bucket.effective_count })}
                            {bucket.dropped_count > 0 ? ` · ${t('bucketPreview.droppedShort', { n: bucket.dropped_count })}` : ''}
                          </>
                        ) : (
                          <>
                            {t('bucketPreview.batchCount', { n: bucket.batch_count })}
                            {bucket.short_batch_count > 0 ? ` · ${t('bucketPreview.shortBatchShort', { n: bucket.short_batch_count })}` : ''}
                          </>
                        )}
                      </div>
                      <div style={{
                        fontSize: isExpanded ? 8 : 9,
                        fontWeight: 700,
                        color: bucket.mean_ar_error < 0.01 ? '#22c55e' : bucket.mean_ar_error < 0.05 ? '#f59e0b' : '#ef4444',
                        fontFamily: '"SF Mono","Fira Code",Menlo,monospace',
                        lineHeight: 1.25,
                        transition: 'font-size 0.3s ease',
                      }}>
                        {analysis.ar_error_metric === 'log' ? 'LogErr' : 'Err'} {bucket.mean_ar_error.toFixed(6)}
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
                      <div onWheel={containWheelScroll} style={{
                        display: 'grid',
                        gridTemplateColumns: 'repeat(3, 1fr)',
                        gridAutoRows: 'calc((100% - 8px) / 3)',
                        gap: 4,
                        height: '100%',
                        overflowY: 'auto',
                        overflowX: 'hidden',
                        overscrollBehavior: 'contain',
                        alignContent: 'start',
                      }}>
                        {(() => {
                          const imgLimit = bucketImgLimits[bucket.index] ?? IMAGES_PER_BATCH;
                          const visibleImages = bucket.images.slice(0, imgLimit);
                          const remaining = bucket.images.length - visibleImages.length;
                          return (
                            <>
                        {visibleImages.map((img, i) => (
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
                        {remaining > 0 && (
                          <button className="btn btn-ghost" style={{ gridColumn: '1 / -1', height: 26, fontSize: 10 }}
                            onClick={() => setBucketImgLimits(prev => ({ ...prev, [bucket.index]: imgLimit + IMAGES_PER_BATCH }))}>
                            {t('common.showMore', { n: remaining })}
                          </button>
                        )}
                            </>
                          );
                        })()}
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
