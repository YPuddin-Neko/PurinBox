import { invoke } from '@tauri-apps/api/core';
import {
  Info,
  Moon,
  Network,
  Sun
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

type Algorithm = 'kmeans' | 'hdbscan';
type FeatureType = 'style' | 'semantic' | 'fusion';

const ALGORITHMS_BASE: { value: Algorithm; label: string }[] = [
  { value: 'kmeans', label: 'K-Means' },
  { value: 'hdbscan', label: 'HDBSCAN' },
];

export default function ImageClusterPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'cluster-progress', taskId: 'image-cluster', pythonEnv: true, logProcessing: p => !!p.message });
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [algorithm, setAlgorithm] = useState<Algorithm>('kmeans');
  const [featureType, setFeatureType] = useState<FeatureType>('semantic');
  const [nClusters, setNClusters] = useState(8);
  const [minClusterSize, setMinClusterSize] = useState(5);
  const [device, setDevice] = useState<'auto' | 'cpu'>('auto');
  const [wStyle, setWStyle] = useState(0.5);
  const [wSemantic, setWSemantic] = useState(0.5);
  const [wColor, setWColor] = useState(0.0);
  const [mapTheme, setMapTheme] = useState<'light' | 'dark'>('light');

  const handleProcess = () => {
    const algoLabel = ALGORITHMS_BASE.find(a => a.value === algorithm)?.label || algorithm;
    const paramStr = algorithm === 'kmeans' ? `${t('imageCluster.groupCount')}: ${nClusters}` : `${t('imageCluster.minClusterSize')}: ${minClusterSize}`;
    const deviceLabel = device === 'auto' ? t('imageCluster.gpuAuto') : 'CPU';
    return task.run({
      taskName: t('imageCluster.taskName'), startLog: t('imageCluster.startMsg', { algo: algoLabel, feat: featureType, param: paramStr, device: deviceLabel }), exec: () => invoke<ProcessResult>('start_image_cluster', {
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
          recursive,
        },
      })
    });
  };

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
      <PageHeader icon={Network} color={'#a78bfa'} title={t('imageCluster.title')} subtitle={t('imageCluster.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧设置 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 文件夹 */}
          <PathFields title={t('imageCluster.folderSelect')} input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

          {/* 参数设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('imageCluster.paramSettings')}</span>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <DeviceToggle useGpu={device === 'auto'} onChange={v => setDevice(v ? 'auto' : 'cpu')} />
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

                </div>
              )}

              {/* 分组参数 + 分布图主题 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 'var(--space-4)', alignItems: 'start' }}>
                <div className="form-group" style={{ marginBottom: 0, opacity: algorithm === 'kmeans' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
                  <label className="form-label">{t('imageCluster.groupCount')}</label>
                  <NumberInput className="form-input" min={2} value={nClusters} disabled={algorithm !== 'kmeans'} style={{ height: 36 }} onChange={setNClusters} fallback={8} integer />
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>{t('imageCluster.groupCountTip')}</span>
                </div>
                <div className="form-group" style={{ marginBottom: 0, opacity: algorithm === 'hdbscan' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
                  <label className="form-label">{t('imageCluster.minClusterSize')}</label>
                  <NumberInput className="form-input" min={2} value={minClusterSize} disabled={algorithm !== 'hdbscan'} style={{ height: 36 }} onChange={setMinClusterSize} fallback={5} integer />

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
              {algorithm === 'hdbscan' && (<div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(167, 139, 250, 0.06)', border: '1px solid rgba(167, 139, 250, 0.1)' }}>
                <Info style={{ width: 13, height: 13, color: '#a78bfa', marginTop: 2, minWidth: 13 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  {t('imageCluster.hdbscanTip')}
                </span>
              </div>)}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_image_cluster" forceCancelCommand="force_cancel_image_cluster"
            startText={t('imageCluster.startCluster')} processingText={t('imageCluster.clustering')} />
          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
