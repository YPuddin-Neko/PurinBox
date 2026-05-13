import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
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

const ALGORITHMS_BASE: { value: Algorithm; label: string }[] = [
  { value: 'kmeans', label: 'K-Means' },
  { value: 'hdbscan', label: 'HDBSCAN' },
];

export default function ImageClusterPage() {
  const { t } = useTranslation();
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

  const selectInputFolder = async () => { const s = await open({ directory: true, multiple: false, title: t('pages.selectInputTitle') }); if (s) setInputPath(s as string); };
  const selectOutputFolder = async () => { const s = await open({ directory: true, multiple: false, title: t('pages.selectOutputTitle') }); if (s) setOutputPath(s as string); };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true); addTask('image-cluster', t('imageCluster.taskName'));
    setProgress(0); setProgressCurrent(0); setProgressTotal(0); setIsDone(false); setHasError(false);
    setProcessStartTime(Date.now());

    const algoLabel = ALGORITHMS_BASE.find(a => a.value === algorithm)?.label || algorithm;
    const featLabel = featureType;
    const paramStr = algorithm === 'kmeans' ? `${t('imageCluster.groupCount')}: ${nClusters}` : `${t('imageCluster.minClusterSize')}: ${minClusterSize}`;
    const deviceLabel = device === 'auto' ? t('imageCluster.gpuAuto') : 'CPU';
    setLogs([{ time: getTimeStr(), message: t('imageCluster.startMsg', { algo: algoLabel, feat: featLabel, param: paramStr, device: deviceLabel }), status: 'info' }]);

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
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
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
          <h1 className="page-title">{t('imageCluster.title')}</h1>
        </div>
        <p className="page-subtitle">{t('imageCluster.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧设置 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 文件夹 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('imageCluster.folderSelect')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group">
                <label className="form-label">{t('blurNoise.inputFolder')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectInputFolder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">{t('blurNoise.outputFolder')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('pages.selectOutputFolder')} value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          {/* 参数设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('imageCluster.paramSettings')}</span>
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
                <label className="form-label">{t('imageCluster.clusterAlgo')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  {ALGORITHMS_BASE.map(a => (
                    <button key={a.value} onClick={() => setAlgorithm(a.value)} style={algoBtnStyle(algorithm === a.value)}>
                      {a.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* 特征类型 */}
              <div className="form-group">
                <label className="form-label">{t('imageCluster.featureType')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  {([{ value: 'style' as FeatureType, label: t('imageCluster.styleLabel'), desc: t('imageCluster.styleDesc'), color: '#f472b6' },
                    { value: 'semantic' as FeatureType, label: t('imageCluster.semanticLabel'), desc: t('imageCluster.semanticDesc'), color: '#60a5fa' },
                    { value: 'fusion' as FeatureType, label: t('imageCluster.fusionLabel'), desc: t('imageCluster.fusionDesc'), color: '#a78bfa' }]).map(f => (
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
                  <label className="form-label" style={{ marginBottom: 'var(--space-2)' }}>{t('imageCluster.fusionWeight')}</label>
                  {([
                    { label: t('imageCluster.styleLabel'), value: wStyle, set: setWStyle, color: '#f472b6' },
                    { label: t('imageCluster.semanticLabel'), value: wSemantic, set: setWSemantic, color: '#60a5fa' },
                    { label: t('imageCluster.colorLabel'), value: wColor, set: setWColor, color: '#fbbf24' },
                  ] as const).map(w => (
                    <div key={w.label} style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 6 }}>
                      <span style={{ fontSize: 12, fontWeight: 600, color: w.color, width: 32, textAlign: 'right' }}>{w.label}</span>
                      <input type="range" min="0" max="1" step="0.1" value={w.value}
                        onChange={(e) => w.set(Number(e.target.value))}
                        style={{ flex: 1, accentColor: w.color }} />
                      <span style={{ fontSize: 12, fontWeight: 700, fontFamily: 'monospace', color: w.color, width: 28, textAlign: 'right' }}>{w.value.toFixed(1)}</span>
                    </div>
                  ))}
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>{t('imageCluster.weightTip')}</span>
                </div>
              )}

              {/* 分组参数 + 分布图主题 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 'var(--space-4)', alignItems: 'start' }}>
                <div className="form-group" style={{ marginBottom: 0, opacity: algorithm === 'kmeans' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
                  <label className="form-label">{t('imageCluster.groupCount')}</label>
                  <input className="form-input" type="number" min={2} value={nClusters}
                    disabled={algorithm !== 'kmeans'}
                    onChange={(e) => { const v = parseInt(e.target.value); if (v >= 2) setNClusters(v); }}
                    style={{ height: 36 }} />
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>{t('imageCluster.groupCountTip')}</span>
                </div>
                <div className="form-group" style={{ marginBottom: 0, opacity: algorithm === 'hdbscan' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
                  <label className="form-label">{t('imageCluster.minClusterSize')}</label>
                  <input className="form-input" type="number" min={2} value={minClusterSize}
                    disabled={algorithm !== 'hdbscan'}
                    onChange={(e) => { const v = parseInt(e.target.value); if (v >= 2) setMinClusterSize(v); }}
                    style={{ height: 36 }} />
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>{t('imageCluster.minClusterSizeTip')}</span>
                </div>
                <div className="form-group" style={{ marginBottom: 0, minWidth: 140 }}>
                  <label className="form-label">{t('imageCluster.mapTheme')}</label>
                  <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                    {([{ val: 'light' as const, label: t('imageCluster.mapLight'), icon: <Sun style={{ width: 13, height: 13 }} /> },
                      { val: 'dark' as const, label: t('imageCluster.mapDark'), icon: <Moon style={{ width: 13, height: 13 }} /> }]).map(th => (
                      <button key={th.val} onClick={() => setMapTheme(th.val)} style={{
                        flex: 1, height: 36, borderRadius: 'var(--radius-sm)', fontSize: 12, fontWeight: 600,
                        cursor: 'pointer', transition: 'all 0.15s', textAlign: 'center',
                        display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 4,
                        border: `1.5px solid ${mapTheme === th.val ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
                        background: mapTheme === th.val ? 'rgba(124,92,252,0.08)' : 'transparent',
                        color: mapTheme === th.val ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
                      }}>{th.icon} {th.label}</button>
                    ))}
                  </div>
                </div>
              </div>

              {/* 提示信息 */}
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(167, 139, 250, 0.06)', border: '1px solid rgba(167, 139, 250, 0.1)' }}>
                <Info style={{ width: 13, height: 13, color: '#a78bfa', marginTop: 2, minWidth: 13 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  {algorithm === 'kmeans'
                    ? t('imageCluster.kmeansTip')
                    : t('imageCluster.hdbscanTip')
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
            startText={t('imageCluster.startCluster')} processingText={t('imageCluster.clustering')}
            onCancelLog={addCancelLog} />
          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} externalStartTime={processStartTime} />
        </div>
      </div>
    </div>
  );
}
