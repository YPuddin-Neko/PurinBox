import { useState, useEffect, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { BarChart3, Download, FolderInput } from 'lucide-react';
import type {
  AggregatePlanEntry, ResolutionAggregateOptions, ResolutionAnalyzeOptions, ResolutionAnalyzeResult, ResolutionGroup,
} from '../api/commandOptions';
import ProgressLog from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';
import CustomSelect from '../components/CustomSelect';
import ResolutionDonut from '../components/ResolutionDonut';
import { useBatchTask } from '../hooks/useBatchTask';
import NumberInput from '../components/ui/NumberInput';
import PathFields from '../components/ui/PathFields';
import PathInput from '../components/ui/PathInput';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import ExportBar from '../components/ExportBar';

/** 宽高比相近的分辨率聚成一组 */
interface ResolutionCluster {
  members: ResolutionGroup[];
  totalCount: number;
  /** 计算的推荐分辨率（组内按数量加权：面积取几何平均，宽高比取算术平均，对齐到所选倍数） */
  computed: { w: number; h: number };
}

const resKey = (w: number, h: number) => `${w}x${h}`;

const ALIGN_STEPS = [16, 32, 64] as const;
const ALIGN_STEP_KEY = 'resolution_align_step';

function loadAlignStep(): number {
  try {
    const v = Number(localStorage.getItem(ALIGN_STEP_KEY));
    if ((ALIGN_STEPS as readonly number[]).includes(v)) return v;
  } catch { /* 存储不可用时用默认值 */ }
  return 64;
}

function computeClusterMiddle(members: ResolutionGroup[], step: number): { w: number; h: number } {
  const total = members.reduce((s, m) => s + m.count, 0);
  const logArea = members.reduce((s, m) => s + m.count * Math.log(m.width * m.height), 0) / total;
  const area = Math.exp(logArea);
  const ar = members.reduce((s, m) => s + m.count * (m.width / m.height), 0) / total;
  const snap = (v: number) => Math.max(step, Math.round(v / step) * step);
  return { w: snap(Math.sqrt(area * ar)), h: snap(Math.sqrt(area / ar)) };
}

/** 按宽高比容差聚类（groups 需已按数量降序，数量多的分辨率作为组的种子） */
function buildClusters(groups: ResolutionGroup[], tolerancePct: number, step: number): ResolutionCluster[] {
  const tol = Math.max(0, tolerancePct) / 100;
  const raw: { members: ResolutionGroup[]; arWeightedSum: number; countSum: number }[] = [];
  for (const g of groups) {
    const ar = g.width / g.height;
    const hit = raw.find(c => {
      const rep = c.arWeightedSum / c.countSum;
      return Math.abs(ar - rep) / rep <= tol;
    });
    if (hit) {
      hit.members.push(g);
      hit.arWeightedSum += ar * g.count;
      hit.countSum += g.count;
    } else {
      raw.push({ members: [g], arWeightedSum: ar * g.count, countSum: g.count });
    }
  }
  return raw.map(c => ({
    members: c.members,
    totalCount: c.countSum,
    computed: computeClusterMiddle(c.members, step),
  }));
}

export default function ResolutionAnalyzePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'resolution-analyze-progress', taskId: 'resolution-analyze', logDone: false });
  const { logger } = task;
  const [inputPath, setInputPath] = useState('');
  const [rareThreshold, setRareThreshold] = useState(10);
  const [recursive, setRecursive] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [result, setResult] = useState<ResolutionAnalyzeResult | null>(null);
  const analyzeGeneration = useRef(0);
  /** 当前查看文件列表的稀有分辨率（"宽x高"，null = 未选） */
  const [selectedRare, setSelectedRare] = useState<string | null>(null);

  // ── 分辨率聚合 ──
  const [arTolerance, setArTolerance] = useState(5);
  const [alignStep, setAlignStep] = useState(loadAlignStep);
  const [aggExportPath, setAggExportPath] = useState('');
  const [aggExporting, setAggExporting] = useState(false);
  const [resultExporting, setResultExporting] = useState(false);
  const [enableAggExport, setEnableAggExport] = useState(false);
  /** 多成员组的目标分辨率选择（key = 组序号，value = "宽x高"） */
  const [clusterTargets, setClusterTargets] = useState<Record<number, string>>({});

  const clusters = useMemo(
    () => (result ? buildClusters(result.groups, arTolerance, alignStep) : []),
    [result, arTolerance, alignStep],
  );
  const multiClusters = useMemo(() => clusters.filter(c => c.members.length > 1), [clusters]);
  const rareGroups = useMemo(() => (result ? result.groups.filter(g => g.is_rare) : []), [result]);
  const singleClusters = useMemo(() => clusters.filter(c => c.members.length === 1), [clusters]);

  useEffect(() => {
    try { localStorage.setItem(ALIGN_STEP_KEY, String(alignStep)); } catch { /* 忽略 */ }
  }, [alignStep]);

  // 容差、对齐倍数或结果变化后组会重算，清空已选目标
  useEffect(() => {
    setClusterTargets({});
  }, [clusters]);

  // 输入目录/递归开关变化后，屏幕上的分析结果与当前设置不再对应；
  // 两个导出入口都按"点击时刻的 inputPath+recursive"重扫，必须让旧结果失效
  useEffect(() => {
    analyzeGeneration.current += 1;
    setResult(null);
    setSelectedRare(null);
  }, [inputPath, recursive]);

  const handleAnalyze = async () => {
    if (!inputPath || task.processing) return;
    const generation = analyzeGeneration.current;
    setProcessing(true); setResult(null); setSelectedRare(null);
    try {
      const res = await task.run({
        taskName: t('resolutionAnalyze.taskName'),
        startLog: t('resolutionAnalyze.analyzing'),
        exec: () => invoke<ResolutionAnalyzeResult>('analyze_resolutions', {
          options: { input_path: inputPath, rare_threshold: rareThreshold, recursive } satisfies ResolutionAnalyzeOptions,
        }),
      });
      if (res && generation === analyzeGeneration.current) {
        setResult(res);
        logger.appendLog(t('resolutionAnalyze.analyzeComplete', {
          total: res.total_images, distinct: res.distinct_count,
        }), res.failed_count > 0 ? 'warning' : 'success');
      }
    } finally { setProcessing(false); }
  };

  const cancelAggregateExport = () => {
    invoke('cancel_resolution_aggregate').catch(() => {});
  };

  const handleAggregateExport = async () => {
    if (!result || !inputPath || !aggExportPath || clusters.length === 0 || task.processing) return;
    setAggExporting(true);
    try {
      const plan: AggregatePlanEntry[] = [
        ...multiClusters.map((c, i) => ({
          folder: clusterTargets[i] ?? resKey(c.computed.w, c.computed.h),
          resolutions: c.members.map((m): [number, number] => [m.width, m.height]),
        })),
        ...singleClusters.map(c => ({
          folder: resKey(c.members[0].width, c.members[0].height),
          resolutions: [[c.members[0].width, c.members[0].height] as [number, number]],
        })),
      ];
      const msg = await task.run({
        taskName: t('resolutionAnalyze.aggregationTitle'),
        startLog: t('resolutionAnalyze.aggregationTitle'),
        keepLogs: true,
        exec: () => invoke<string>('export_resolution_aggregation', {
          options: { input_path: inputPath, recursive, output_path: aggExportPath, plan } satisfies ResolutionAggregateOptions,
        }),
      });
      if (msg) logger.appendLog(msg, 'success');
    } finally { setAggExporting(false); }
  };

  const handleResultExport = async () => {
    if (!result || !inputPath || result.groups.length === 0 || task.processing || resultExporting) return;
    setResultExporting(true);
    try {
      const outputPath = await open({ directory: true, multiple: false, title: t('resolutionAnalyze.selectResultExportFolder') });
      if (!outputPath) return;
      const msg = await task.run({
        taskName: t('resolutionAnalyze.export'),
        startLog: t('resolutionAnalyze.resultExportStart'),
        keepLogs: true,
        exec: () => invoke<string>('export_resolution_aggregation', {
          options: {
            input_path: inputPath, recursive, output_path: outputPath as string,
            plan: result.groups.map(group => ({
              folder: resKey(group.width, group.height), resolutions: [[group.width, group.height]],
            })),
          } satisfies ResolutionAggregateOptions,
        }),
      });
      if (msg) logger.appendLog(msg, 'success');
    } catch (error) {
      logger.appendCatchError(error, t('pages.errorPrefix'));
    } finally { setResultExporting(false); }
  };

  const resultPanel = result && (
    <div className="tool-panel">
      <div className="tool-panel-header">
        <span className="tool-panel-title">{t('resolutionAnalyze.analysisResults')}</span>
        <button
          className="btn btn-ghost btn-sm"
          type="button"
          onClick={handleResultExport}
          disabled={resultExporting || aggExporting}
          title={t('resolutionAnalyze.exportResultTip')}
          style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: 11 }}
        >
          <Download style={{ width: 13, height: 13 }} />
          {resultExporting ? t('resolutionAnalyze.exportingResult') : t('resolutionAnalyze.exportResult')}
        </button>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
        {/* 概览统计：窄栏内自动换行 */}
        <div style={{
          display: 'flex', flexWrap: 'wrap', rowGap: 4,
          padding: '4px 8px',
          borderRadius: 'var(--radius-sm)',
          border: '1px solid var(--color-border)',
          background: 'var(--color-bg-input)',
        }}>
          {[
            { label: t('resolutionAnalyze.totalImages'), value: result.total_images.toLocaleString() },
            { label: t('resolutionAnalyze.distinctResolutions'), value: String(result.distinct_count) },
            { label: t('resolutionAnalyze.sizeRange'), value: `${result.min_width}×${result.min_height} ~ ${result.max_width}×${result.max_height}` },
            { label: t('resolutionAnalyze.readErrors'), value: String(result.failed_count), danger: result.failed_count > 0 },
          ].map((s) => (
            <div key={s.label} style={{ display: 'flex', alignItems: 'baseline', gap: 5, padding: '0 8px' }}>
              <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{s.label}</span>
              <span style={{ fontSize: 12, fontWeight: 700, color: s.danger ? '#ef4444' : 'var(--color-text-primary)', whiteSpace: 'nowrap' }}>{s.value}</span>
            </div>
          ))}
        </div>

        <ResolutionDonut groups={result.groups} totalImages={result.total_images} />

        {rareGroups.length > 0 && (
          <div>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 6, marginBottom: 4 }}>
              <span style={{ fontSize: 11, fontWeight: 600, color: '#ef4444' }}>
                {t('resolutionAnalyze.rareTitle', { n: rareGroups.length })}
              </span>
            </div>
            <div style={{
              display: 'flex', flexWrap: 'wrap', gap: 4,
              maxHeight: 132, overflowY: 'auto',
            }}>
              {rareGroups.map((g) => {
                const key = resKey(g.width, g.height);
                const sel = selectedRare === key;
                return (
                  <button
                    key={key}
                    title={`${g.count} (${g.percent.toFixed(1)}%)${g.aspect_label ? ` · ${g.aspect_label}` : ''}`}
                    onClick={() => setSelectedRare(sel ? null : key)}
                    style={{
                      display: 'inline-flex', alignItems: 'center', gap: 4,
                      padding: '1px 7px', borderRadius: 4,
                      fontSize: 10, fontFamily: 'monospace', lineHeight: '17px',
                      border: `1px solid ${sel ? '#ef4444' : 'rgba(239, 68, 68, 0.45)'}`,
                      background: 'rgba(239, 68, 68, 0.06)',
                      color: 'var(--color-text-secondary)',
                      cursor: 'pointer',
                      boxShadow: sel ? '0 0 0 1px rgba(239, 68, 68, 0.35)' : 'none',
                    }}
                  >
                    {g.width}×{g.height}
                    <span style={{ fontWeight: 700, color: '#ef4444' }}>{g.count}</span>
                  </button>
                );
              })}
            </div>
            {selectedRare && (() => {
              const g = result.groups.find(x => resKey(x.width, x.height) === selectedRare);
              if (!g) return null;
              return (
                <div style={{
                  marginTop: 4, padding: '5px 7px', borderRadius: 4,
                  border: '1px solid rgba(239, 68, 68, 0.3)',
                  background: 'rgba(239, 68, 68, 0.04)',
                  fontSize: 10, color: 'var(--color-text-secondary)',
                  maxHeight: 110, overflowY: 'auto',
                  wordBreak: 'break-all',
                }}>
                  {g.files.map((f, i) => (
                    <div key={i} style={{ marginBottom: 2 }}>{f}</div>
                  ))}
                </div>
              );
            })()}
          </div>
        )}

        {result.failed_count > 0 && (
          <div style={{
            padding: '5px 7px', borderRadius: 4,
            border: '1px solid #ef4444',
            background: 'rgba(239, 68, 68, 0.05)',
            fontSize: 10, color: 'var(--color-text-secondary)',
            maxHeight: 100, overflowY: 'auto',
            wordBreak: 'break-all',
          }}>
            <div style={{ fontWeight: 600, color: '#ef4444', marginBottom: 2 }}>
              {t('resolutionAnalyze.failedFiles')} ({result.failed_count})
            </div>
            {result.failed_files.map((f, i) => (
              <div key={i} style={{ marginBottom: 2 }}>{f}</div>
            ))}
          </div>
        )}
      </div>
    </div>
  );

  return (
    <ToolPageLayout icon={BarChart3} color="#4ade80" title={t('resolutionAnalyze.title')} subtitle={t('resolutionAnalyze.subtitle')}
      gap="var(--space-5)"
      aside={<>
        <ProcessButton
          processing={processing}
          onStart={handleAnalyze}
          disabled={!inputPath || aggExporting || resultExporting}
          cancelCommand="cancel_resolution_analyze"
          startText={t('resolutionAnalyze.startAnalyze')}
          processingText={t('pages.processing')}
          onCancelLog={task.buttonProps.onCancelLog}
        />
        <ProgressLog {...task.progressLogProps} />
        {/* 分析结果放在日志下方，利用右栏剩余空间 */}
        {resultPanel}
      </>}>
      <PathFields input={inputPath} onInput={setInputPath} recursive={recursive} onRecursive={setRecursive} />

      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('resolutionAnalyze.analysisOptions')}</span>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 'var(--space-4)' }}>
          <div className="form-group">
            <label className="form-label">{t('resolutionAnalyze.rareThreshold')}</label>
            <NumberInput value={rareThreshold} onChange={setRareThreshold} min={1} integer fallback={10} />
            <p style={{ fontSize: 11, color: 'var(--color-text-tertiary)', margin: '4px 0 0' }}>
              {t('resolutionAnalyze.rareThresholdDesc')}
            </p>
          </div>
          <div className="form-group">
            <label className="form-label">{t('resolutionAnalyze.aggregateTolerance')}</label>
            <NumberInput value={arTolerance} onChange={setArTolerance} min={0} max={50} integer fallback={5} />
          </div>
          <div className="form-group">
            <label className="form-label">{t('resolutionAnalyze.alignStep')}</label>
            <CustomSelect
              value={String(alignStep)}
              options={ALIGN_STEPS.map(n => ({ value: String(n), label: t('resolutionAnalyze.alignStepOption', { n }) }))}
              onChange={(v) => setAlignStep(Number(v))}
            />
          </div>
        </div>
      </div>

      {/* 比例相近的分辨率归组，每组可选成员分辨率或计算推荐值 */}
      {result && clusters.length > 0 && (
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('resolutionAnalyze.aggregationTitle')}</span>
            <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>
              {multiClusters.length > 0
                ? t('resolutionAnalyze.groupCount', { n: multiClusters.length })
                : t('resolutionAnalyze.noSimilarGroups')}
            </span>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <p style={{ fontSize: 11, color: 'var(--color-text-tertiary)', margin: 0, lineHeight: 1.6 }}>
              {t('resolutionAnalyze.aggregationDesc')}
              {singleClusters.length > 0 && ` ${t('resolutionAnalyze.singlesNote', { n: singleClusters.length })}`}
            </p>

            {multiClusters.length > 0 && (
              <div style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fill, minmax(250px, 1fr))',
                gap: 8,
              }}>
                {multiClusters.map((c, i) => {
                  const computedKey = resKey(c.computed.w, c.computed.h);
                  const memberOptions = c.members.map(m => ({
                    value: resKey(m.width, m.height),
                    label: t('resolutionAnalyze.memberOption', { res: `${m.width}×${m.height}`, count: m.count }),
                  }));
                  const options = memberOptions.some(o => o.value === computedKey)
                    ? memberOptions
                    : [{ value: computedKey, label: t('resolutionAnalyze.recommendedComputed', { res: `${c.computed.w}×${c.computed.h}` }) }, ...memberOptions];
                  return (
                    <div key={`${computedKey}-${i}`} style={{
                      padding: '8px 10px',
                      borderRadius: 'var(--radius-sm)',
                      border: '1px solid var(--color-border)',
                      background: 'var(--color-bg-input)',
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 6,
                    }}>
                      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>
                          {t('resolutionAnalyze.groupLabel', { n: i + 1 })}
                        </span>
                        <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>
                          {t('resolutionAnalyze.totalInGroup', { count: c.totalCount })}
                        </span>
                      </div>
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                        {c.members.map(m => (
                          <span key={resKey(m.width, m.height)} style={{
                            fontSize: 10, padding: '1px 6px', borderRadius: 4,
                            background: 'var(--color-bg-secondary)',
                            border: '1px solid var(--color-border)',
                            color: 'var(--color-text-secondary)',
                            fontFamily: 'monospace',
                          }}>
                            {m.width}×{m.height} · {m.count}
                          </span>
                        ))}
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', flexShrink: 0 }}>
                          {t('resolutionAnalyze.targetResolution')}
                        </span>
                        <CustomSelect
                          compact
                          style={{ flex: 1, minWidth: 0 }}
                          value={clusterTargets[i] ?? computedKey}
                          options={options}
                          onChange={(v) => setClusterTargets(prev => ({ ...prev, [i]: v }))}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            <ExportBar enabled={enableAggExport} onChange={setEnableAggExport} label={t('resolutionAnalyze.exportAggregation')} disabled={aggExporting}>
              <PathInput kind="output" size="sm" value={aggExportPath} onChange={setAggExportPath} style={{ flex: 1 }} />
              {aggExporting ? (
                <button className="btn btn-secondary" style={{ height: 32, padding: '0 16px', fontSize: 12, whiteSpace: 'nowrap' }} onClick={cancelAggregateExport}>
                  {t('common.cancel')}
                </button>
              ) : (
                <button className="btn btn-primary" style={{ height: 32, padding: '0 16px', fontSize: 12, whiteSpace: 'nowrap' }} onClick={handleAggregateExport} disabled={!aggExportPath || processing || resultExporting}>
                  <FolderInput style={{ width: 14, height: 14 }} /> {t('resolutionAnalyze.export')}
                </button>
              )}
            </ExportBar>
          </div>
        </div>
      )}
    </ToolPageLayout>
  );
}
