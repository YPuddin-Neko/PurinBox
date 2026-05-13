import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import {
  Network,
  FolderOpen,
  Info,
  Cpu,
  Gpu,
  Sun,
  Moon,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

type Algorithm = 'kmeans' | 'hdbscan';
type FeatureType = 'style' | 'semantic' | 'fusion';

const ALGORITHMS: { value: Algorithm; label: string; desc: string }[] = [
  { value: 'kmeans', label: 'K-Means', desc: '指定分组数量' },
  { value: 'hdbscan', label: 'HDBSCAN', desc: '自动发现分组' },
];

const FEATURES: { value: FeatureType; label: string; desc: string; color: string }[] = [
  { value: 'style', label: '风格', desc: '纹理/色调/画风', color: '#f472b6' },
  { value: 'semantic', label: '语义', desc: '内容/物体/场景', color: '#60a5fa' },
  { value: 'fusion', label: '融合', desc: '自定义比例混合', color: '#a78bfa' },
];

export default function ImageClusterPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [algorithm, setAlgorithm] = useState<Algorithm>('kmeans');
  const [featureType, setFeatureType] = useState<FeatureType>('semantic');
  const [nClusters, setNClusters] = useState(8);
  const [minClusterSize, setMinClusterSize] = useState(5);
  const [device, setDevice] = useState<'auto' | 'cpu'>('auto');
  const [wStyle, setWStyle] = useState(0.5);
  const [wSemantic, setWSemantic] = useState(0.5);
  const [wColor, setWColor] = useState(0.0);
  const [mapTheme, setMapTheme] = useState<'light' | 'dark'>('light');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [processStartTime, setProcessStartTime] = useState(0);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('cluster-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) setProgress((d.current / d.total) * 100);
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs((prev) => [...prev, {
          time: getTimeStr(),
          message: d.message,
          status: d.status === 'done' ? 'info' : d.status as LogEntry['status'],
        }]);
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, []);

  const selectInputFolder = async () => { const s = await open({ directory: true, multiple: false, title: '选择输入文件夹' }); if (s) setInputPath(s as string); };
  const selectOutputFolder = async () => { const s = await open({ directory: true, multiple: false, title: '选择输出文件夹' }); if (s) setOutputPath(s as string); };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true); addTask('image-cluster', '图片聚类');
    setProgress(0); setProgressCurrent(0); setProgressTotal(0); setIsDone(false); setHasError(false);
    setProcessStartTime(Date.now());

    const algoLabel = ALGORITHMS.find(a => a.value === algorithm)?.label || algorithm;
    const featLabel = FEATURES.find(f => f.value === featureType)?.label || featureType;
    const paramStr = algorithm === 'kmeans' ? `分组数: ${nClusters}` : `最小簇: ${minClusterSize}`;
    const deviceLabel = device === 'auto' ? 'GPU (自动)' : 'CPU';
    setLogs([{ time: getTimeStr(), message: `开始聚类 | ${algoLabel} | ${featLabel} | ${paramStr} | ${deviceLabel}`, status: 'info' }]);

    try {
      await invoke<ProcessResult>('start_image_cluster', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          algorithm,
          feature_type: featureType,
          n_clusters: nClusters,
          min_cluster_size: minClusterSize,
          device,
          weight_style: wStyle,
          weight_semantic: wSemantic,
          weight_color: wColor,
          map_theme: mapTheme,
        },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); setProcessStartTime(0); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  const algoBtnStyle = (active: boolean): React.CSSProperties => ({
    flex: 1, padding: '10px 0', borderRadius: 'var(--radius-sm)', fontSize: 13, fontWeight: 700,
    cursor: 'pointer', transition: 'all 0.15s', textAlign: 'center',
    border: `1.5px solid ${active ? 'var(--color-border-active)' : 'var(--color-border)'}`,
    background: active ? 'rgba(124,92,252,0.08)' : 'transparent',
    color: active ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
  });

  const featBtnStyle = (active: boolean, color: string): React.CSSProperties => ({
    flex: 1, padding: '10px 0', borderRadius: 'var(--radius-sm)', fontSize: 12, fontWeight: 700,
    cursor: 'pointer', transition: 'all 0.15s', textAlign: 'center',
    display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2,
    border: `1.5px solid ${active ? color : 'var(--color-border)'}`,
    background: active ? `${color}10` : 'transparent',
    color: active ? color : 'var(--color-text-tertiary)',
  });

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Network style={{ width: 28, height: 28, color: '#a78bfa' }} />
          <h1 className="page-title">图片聚类</h1>
        </div>
        <p className="page-subtitle">基于视觉特征自动将相似图片分组</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧设置 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 文件夹 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">文件夹选择</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group">
                <label className="form-label">输入文件夹</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择包含图片的文件夹..." value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">输出文件夹</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="聚类结果将输出到子文件夹..." value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          {/* 参数设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">参数设置</span>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                {([{ value: 'cpu' as const, label: 'CPU', icon: <Cpu style={{ width: 13, height: 13 }} />, color: '#fbbf24' },
                  { value: 'auto' as const, label: 'GPU', icon: <Gpu style={{ width: 13, height: 13 }} />, color: '#4ade80' }] as const).map(d => (
                  <button key={d.value} onClick={() => setDevice(d.value)} style={{
                    padding: '4px 12px', borderRadius: 'var(--radius-sm)', fontSize: 12, fontWeight: 700,
                    cursor: 'pointer', transition: 'all 0.15s',
                    display: 'flex', alignItems: 'center', gap: 4,
                    border: `1.5px solid ${device === d.value ? d.color : 'var(--color-border)'}`,
                    background: device === d.value ? `${d.color}12` : 'transparent',
                    color: device === d.value ? d.color : 'var(--color-text-tertiary)',
                  }}>
                    {d.icon} {d.label}
                  </button>
                ))}
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {/* 聚类算法 */}
              <div className="form-group">
                <label className="form-label">聚类算法</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  {ALGORITHMS.map(a => (
                    <button key={a.value} onClick={() => setAlgorithm(a.value)} style={algoBtnStyle(algorithm === a.value)}>
                      {a.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* 特征类型 */}
              <div className="form-group">
                <label className="form-label">特征类型</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  {FEATURES.map(f => (
                    <button key={f.value} onClick={() => setFeatureType(f.value)} style={featBtnStyle(featureType === f.value, f.color)}>
                      <span>{f.label}</span>
                      <span style={{ fontSize: 9, opacity: 0.7, fontWeight: 400 }}>{f.desc}</span>
                    </button>
                  ))}
                </div>
              </div>

              {/* 融合权重滑块 */}
              {featureType === 'fusion' && (
                <div className="form-group" style={{ background: 'rgba(167,139,250,0.04)', border: '1px solid rgba(167,139,250,0.1)', borderRadius: 'var(--radius-sm)', padding: 'var(--space-3)' }}>
                  <label className="form-label" style={{ marginBottom: 'var(--space-2)' }}>融合权重</label>
                  {([
                    { label: '风格', value: wStyle, set: setWStyle, color: '#f472b6' },
                    { label: '语义', value: wSemantic, set: setWSemantic, color: '#60a5fa' },
                    { label: '颜色', value: wColor, set: setWColor, color: '#fbbf24' },
                  ] as const).map(w => (
                    <div key={w.label} style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 6 }}>
                      <span style={{ fontSize: 12, fontWeight: 600, color: w.color, width: 32, textAlign: 'right' }}>{w.label}</span>
                      <input type="range" min="0" max="1" step="0.1" value={w.value}
                        onChange={(e) => w.set(Number(e.target.value))}
                        style={{ flex: 1, accentColor: w.color }} />
                      <span style={{ fontSize: 12, fontWeight: 700, fontFamily: 'monospace', color: w.color, width: 28, textAlign: 'right' }}>{w.value.toFixed(1)}</span>
                    </div>
                  ))}
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>设为 0 则忽略该特征，权重越大影响越大</span>
                </div>
              )}

              {/* 分组参数 + 分布图主题 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 'var(--space-4)', alignItems: 'start' }}>
                <div className="form-group" style={{ marginBottom: 0, opacity: algorithm === 'kmeans' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
                  <label className="form-label">分组数量 (K)</label>
                  <input className="form-input" type="number" min={2} value={nClusters}
                    disabled={algorithm !== 'kmeans'}
                    onChange={(e) => { const v = parseInt(e.target.value); if (v >= 2) setNClusters(v); }}
                    style={{ height: 36 }} />
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>最小 2，超出自动限制</span>
                </div>
                <div className="form-group" style={{ marginBottom: 0, opacity: algorithm === 'hdbscan' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
                  <label className="form-label">最小簇大小</label>
                  <input className="form-input" type="number" min={2} value={minClusterSize}
                    disabled={algorithm !== 'hdbscan'}
                    onChange={(e) => { const v = parseInt(e.target.value); if (v >= 2) setMinClusterSize(v); }}
                    style={{ height: 36 }} />
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>越小分组越多越细</span>
                </div>
                <div className="form-group" style={{ marginBottom: 0, minWidth: 140 }}>
                  <label className="form-label">分布图主题</label>
                  <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                    {([{ val: 'light' as const, label: '浅色', icon: <Sun style={{ width: 13, height: 13 }} /> },
                      { val: 'dark' as const, label: '深色', icon: <Moon style={{ width: 13, height: 13 }} /> }]).map(t => (
                      <button key={t.val} onClick={() => setMapTheme(t.val)} style={{
                        flex: 1, height: 36, borderRadius: 'var(--radius-sm)', fontSize: 12, fontWeight: 600,
                        cursor: 'pointer', transition: 'all 0.15s', textAlign: 'center',
                        display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 4,
                        border: `1.5px solid ${mapTheme === t.val ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
                        background: mapTheme === t.val ? 'rgba(124,92,252,0.08)' : 'transparent',
                        color: mapTheme === t.val ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
                      }}>{t.icon} {t.label}</button>
                    ))}
                  </div>
                </div>
              </div>

              {/* 提示信息 */}
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(167, 139, 250, 0.06)', border: '1px solid rgba(167, 139, 250, 0.1)' }}>
                <Info style={{ width: 13, height: 13, color: '#a78bfa', marginTop: 2, minWidth: 13 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  {algorithm === 'kmeans'
                    ? '使用 K-Means 需要预先指定分组数量。建议先用较大的 K 值，再根据结果调整。'
                    : 'HDBSCAN 会自动发现分组数量，不需要预设。无法归入任何组的图片会放入 noise 文件夹。'
                  }
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_image_cluster" forceCancelCommand="force_cancel_image_cluster"
            startText="开始聚类" processingText="聚类中..."
            onCancelLog={addCancelLog} />
          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} externalStartTime={processStartTime} />
        </div>
      </div>
    </div>
  );
}
