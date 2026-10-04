import { invoke } from '@tauri-apps/api/core';
import {
  Info,
  Moon,
  Network,
  Sun
} from 'lucide-react';
import { useState, type CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';
import type { ClusterOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PathFields from '../components/ui/PathFields';
import RangeField from '../components/ui/RangeField';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

type Algorithm = ClusterOptions['algorithm'];
type FeatureType = ClusterOptions['feature_type'];

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
  const [device, setDevice] = useState<ClusterOptions['device']>('auto');
  const [wStyle, setWStyle] = useState(0.5);
  const [wSemantic, setWSemantic] = useState(0.5);
  const [wColor, setWColor] = useState(0.0);
  const [mapTheme, setMapTheme] = useState<ClusterOptions['map_theme']>('light');

  const handleProcess = () => {
    const algoLabel = ALGORITHMS_BASE.find(a => a.value === algorithm)?.label || algorithm;
    const paramStr = algorithm === 'kmeans' ? `${t('imageCluster.groupCount')}: ${nClusters}` : `${t('imageCluster.minClusterSize')}: ${minClusterSize}`;
    const deviceLabel = device === 'auto' ? t('imageCluster.gpuAuto') : 'CPU';
    return task.run({
      taskName: t('imageCluster.taskName'),
      startLog: t('imageCluster.startMsg', { algo: algoLabel, feat: featureType, param: paramStr, device: deviceLabel }),
      exec: () => invoke<ProcessResult>('start_image_cluster', {
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
        } satisfies ClusterOptions,
      }),
    });
  };

  const features: { value: FeatureType; label: string; desc: string; color: string }[] = [
    { value: 'style', label: t('imageCluster.styleLabel'), desc: t('imageCluster.styleDesc'), color: '#f472b6' },
    { value: 'semantic', label: t('imageCluster.semanticLabel'), desc: t('imageCluster.semanticDesc'), color: '#60a5fa' },
    { value: 'fusion', label: t('imageCluster.fusionLabel'), desc: t('imageCluster.fusionDesc'), color: '#a78bfa' },
  ];
  const weights = [
    { label: t('imageCluster.styleLabel'), value: wStyle, set: setWStyle, color: '#f472b6' },
    { label: t('imageCluster.semanticLabel'), value: wSemantic, set: setWSemantic, color: '#60a5fa' },
    { label: t('imageCluster.colorLabel'), value: wColor, set: setWColor, color: '#fbbf24' },
  ];
  const mapThemes = [
    { value: 'light' as const, label: t('imageCluster.mapLight'), Icon: Sun },
    { value: 'dark' as const, label: t('imageCluster.mapDark'), Icon: Moon },
  ];

  return (
    <ToolPageLayout icon={Network} color="#a78bfa" title={t('imageCluster.title')} subtitle={t('imageCluster.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath}
          cancelCommand="cancel_image_cluster" forceCancelCommand="force_cancel_image_cluster"
          startText={t('imageCluster.startCluster')} processingText={t('imageCluster.clustering')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields title={t('imageCluster.folderSelect')} input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive} />

      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('imageCluster.paramSettings')}</span>
          <DeviceToggle useGpu={device === 'auto'} onChange={v => setDevice(v ? 'auto' : 'cpu')} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          <div className="form-group">
            <label className="form-label">{t('imageCluster.clusterAlgo')}</label>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              {ALGORITHMS_BASE.map(a => (
                <button key={a.value} type="button" className="ui-toggle" aria-pressed={algorithm === a.value}
                  onClick={() => setAlgorithm(a.value)} style={{ flex: 1, padding: '10px 0', fontSize: 13 }}>
                  {a.label}
                </button>
              ))}
            </div>
          </div>

          <div className="form-group">
            <label className="form-label">{t('imageCluster.featureType')}</label>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              {features.map(f => (
                <button key={f.value} type="button" className="ui-toggle" aria-pressed={featureType === f.value}
                  onClick={() => setFeatureType(f.value)}
                  style={{ flex: 1, flexDirection: 'column', gap: 2, padding: '10px 0', fontSize: 12, '--toggle-color': f.color } as CSSProperties}>
                  <span>{f.label}</span>
                  <span style={{ fontSize: 9, opacity: 0.7, fontWeight: 400 }}>{f.desc}</span>
                </button>
              ))}
            </div>
          </div>

          {featureType === 'fusion' && (
            <div className="form-group" style={{ background: 'rgba(167,139,250,0.04)', border: '1px solid rgba(167,139,250,0.1)', borderRadius: 'var(--radius-sm)', padding: 'var(--space-3)' }}>
              <label className="form-label" style={{ marginBottom: 'var(--space-2)' }}>{t('imageCluster.fusionWeight')}</label>
              {weights.map(w => (
                <RangeField key={w.label} inline label={w.label} value={w.value} onChange={w.set}
                  min={0} max={1} step={0.1} color={w.color} format={v => v.toFixed(1)} style={{ marginBottom: 6 }} />
              ))}
            </div>
          )}

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 'var(--space-4)', alignItems: 'start' }}>
            <div className="form-group" style={{ opacity: algorithm === 'kmeans' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
              <label className="form-label">{t('imageCluster.groupCount')}</label>
              <NumberInput className="form-input" min={2} value={nClusters} disabled={algorithm !== 'kmeans'} style={{ height: 36 }} onChange={setNClusters} fallback={8} integer />
              <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>{t('imageCluster.groupCountTip')}</span>
            </div>
            <div className="form-group" style={{ opacity: algorithm === 'hdbscan' ? 1 : 0.4, transition: 'opacity 0.15s' }}>
              <label className="form-label">{t('imageCluster.minClusterSize')}</label>
              <NumberInput className="form-input" min={2} value={minClusterSize} disabled={algorithm !== 'hdbscan'} style={{ height: 36 }} onChange={setMinClusterSize} fallback={5} integer />
            </div>
            <div className="form-group" style={{ minWidth: 140 }}>
              <label className="form-label">{t('imageCluster.mapTheme')}</label>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                {mapThemes.map(({ value, label, Icon }) => (
                  <button key={value} type="button" className="ui-toggle" aria-pressed={mapTheme === value}
                    onClick={() => setMapTheme(value)} style={{ flex: 1, height: 36, fontSize: 12, fontWeight: 600 }}>
                    <Icon style={{ width: 13, height: 13 }} /> {label}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {algorithm === 'hdbscan' && (
            <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(167, 139, 250, 0.06)', border: '1px solid rgba(167, 139, 250, 0.1)' }}>
              <Info style={{ width: 13, height: 13, color: '#a78bfa', marginTop: 2, minWidth: 13 }} />
              <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                {t('imageCluster.hdbscanTip')}
              </span>
            </div>
          )}
        </div>
      </div>
    </ToolPageLayout>
  );
}
