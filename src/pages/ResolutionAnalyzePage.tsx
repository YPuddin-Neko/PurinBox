import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { BarChart3, Download, FolderOpen, FolderInput } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';
import RecursiveScanToggle from '../components/RecursiveScanToggle';
import CustomSelect from '../components/CustomSelect';
import ResolutionDonut from '../components/ResolutionDonut';
import { useTaskQueue } from '../components/TaskContext';

interface ResolutionGroup {
  width: number;
  height: number;
  count: number;
  percent: number;
  aspect_ratio: number;
  aspect_label: string;
  is_rare: boolean;
  files: string[];
}

interface AnalyzeResult {
  total_images: number;
  failed_count: number;
  failed_files: string[];
  distinct_count: number;
  groups: ResolutionGroup[];
  min_width: number;
  max_width: number;
  min_height: number;
  max_height: number;
}

/** 宽高比相近的分辨率聚成一组 */
interface ResolutionCluster {
  members: ResolutionGroup[];
  totalCount: number;
  /** 计算的推荐分辨率（组内按数量加权：面积取几何平均，宽高比取算术平均，对齐 8） */
  computed: { w: number; h: number };
}

const resKey = (w: number, h: number) => `${w}x${h}`;

function computeClusterMiddle(members: ResolutionGroup[]): { w: number; h: number } {
  const total = members.reduce((s, m) => s + m.count, 0);
  const logArea = members.reduce((s, m) => s + m.count * Math.log(m.width * m.height), 0) / total;
  const area = Math.exp(logArea);
  const ar = members.reduce((s, m) => s + m.count * (m.width / m.height), 0) / total;
  const snap = (v: number) => Math.max(64, Math.round(v / 8) * 8);
  return { w: snap(Math.sqrt(area * ar)), h: snap(Math.sqrt(area / ar)) };
}

/** 按宽高比容差聚类（groups 需已按数量降序，数量多的分辨率作为组的种子） */
function buildClusters(groups: ResolutionGroup[], tolerancePct: number): ResolutionCluster[] {
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
    computed: computeClusterMiddle(c.members),
  }));
}

