import { useId, useState, useEffect, useMemo, useRef, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import ThumbImage from '../components/ThumbImage';
import {
  Grid3X3,
  Play,
  Loader2,
  Download,
  ImageIcon,
  ChevronDown,
  Check,
  SlidersHorizontal,
  Sparkles,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { BUCKET_DEFAULTS, buildBucketOptions, type BucketMode } from '../api/commandOptions';
import RecursiveScanToggle from '../components/RecursiveScanToggle';
import { isCancelMessage } from '../components/TaskContext';
import PageHeader from '../components/ui/PageHeader';
import NumberInput from '../components/ui/NumberInput';
import PathInput from '../components/ui/PathInput';
import Switch from '../components/ui/Switch';
import Pager from '../components/ui/Pager';
import ExportBar from '../components/ExportBar';

type BucketEngine = 'sd' | 'diffusion_pipe';
type SdBucketMode = Exclude<BucketMode, 'diffusion_pipe'>;

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
  dropped_count: number;
  usable_rate: number;
  mean_ar_error: number;
}

interface BucketParamRecommendation {
  total_images: number;
  unique_sizes: number;
  min_bucket_reso: number;
  max_bucket_reso: number;
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

const fieldLabelStyle = (invalid = false): CSSProperties => ({ fontSize: 10, color: invalid ? '#ef4444' : undefined });

const fieldInputStyle = (invalid = false): CSSProperties => ({
  height: 32,
  borderColor: invalid ? '#ef4444' : undefined,
  boxShadow: invalid ? '0 0 0 1px #ef4444' : undefined,
});

export default function BucketPreviewPage() {
  const { t } = useTranslation();
  const d = BUCKET_DEFAULTS;
  const datasetId = useId();
  const [inputPath, setInputPath] = useState('');
  const [resolution, setResolution] = useState(`${d.res_width},${d.res_height}`);
  const [bucketRange, setBucketRange] = useState(`${d.min_bucket_reso},${d.max_bucket_reso}`);
  const [steps, setSteps] = useState(d.steps);
  const [noUpscale, setNoUpscale] = useState(d.no_upscale);
  const [bucketEngine, setBucketEngine] = useState<BucketEngine>('sd');
  const [bucketMode, setBucketMode] = useState<SdBucketMode>('legacy');
  const [modeMenuOpen, setModeMenuOpen] = useState(false);
  const [dpMinAr, setDpMinAr] = useState(d.dp_min_ar);
  const [dpMaxAr, setDpMaxAr] = useState(d.dp_max_ar);
  const [dpArBucketCount, setDpArBucketCount] = useState(d.dp_num_ar_buckets);
  const [batchSize, setBatchSize] = useState(d.batch_size);
  const [dropLast, setDropLast] = useState(d.drop_last);
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
  const resWidth = resPair?.[0] ?? d.res_width;
  const resHeight = resPair?.[1] ?? d.res_height;
  const minBucketReso = rangePair?.[0] ?? d.min_bucket_reso;
  const maxBucketReso = rangePair?.[1] ?? d.max_bucket_reso;

  // NumberInput 保证 steps ≥ 32 且为整数，只剩「大于 32 时须是 64 的倍数」要查
  const stepsError = steps > 32 && steps % 64 !== 0;
  const resError = !resPair;
  const dpArError = isDpMode && dpMaxAr <= dpMinAr;

  const [analyzing, setAnalyzing] = useState(false);
  const analyzeActive = useRef(false);
  /** 本轮分析 / 推荐点过取消：后端以「已取消」返回时按取消收尾，不报错 */
  const analyzeCancelRequested = useRef(false);
  const recommendCancelRequested = useRef(false);
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
  const exportActive = useRef(false);
  const [toast, setToast] = useState<{ msg: string; type: 'success' | 'error' } | null>(null);
  // toast 定时器：先 clear 再设，避免快速连续 toast 时旧定时器提前清掉新 toast
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 分析请求代际：输入/引擎切换或重新分析后，旧 invoke 的 resolve 一律作废
  const analyzeGenRef = useRef(0);

  const [expandedBuckets, setExpandedBuckets] = useState<Set<number>>(new Set());
  const [bucketPage, setBucketPage] = useState(0);
  const BUCKETS_PER_PAGE = 3;
  const RECOMMENDATIONS_PER_PAGE = 4;
  // 展开桶内图片的分批渲染数量
  const IMAGES_PER_BATCH = 60;
  const [bucketImgLimits, setBucketImgLimits] = useState<Record<number, number>>({});
  const bucketEngineOptions: { value: BucketEngine; label: string }[] = [
    { value: 'sd', label: t('bucketPreview.modeSd') },
    { value: 'diffusion_pipe', label: t('bucketPreview.modeDiffusionPipe') },
  ];
  const currentBucketEngineLabel = bucketEngineOptions.find(option => option.value === bucketEngine)?.label ?? t('bucketPreview.modeSd');

  const clearAnalysisResult = () => {
    setAnalysis(null);
    setScanMsg('');
    setScanProgress(0);
    setExpandedBuckets(new Set());
    setBucketPage(0);
    setBucketImgLimits({});
  };

  const showToast = (msg: string, type: 'success' | 'error') => {
    setToast({ msg, type });
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    toastTimerRef.current = setTimeout(() => setToast(null), 3000);
  };

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
    analyzeGenRef.current++; // 让在途分析的 resolve 作废，旧目录/旧引擎的结果不能回填
    setRecommendation(null);
    setRecommendPage(0);
    clearAnalysisResult();
  }, [inputPath, recursive, bucketEngine]);

  useEffect(() => {
    let active = true;
    const p1 = listen<ScanProgress>('bucket-progress', (e) => {
      if (!active || !analyzeActive.current) return;
      setScanMsg(e.payload.message);
      if (e.payload.total > 0) setScanProgress((e.payload.current / e.payload.total) * 100);
    });
    // 导出每复制 20 个文件报一次进度
    const p2 = listen<ScanProgress>('bucket-export-progress', (e) => {
      if (!active || !exportActive.current) return;
      showToast(e.payload.message, 'success');
    });
    return () => {
      active = false; p1.then(fn => fn()); p2.then(fn => fn());
      if (toastTimerRef.current) { clearTimeout(toastTimerRef.current); toastTimerRef.current = null; }
    };
  }, []);

  const handleAnalyze = async () => {
    if (!inputPath || analyzing || recommending || exporting) return;
    if (resError || stepsError || dpArError) return;
    const gen = ++analyzeGenRef.current;
    analyzeActive.current = true;
    analyzeCancelRequested.current = false;
    setAnalyzing(true);
    clearAnalysisResult();
    setScanMsg(t('bucketPreview.scanning'));
    try {
      const result = await invoke<BucketAnalysis>('analyze_buckets', {
        options: buildBucketOptions({ input_path: inputPath, recursive }, {
          res_width: resWidth,
          res_height: resHeight,
          steps,
          no_upscale: noUpscale,
          min_bucket_reso: minBucketReso,
          max_bucket_reso: maxBucketReso,
          bucket_mode: isDpMode ? 'diffusion_pipe' : bucketMode,
          dp_min_ar: dpMinAr,
          dp_max_ar: dpMaxAr,
          dp_num_ar_buckets: dpArBucketCount,
          batch_size: batchSize,
          drop_last: dropLast,
        }),
      });
      if (gen !== analyzeGenRef.current) return; // 输入/引擎已切换，丢弃过期结果
      setAnalysis(result);
    } catch (e) {
      const text = String(e);
      if (gen === analyzeGenRef.current && !(analyzeCancelRequested.current && isCancelMessage(text))) {
        showToast(`${t('pages.errorPrefix')}: ${text}`, 'error');
      }
    } finally {
      analyzeActive.current = false;
      setAnalyzing(false);
    }
  };

  const formatRecommendedAr = (value: number) => {
    const fixed = value.toFixed(3);
    return fixed.replace(/0+$/, '').replace(/\.$/, '');
  };

  const formatPercent = (value: number) => `${(value * 100).toFixed(1)}%`;

  const applyRecommendation = (candidate: BucketParamCandidate) => {
    setResolution(`${candidate.res_width},${candidate.res_height}`);
    setSteps(candidate.steps);
    setDpMinAr(Number(formatRecommendedAr(candidate.dp_min_ar)));
    setDpMaxAr(Number(formatRecommendedAr(candidate.dp_max_ar)));
    setDpArBucketCount(candidate.dp_num_ar_buckets);
    setBatchSize(candidate.batch_size);
    clearAnalysisResult();
  };

  // 取消进行中的分析/推荐：后端命中取消标志后以「已取消」错误返回，走各自的 catch 分支收尾
  const cancelAnalyze = () => {
    analyzeCancelRequested.current = true;
    invoke('cancel_bucket_analysis').catch(() => {});
  };

  const cancelRecommend = () => {
    recommendCancelRequested.current = true;
    invoke('cancel_bucket_recommend').catch(() => {});
  };

  const handleRecommend = async () => {
    if (!inputPath || recommending || analyzing || exporting) return;
    const gen = analyzeGenRef.current;
    recommendCancelRequested.current = false;
    setRecommending(true);
    try {
      const recommendation = await invoke<BucketParamRecommendation>('recommend_bucket_params', {
        options: {
          input_path: inputPath,
          recursive,
        },
      });
      if (gen !== analyzeGenRef.current) return;
      if (!recommendation.candidates.length) {
        showToast(t('bucketPreview.recommendFailed'), 'error');
        return;
      }
      setRecommendation(recommendation);
      setRecommendPage(0);
      applyRecommendation(recommendation.candidates[0]);
      setBucketRange(`${recommendation.min_bucket_reso},${recommendation.max_bucket_reso}`);
      showToast(t('bucketPreview.recommendApplied', {
        n: recommendation.total_images,
        sizes: recommendation.unique_sizes,
      }), 'success');
    } catch (e) {
      const text = String(e);
      if (!(recommendCancelRequested.current && isCancelMessage(text))) {
        showToast(`${t('bucketPreview.recommendFailed')}: ${text}`, 'error');
      }
    } finally {
      setRecommending(false);
    }
  };

  const handleExport = async () => {
    if (!analysis || !exportPath || exporting || analyzing || recommending) return;
    exportActive.current = true;
    setExporting(true);
    try {
      const msg = await invoke<string>('export_buckets', { analysis, outputPath: exportPath });
      showToast(msg, 'success');
    } catch (e) {
      showToast(`${t('bucketPreview.exportFailed')}: ${String(e)}`, 'error');
    } finally {
      exportActive.current = false;
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
      {/* 提示条 */}
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
      <PageHeader icon={Grid3X3} color="#f59e0b" title={t('bucketPreview.title')} subtitle={t('bucketPreview.subtitle')} style={{ flexShrink: 0 }} />

      {/* 参数 */}
      <div className="tool-panel" style={{ flexShrink: 0 }}>
        <div className="tool-panel-header" style={{ gap: 12 }}>
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
              <label className="form-label" htmlFor={datasetId}>{t('bucketPreview.datasetFolder')}</label>
              <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
            </div>
            <PathInput id={datasetId} value={inputPath} onChange={setInputPath}
              dialogTitle={t('bucketPreview.selectDataset')} placeholder={t('bucketPreview.datasetPlaceholder')} />
          </div>

          <div style={{ display: 'flex', gap: 8, alignItems: 'end', flexWrap: 'wrap' }}>
            {/* 训练分辨率 */}
            <div style={{ position: 'relative', flex: '1 1 120px', minWidth: 100 }}>
              <label className="form-label" style={fieldLabelStyle(resError)}>{t('bucketPreview.resolution')}</label>
              <input className="form-input" placeholder={t('bucketPreview.resolutionPlaceholder')} value={resolution} onChange={e => setResolution(e.target.value)} style={fieldInputStyle(resError)} />
            </div>

            {isDpMode && (
              <>
                <div style={{ position: 'relative', width: 92 }}>
                  <label className="form-label" style={fieldLabelStyle(dpArError)}>{t('bucketPreview.dpMinAr')}</label>
                  <NumberInput value={dpMinAr} onChange={setDpMinAr} min={0.01} step={0.01} fallback={d.dp_min_ar} style={fieldInputStyle(dpArError)} />
                </div>

                <div style={{ position: 'relative', width: 92 }}>
                  <label className="form-label" style={fieldLabelStyle(dpArError)}>{t('bucketPreview.dpMaxAr')}</label>
                  <NumberInput value={dpMaxAr} onChange={setDpMaxAr} min={0.01} step={0.01} fallback={d.dp_max_ar} style={fieldInputStyle(dpArError)} />
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
              <label className="form-label" style={fieldLabelStyle(stepsError)}>{t('bucketPreview.stepsLabel')}</label>
              <NumberInput value={steps} onChange={setSteps} min={32} step={32} integer fallback={d.steps} style={fieldInputStyle(stepsError)} />
              {stepsError && <div style={{
                position: 'absolute', top: '100%', left: 0, marginTop: 2,
                fontSize: 9, color: '#ef4444', whiteSpace: 'nowrap',
              }}>{t('bucketPreview.stepsError')}</div>}
            </div>

            <div style={{ position: 'relative', width: 82 }}>
              <label className="form-label" style={fieldLabelStyle()}>{t('bucketPreview.batchSize')}</label>
              <NumberInput value={batchSize} onChange={setBatchSize} min={1} step={1} integer fallback={d.batch_size} style={fieldInputStyle()} />
            </div>

            {isDpMode && (
              <label title={dropLast ? t('bucketPreview.dpDropLastTip') : t('bucketPreview.dpKeepShortBatchTip')} style={{
                display: 'flex', alignItems: 'center', gap: 8, height: 32,
                padding: '0 10px', borderRadius: 'var(--radius-md)',
                border: dropLast ? '1px solid rgba(248,113,113,0.55)' : '1px solid rgba(96,165,250,0.45)',
                background: dropLast ? 'rgba(248,113,113,0.08)' : 'rgba(96,165,250,0.08)',
                flexShrink: 0, cursor: 'pointer',
                transition: 'all 0.2s',
                color: 'inherit', font: 'inherit',
              }}>
                <span style={{ fontSize: 10, color: dropLast ? '#ef4444' : '#60a5fa', whiteSpace: 'nowrap', fontWeight: 700 }}>
                  {dropLast ? t('bucketPreview.dropLast') : t('bucketPreview.keepShortBatch')}
                </span>
                <Switch checked={dropLast} onChange={value => { setDropLast(value); clearAnalysisResult(); }} size="sm"
                  color="#ef4444" offColor="rgba(96,165,250,0.65)" aria-label={t('bucketPreview.dropLast')} />
              </label>
            )}

            {!isDpMode && (
              <div>
                <label className="form-label" style={{ fontSize: 10 }}>{t('bucketPreview.sdBucketMode')}</label>
                <div style={{ display: 'flex', gap: 4 }} role="group" aria-label={t('bucketPreview.sdBucketMode')}>
                  {(['legacy', 'nearest_only'] as const).map(value => (
                    <button key={value} type="button" aria-pressed={bucketMode === value} onClick={() => setBucketMode(value)} style={{
                      padding: '4px 10px', borderRadius: 'var(--radius-sm)',
                      border: `1px solid ${bucketMode === value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                      background: bucketMode === value ? 'rgba(124,92,252,0.08)' : 'transparent',
                      color: bucketMode === value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
                      fontSize: 11, fontWeight: 600, cursor: 'pointer',
                    }}>{t(value === 'legacy' ? 'bucketPreview.modeLegacy' : 'bucketPreview.modeNearest')}</button>
                  ))}
                </div>
              </div>
            )}

            {isDpMode && (
              <div style={{ position: 'relative', width: 84 }}>
                <label className="form-label" style={fieldLabelStyle()}>{t('bucketPreview.dpArBuckets')}</label>
                <NumberInput value={dpArBucketCount} onChange={setDpArBucketCount} min={1} step={1} integer fallback={d.dp_num_ar_buckets} style={fieldInputStyle()} />
              </div>
            )}

            {/* 桶不放大图片 */}
            {!isDpMode && <label style={{
              display: 'flex', alignItems: 'center', gap: 8, height: 32,
              padding: '0 10px', borderRadius: 'var(--radius-md)',
              border: `1px solid ${noUpscale ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
              background: noUpscale ? 'rgba(124,58,237,0.06)' : 'var(--color-bg-secondary)',
              cursor: 'pointer', flexShrink: 0,
              transition: 'all 0.2s',
            }}>
              <span style={{ fontSize: 10, color: 'var(--color-text-secondary)', whiteSpace: 'nowrap' }}>{t('bucketPreview.noUpscale')}</span>
              <Switch checked={noUpscale} onChange={setNoUpscale} size="sm" aria-label={t('bucketPreview.noUpscale')} />
            </label>}
          </div>

          <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
            {analyzing && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', whiteSpace: 'nowrap', flexShrink: 0 }}>{scanMsg}</span>}
            {analyzing && <div style={{ flex: 1, height: 4, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}>
              <div style={{ height: '100%', width: `${scanProgress}%`, background: 'var(--color-accent-primary)', borderRadius: 2, transition: 'width 0.3s' }} />
            </div>}
            {!analyzing && <div style={{ flex: 1 }} />}
            {isDpMode && (
              <button className="btn btn-secondary" style={{ height: 34, padding: '0 16px', flexShrink: 0, gap: 6 }} onClick={recommending ? cancelRecommend : handleRecommend} disabled={!recommending && (analyzing || !inputPath)}>
                {recommending ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('common.cancel')}</> : <><Sparkles style={{ width: 14, height: 14 }} /> {t('bucketPreview.recommendParams')}</>}
              </button>
            )}
            <button className="btn btn-primary" style={{ height: 34, padding: '0 20px', flexShrink: 0 }} onClick={analyzing ? cancelAnalyze : handleAnalyze} disabled={!analyzing && (recommending || !inputPath || resError || stepsError || dpArError)}>
              {analyzing ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('common.cancel')}</> : <><Play style={{ width: 14, height: 14 }} /> {t('bucketPreview.startPreview')}</>}
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
                    <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.recommendBatch', { n: recommendation.candidates[0].batch_size })}</span>
                    <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.activeBuckets', { n: recommendation.candidates[0].active_bucket_count })}</span>
                    <span style={{ fontSize: 11, color: recommendation.candidates[0].usable_rate >= 0.97 ? '#22c55e' : '#f59e0b', fontWeight: 700 }}>{t('bucketPreview.usableRate', { rate: formatPercent(recommendation.candidates[0].usable_rate) })}</span>
                    {totalRecommendPages > 1 && <Pager page={currentRecommendPage} pages={totalRecommendPages} onChange={setRecommendPage}
                      size="sm" style={{ marginLeft: 'auto' }}
                      prevTitle={t('bucketPreview.previousRecommendPage')} nextTitle={t('bucketPreview.nextRecommendPage')}
                      formatLabel={(current, total) => t('bucketPreview.recommendPage', { current, total })} />}
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

      {/* 分析结果 */}
      {analysis && (
        <div style={{ display: 'flex', flexDirection: 'column', overflow: 'visible', marginTop: 'var(--space-4)' }}>
          {/* 统计 */}
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
            <span style={{ fontSize: 11, fontWeight: 600, fontFamily: '"SF Mono","Fira Code",Menlo,monospace', color: analysis.mean_ar_error < 0.01 ? '#4ade80' : analysis.mean_ar_error < 0.05 ? '#fbbf24' : '#f87171' }} title={`Mean ${analysisIsDpMode ? 'log ' : ''}AR Error (without repeats): ${analysis.mean_ar_error}`}>
              {analysisIsDpMode ? 'Log AR Error' : 'AR Error'}: {analysis.mean_ar_error.toFixed(16)}
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
              <div style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fit, minmax(230px, 1fr))',
                gap: 8,
                maxHeight: 142,
                overflowY: 'auto',
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
                            <ThumbImage path={item.path} alt={item.name} style={{ width: '100%', height: '100%', objectFit: 'cover', display: 'block' }} />
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

          {/* 桶网格：固定 3 列，分页 */}
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
                    {/* 左上：序号 */}
                    <div style={{ position: 'absolute', top: 6, left: 8, fontSize: 9, fontWeight: 700, color: 'var(--color-text-tertiary)', zIndex: 1 }}>#{bucket.index}</div>
                    {/* 右上：方向标记 */}
                    <div style={{
                      position: 'absolute', top: 6, right: 8, zIndex: 1,
                      width: 20, height: 20, borderRadius: 5, background: color, opacity: 0.85,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      fontSize: 10, fontWeight: 800, color: '#fff',
                    }}>{orientLabel}</div>

                    {/* 上方留白：收起时让信息区垂直居中，展开时收拢 */}
                    <div style={{ flex: isExpanded ? 0 : 1, transition: 'flex 0.35s ease' }} />

                    {/* 信息区 */}
                    <div style={{
                      display: 'flex', flexDirection: 'column', alignItems: 'center',
                      gap: isExpanded ? 1 : 4,
                      padding: isExpanded ? '4px 8px 3px' : '0 10px',
                      transition: 'gap 0.35s ease, padding 0.35s ease',
                      flexShrink: 0,
                    }}>
                      {/* 宽高比示意，展开时收起 */}
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
                      {/* 分辨率 */}
                      <div style={{
                        fontSize: isExpanded ? 11 : 13,
                        fontWeight: 700, color: 'var(--color-text-primary)', lineHeight: 1.2,
                        transition: 'font-size 0.3s ease',
                      }}>
                        {bucket.bucket_width}×{bucket.bucket_height}
                      </div>
                      {/* 数量 */}
                      <div style={{
                        fontSize: isExpanded ? 8 : 10,
                        color: 'var(--color-text-tertiary)', textAlign: 'center', lineHeight: 1.3,
                        transition: 'font-size 0.3s ease',
                      }}>
                        {t('bucketPreview.nImagesShort', { n: bucket.image_count })} · count {bucket.total_count}
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
                        {analysisIsDpMode ? 'LogErr' : 'Err'} {bucket.mean_ar_error.toFixed(6)}
                      </div>
                      {/* 占比条，展开时隐藏 */}
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

                    {/* 下方留白：收起时让信息区垂直居中 */}
                    <div style={{ flex: isExpanded ? 0 : 1, transition: 'flex 0.35s ease' }} />

                    {/* 图片网格：展开时占满剩余空间 */}
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
                        // overflowX/Y 同时设置时 style 属性会写成 overflow 简写，global.css 的属性选择器匹配不到
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
                              <ThumbImage path={img.path} alt={img.name}
                                draggable={false}
                                style={{
                                  maxWidth: '100%', maxHeight: '100%', objectFit: 'contain',
                                  pointerEvents: 'none',
                                }} />
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

          {/* 翻页 */}
          {totalBucketPages > 1 && <Pager page={bucketPage} pages={totalBucketPages} onChange={setBucketPage} footer>
            <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.allBuckets', { n: analysis.buckets.length })}</span>
          </Pager>}
              </>
            );
          })()}

          <ExportBar enabled={enableExport} onChange={setEnableExport} label={t('bucketPreview.exportResult')}
            disabled={exporting} style={{ marginTop: 'var(--space-3)' }}>
            <PathInput kind="output" size="sm" value={exportPath} onChange={setExportPath} style={{ flex: 1 }}
              dialogTitle={t('bucketPreview.selectExport')} placeholder={t('bucketPreview.exportPlaceholder')} />
            <button className="btn btn-primary" style={{ height: 32, padding: '0 16px', fontSize: 12, whiteSpace: 'nowrap' }} onClick={handleExport} disabled={exporting || analyzing || recommending || !exportPath}>
              {exporting ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('bucketPreview.exporting')}</> : <><Download style={{ width: 14, height: 14 }} /> {t('bucketPreview.export')}</>}
            </button>
          </ExportBar>
        </div>
      )}

      {/* 空状态 */}
      {!analysis && !analyzing && (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, opacity: 0.5 }}>
          <ImageIcon style={{ width: 48, height: 48, color: 'var(--color-text-tertiary)' }} />
          <span style={{ fontSize: 14, color: 'var(--color-text-tertiary)' }}>{t('bucketPreview.emptyHint')}</span>
        </div>
      )}
    </div>
  );
}