export default function ResolutionAnalyzePage() {
  const { t } = useTranslation();
  const { addTask, updateTask } = useTaskQueue();
  const [inputPath, setInputPath] = useState('');
  const [rareThreshold, setRareThreshold] = useState(10);
  const [recursive, setRecursive] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [result, setResult] = useState<AnalyzeResult | null>(null);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  /** 当前查看文件列表的稀有分辨率（"宽x高"，null = 未选） */
  const [selectedRare, setSelectedRare] = useState<string | null>(null);

  // ── 分辨率聚合 ──
  const [arTolerance, setArTolerance] = useState(5);
  const [aggExportPath, setAggExportPath] = useState('');
  const [aggExporting, setAggExporting] = useState(false);
  const [resultExporting, setResultExporting] = useState(false);
  const [enableAggExport, setEnableAggExport] = useState(false);
  /** 多成员组的目标分辨率选择（key = 组序号，value = "宽x高"） */
  const [clusterTargets, setClusterTargets] = useState<Record<number, string>>({});

  const clusters = useMemo(
    () => (result ? buildClusters(result.groups, Number.isFinite(arTolerance) ? arTolerance : 5) : []),
    [result, arTolerance],
  );
  const multiClusters = useMemo(() => clusters.filter(c => c.members.length > 1), [clusters]);
  const singleClusters = useMemo(() => clusters.filter(c => c.members.length === 1), [clusters]);

  // 容差或结果变化后组会重算，清空已选目标
  useEffect(() => {
    setClusterTargets({});
  }, [clusters]);

  /** 组的默认目标：计算推荐值；若与某成员相同则直接用该成员 */
  const defaultClusterTarget = (c: ResolutionCluster) => resKey(c.computed.w, c.computed.h);

  useEffect(() => {
    let active = true;
    // 事件名与后端 emit 保持一致（连字符风格，见 resolution_analyze.rs）
    const p = listen('resolution-analyze-progress', (event: any) => {
      if (!active) return;
      const d = event.payload;
      if (d.status === 'processing') {
        setProgressCurrent(d.current ?? 0);
        setProgressTotal(d.total ?? 0);
        setProgress(d.total > 0 ? Math.round(((d.current ?? 0) / d.total) * 100) : 0);
      } else if (d.status === 'done') {
        setProcessing(false);
        setIsDone(true);
        updateTask('resolution-analyze', { status: 'done' });
      } else if (d.status === 'error') {
        setHasError(true);
        updateTask('resolution-analyze', { status: 'error', message: d.message });
      }
      if (d.status !== 'processing') {
        setLogs((prev) => [
          ...prev,
          {
            time: getTimeStr(),
            message: d.message,
            status: d.status === 'done' ? 'info' : (d.status as LogEntry['status']),
          },
        ]);
      }
    });
    return () => {
      active = false;
      p.then((unlisten) => unlisten());
    };
  }, [updateTask]);

  const selectInputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('pages.selectInputTitle') });
    if (selected) setInputPath(selected as string);
  };

  const handleAnalyze = async () => {
    if (!inputPath) return;
    // 数字字段兜底：输入中途可能为 ""（空串），提交前规整为合法值
    const threshold = Number.isFinite(rareThreshold) && rareThreshold >= 1 ? Math.floor(rareThreshold) : 10;
    if (threshold !== rareThreshold) setRareThreshold(threshold);
    setProcessing(true);
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setResult(null);
    setIsDone(false);
    setHasError(false);
    setSelectedRare(null);

    addTask('resolution-analyze', t('resolutionAnalyze.taskName'));
    setLogs([{
      time: getTimeStr(),
      message: `${t('pages.startPrefix')}${t('resolutionAnalyze.analyzing')}...`,
      status: 'info',
    }]);

    try {
      const res = await invoke<AnalyzeResult>('analyze_resolutions', {
        options: {
          input_path: inputPath,
          rare_threshold: threshold,
          recursive,
        },
      });
      setResult(res);
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: t('resolutionAnalyze.analyzeComplete', {
            total: res.total_images,
            distinct: res.distinct_count,
          }),
          status: 'success',
        },
      ]);
    } catch (e: any) {
      setLogs((prev) => [
        ...prev,
        {
          time: getTimeStr(),
          message: typeof e === 'string' ? e : e?.message || t('resolutionAnalyze.analyzeError'),
          status: 'error',
        },
      ]);
      setHasError(true);
      setIsDone(true);
      const errStr = String(e);
      updateTask('resolution-analyze', {
        status: /已取消|cancel/i.test(errStr) ? 'cancelled' : 'error',
        message: errStr,
      });
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = () => {
    setLogs([]);
    setIsDone(false);
    setHasError(false);
  };

  const selectAggExportFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('pages.selectOutputFolder') });
    if (selected) setAggExportPath(selected as string);
  };

  const cancelAggregateExport = () => {
    invoke('cancel_resolution_aggregate').catch(() => {});
  };

  const handleAggregateExport = async () => {
    if (!result || !inputPath || !aggExportPath || clusters.length === 0 || resultExporting) return;
    setAggExporting(true);
    setLogs((prev) => [...prev, {
      time: getTimeStr(),
      message: `${t('pages.startPrefix')}${t('resolutionAnalyze.aggregationTitle')}...`,
      status: 'info',
    }]);
    try {
      // 多成员组用所选目标；独立分辨率导出到各自同名文件夹
      const plan = [
        ...multiClusters.map((c, i) => ({
          folder: clusterTargets[i] ?? defaultClusterTarget(c),
          resolutions: c.members.map(m => [m.width, m.height]),
        })),
        ...singleClusters.map(c => ({
          folder: resKey(c.members[0].width, c.members[0].height),
          resolutions: [[c.members[0].width, c.members[0].height]],
        })),
      ];
      const msg = await invoke<string>('export_resolution_aggregation', {
        options: {
          input_path: inputPath,
          recursive,
          output_path: aggExportPath,
          plan,
        },
      });
      setLogs((prev) => [...prev, { time: getTimeStr(), message: msg, status: 'success' }]);
    } catch (e: any) {
      const errStr = typeof e === 'string' ? e : e?.message || String(e);
      setLogs((prev) => [...prev, {
        time: getTimeStr(),
        message: errStr,
        status: /已取消|cancel/i.test(errStr) ? 'warning' : 'error',
      }]);
    } finally {
      setAggExporting(false);
    }
  };

  const handleResultExport = async () => {
    if (!result || !inputPath || result.groups.length === 0 || aggExporting) return;
    const outputPath = await open({
      directory: true,
      multiple: false,
      title: t('resolutionAnalyze.selectResultExportFolder'),
    });
    if (!outputPath) return;

    setResultExporting(true);
    setLogs((prev) => [...prev, {
      time: getTimeStr(),
      message: t('resolutionAnalyze.resultExportStart'),
      status: 'info',
    }]);
    try {
      const msg = await invoke<string>('export_resolution_aggregation', {
        options: {
          input_path: inputPath,
          recursive,
          output_path: outputPath as string,
          plan: result.groups.map((group) => ({
            folder: resKey(group.width, group.height),
            resolutions: [[group.width, group.height]],
          })),
        },
      });
      setLogs((prev) => [...prev, { time: getTimeStr(), message: msg, status: 'success' }]);
    } catch (e: any) {
      const errStr = typeof e === 'string' ? e : e?.message || String(e);
      setLogs((prev) => [...prev, {
        time: getTimeStr(),
        message: errStr,
        status: /已取消|cancel/i.test(errStr) ? 'warning' : 'error',
      }]);
    } finally {
      setResultExporting(false);
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <BarChart3 style={{ width: 28, height: 28, color: '#4ade80' }} />
          <h1 className="page-title">{t('resolutionAnalyze.title')}</h1>
        </div>
        <p className="page-subtitle">{t('resolutionAnalyze.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-5)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('pages.pathSettings')}</span>
            </div>
            <div className="tool-panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <div className="form-label-row">
                  <label className="form-label">{t('pages.inputPathShort')}</label>
                  <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input
                    className="form-input"
                    placeholder={t('pages.selectInputFolder')}
                    value={inputPath}
                    onChange={(e) => setInputPath(e.target.value)}
                    style={{ flex: 1 }}
                  />
                  <button className="btn btn-secondary" onClick={selectInputFolder}>
                    <FolderOpen style={{ width: 16, height: 16 }} />
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('resolutionAnalyze.analysisOptions')}</span>
            </div>
            <div className="tool-panel-body">
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)' }}>
                <div className="form-group" style={{ marginBottom: 0 }}>
                  <label className="form-label">{t('resolutionAnalyze.rareThreshold')}</label>
                  <input
                    className="form-input"
                    type="number"
                    value={rareThreshold}
                    onChange={(e) => setRareThreshold(Math.max(1, parseInt(e.target.value) || 1))}
                    onBlur={(e) => { if (e.target.value === "") setRareThreshold(10); }}
                    min={1}
                  />
                  <p style={{ fontSize: 11, color: 'var(--color-text-tertiary)', margin: '4px 0 0' }}>
                    {t('resolutionAnalyze.rareThresholdDesc')}
                  </p>
                </div>
                <div className="form-group" style={{ marginBottom: 0 }}>
                  <label className="form-label">{t('resolutionAnalyze.aggregateTolerance')}</label>
                  <input
                    className="form-input"
                    type="number"
                    value={arTolerance}
                    onChange={(e) => setArTolerance(Math.min(50, Math.max(0, parseInt(e.target.value) || 0)))}
                    onBlur={(e) => { if (e.target.value === "") setArTolerance(5); }}
                    min={0}
                    max={50}
                  />
                  <p style={{ fontSize: 11, color: 'var(--color-text-tertiary)', margin: '4px 0 0' }}>
                    {t('resolutionAnalyze.aggregateToleranceDesc')}
                  </p>
                </div>
              </div>
            </div>
          </div>

          {/* 分辨率聚合：比例相近的分辨率归组，每组可选成员分辨率或计算推荐值；导出栏样式与分桶预览一致 */}
          {result && clusters.length > 0 && (
            <div className="tool-panel">
              <div className="tool-panel-header" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <span className="tool-panel-title">{t('resolutionAnalyze.aggregationTitle')}</span>
                <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)' }}>
                  {multiClusters.length > 0
                    ? t('resolutionAnalyze.groupCount', { n: multiClusters.length })
                    : t('resolutionAnalyze.noSimilarGroups')}
                </span>
              </div>
              <div className="tool-panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
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
                              value={clusterTargets[i] ?? defaultClusterTarget(c)}
                              options={options}
                              onChange={(v) => setClusterTargets(prev => ({ ...prev, [i]: v }))}
                            />
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}

                {/* 导出栏：与分桶预览的导出栏同款 */}
                <div style={{
                  flexShrink: 0, padding: '10px 16px',
                  borderRadius: 'var(--radius-md)', border: '1px solid var(--color-border)',
                  background: 'var(--color-bg-secondary)',
                  display: 'flex', alignItems: 'center', gap: 12,
                }}>
                  <div onClick={() => setEnableAggExport(!enableAggExport)} style={{
                    width: 18, height: 18, borderRadius: 4, cursor: 'pointer',
                    border: `2px solid ${enableAggExport ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
                    background: enableAggExport ? 'var(--color-accent-primary)' : 'transparent',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    transition: 'all 0.2s', flexShrink: 0,
                  }}>
                    {enableAggExport && <svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 5L4 7L8 3" stroke="#fff" strokeWidth="2" fill="none" strokeLinecap="round" strokeLinejoin="round"/></svg>}
                  </div>
                  <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)', whiteSpace: 'nowrap' }}>{t('resolutionAnalyze.exportAggregation')}</span>
                  {enableAggExport && (
                    <>
                      <div style={{ flex: 1, display: 'flex', gap: 'var(--space-2)' }}>
                        <input className="form-input" placeholder={t('pages.selectOutputFolder')} value={aggExportPath} onChange={e => setAggExportPath(e.target.value)} style={{ flex: 1, height: 32, fontSize: 12 }} />
                        <button className="btn btn-secondary" onClick={selectAggExportFolder} style={{ height: 32 }}><FolderOpen style={{ width: 14, height: 14 }} /></button>
                      </div>
                      {aggExporting ? (
                        <button className="btn btn-secondary" style={{ height: 32, padding: '0 16px', fontSize: 12, whiteSpace: 'nowrap' }} onClick={cancelAggregateExport}>
                          {t('common.cancel')}
                        </button>
                      ) : (
                        <button className="btn btn-primary" style={{ height: 32, padding: '0 16px', fontSize: 12, whiteSpace: 'nowrap' }} onClick={handleAggregateExport} disabled={!aggExportPath || processing || resultExporting}>
                          <FolderInput style={{ width: 14, height: 14 }} /> {t('resolutionAnalyze.export')}
                        </button>
                      )}
                    </>
                  )}
                </div>
              </div>
            </div>
          )}

        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton
            processing={processing}
            onStart={handleAnalyze}
            disabled={!inputPath}
            cancelCommand="cancel_resolution_analyze"
            startText={t('resolutionAnalyze.startAnalyze')}
            processingText={t('pages.processing')}
            onCancelLog={(msg) => setLogs((prev) => [...prev, { time: getTimeStr(), message: msg, status: 'warning' }])}
          />

          <ProgressLog
            progress={progress}
            current={progressCurrent}
            total={progressTotal}
            logs={logs}
            isDone={isDone}
            hasError={hasError}
            onClearLogs={clearLogs}
          />

          {/* 分析结果：统计条 + 分布环形图 + 稀有分辨率（利用日志下方空间） */}
          {result && (
            <div className="tool-panel">
              <div className="tool-panel-header" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
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
              <div className="tool-panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
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

                {/* 稀有分辨率：点击查看文件 */}
                {result.groups.some(g => g.is_rare) && (
                  <div>
                    <div style={{ display: 'flex', alignItems: 'baseline', gap: 6, marginBottom: 4 }}>
                      <span style={{ fontSize: 11, fontWeight: 600, color: '#ef4444' }}>
                        {t('resolutionAnalyze.rareTitle', { n: result.groups.filter(g => g.is_rare).length })}
                      </span>
                      <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>
                        {t('resolutionAnalyze.clickRareHint')}
                      </span>
                    </div>
                    <div style={{
                      display: 'flex', flexWrap: 'wrap', gap: 4,
                      maxHeight: 132, overflowY: 'auto', overscrollBehavior: 'contain',
                    }}>
                      {result.groups.filter(g => g.is_rare).map((g) => {
                        const key = `${g.width}x${g.height}`;
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
                      const g = result.groups.find(x => `${x.width}x${x.height}` === selectedRare);
                      if (!g) return null;
                      return (
                        <div style={{
                          marginTop: 4, padding: '5px 7px', borderRadius: 4,
                          border: '1px solid rgba(239, 68, 68, 0.3)',
                          background: 'rgba(239, 68, 68, 0.04)',
                          fontSize: 10, color: 'var(--color-text-secondary)',
                          maxHeight: 110, overflowY: 'auto', overscrollBehavior: 'contain',
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

                {/* 读取失败文件 */}
                {result.failed_count > 0 && (
                  <div style={{
                    padding: '5px 7px', borderRadius: 4,
                    border: '1px solid #ef4444',
                    background: 'rgba(239, 68, 68, 0.05)',
                    fontSize: 10, color: 'var(--color-text-secondary)',
                    maxHeight: 100, overflowY: 'auto', overscrollBehavior: 'contain',
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
          )}
        </div>
      </div>
    </div>
  );
}
