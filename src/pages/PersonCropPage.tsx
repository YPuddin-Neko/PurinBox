import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import {
  ScanFace, FolderOpen, Loader2, Info, User,
  PersonStanding, CircleUser, Eye, Download,
  Cpu, Gpu,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface CropModelInfo { id: string; name: string; crop_type: string; size_mb: number; downloaded: boolean; path: string; }
interface DlProgress { downloaded: number; total: number; percent: number; speed_mbps: number; status: string; message: string; }

export default function PersonCropPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [useGpu, setUseGpu] = useState(() => localStorage.getItem('person_crop_gpu') === 'true');
  const [models, setModels] = useState<CropModelInfo[]>([]);
  const [downloading, setDownloading] = useState(false);
  const [personEnabled, setPersonEnabled] = useState(true);
  const [personConf, setPersonConf] = useState(0.3);
  const [upperEnabled, setUpperEnabled] = useState(true);
  const [upperConf, setUpperConf] = useState(0.5);
  const [upperTag, setUpperTag] = useState('upper body');
  const [headEnabled, setHeadEnabled] = useState(true);
  const [headConf, setHeadConf] = useState(0.4);
  const [headTag, setHeadTag] = useState('head view');
  const [headScale, setHeadScale] = useState(1.5);
  const [eyesEnabled, setEyesEnabled] = useState(true);
  const [eyesConf, setEyesConf] = useState(0.3);
  const [eyesTag, setEyesTag] = useState('eyes view');
  const [eyesScale, setEyesScale] = useState(2.4);
  const [keepOriginalTags, setKeepOriginalTags] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  const loadModels = useCallback(async () => {
    try { setModels(await invoke<CropModelInfo[]>('get_person_crop_models')); } catch (e) { console.error(e); }
  }, []);

  useEffect(() => { loadModels(); }, []);


  const downloadAll = async () => {
    setDownloading(true);
    setLogs(prev => [...prev, { time: getTimeStr(), message: '开始下载模型包...', status: 'info' }]);
    try {
      await invoke('download_person_crop_model', { modelId: 'all' });
      loadModels();
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: `下载失败: ${String(e)}`, status: 'error' }]);
    } finally { setDownloading(false); }
  };

  useEffect(() => {
    let active = true;
    const p = listen<DlProgress>('person-crop-download', (e) => {
      if (!active) return;
      const d = e.payload;
      if (d.status === 'done' || d.status === 'cancelled') {
        setLogs(prev => [...prev.filter(l => l.status !== 'download'), { time: getTimeStr(), message: d.message, status: d.status === 'done' ? 'success' : 'info' }]);
        if (d.status === 'done') loadModels();
      } else if (d.status === 'error') {
        setDownloading(false);
        setLogs(prev => [...prev.filter(l => l.status !== 'download'), { time: getTimeStr(), message: d.message, status: 'error' }]);
      } else {
        setLogs(prev => {
          const idx = prev.findIndex(l => l.status === 'download');
          const entry: LogEntry = { time: getTimeStr(), message: d.message, status: 'download', dlPercent: d.percent, dlSpeed: d.speed_mbps > 0 ? `${d.speed_mbps.toFixed(1)} MB/s` : '' };
          if (idx >= 0) { const n = [...prev]; n[idx] = entry; return n; }
          return [...prev, entry];
        });
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, [loadModels]);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('person-crop-progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current); setProgressTotal(d.total);
      if (d.total > 0) setProgress((d.current / d.total) * 100);
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs((prev) => [...prev, { time: getTimeStr(), message: d.message, status: d.status === 'done' ? 'info' : d.status as LogEntry['status'] }]);
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, []);

  const selectInputFolder = async () => { const s = await open({ directory: true, multiple: false, title: '选择输入文件夹' }); if (s) setInputPath(s as string); };
  const selectOutputFolder = async () => { const s = await open({ directory: true, multiple: false, title: '选择输出文件夹' }); if (s) setOutputPath(s as string); };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true); addTask('person-crop', '三分法裁切');
    setProgress(0); setProgressCurrent(0); setProgressTotal(0); setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: '开始三分法裁切处理...', status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_person_crop', {
        options: {
          input_path: inputPath, output_path: outputPath, use_gpu: useGpu,
          person_enabled: personEnabled, person_conf: personConf,
          upper_enabled: upperEnabled, upper_conf: upperConf, upper_tag: upperTag,
          head_enabled: headEnabled, head_conf: headConf, head_tag: headTag, head_scale: headScale,
          eyes_enabled: eyesEnabled, eyes_conf: eyesConf, eyes_tag: eyesTag, eyes_scale: eyesScale,
          keep_original_tags: keepOriginalTags,
        },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);
  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);

  const detCard = (
    enabled: boolean, setEnabled: (v: boolean) => void,
    icon: React.ReactNode, label: string, color: string, alphaBase: string,
    modelType: string, children: React.ReactNode,
  ) => {
    const m = models.find(x => x.crop_type === modelType);
    return (
      <div style={{ padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', border: `1px solid ${enabled ? `${alphaBase}0.35)` : 'var(--color-border)'}`, background: enabled ? `${alphaBase}0.04)` : 'var(--color-bg-input)', transition: 'all 0.2s' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: enabled ? 'var(--space-3)' : 0 }}>
          <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', flex: 1 }}>
            <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} style={{ accentColor: color, width: 16, height: 16 }} />
            <span style={{ color: enabled ? color : 'var(--color-text-tertiary)' }}>{icon}</span>
            <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{label}</span>
          </label>
          {m && <span style={{ fontSize: 10, padding: '2px 8px', borderRadius: 10, background: m.downloaded ? 'rgba(74,222,128,0.1)' : 'rgba(251,191,36,0.1)', color: m.downloaded ? '#4ade80' : '#fbbf24', fontWeight: 600 }}>{m.downloaded ? '✓ 就绪' : '⬇ 待下载'}</span>}
        </div>
        {enabled && <div style={{ paddingLeft: 32 }}>{children}</div>}
      </div>
    );
  };

  const confSlider = (value: number, onChange: (v: number) => void, color: string) => (
    <div className="form-group">
      <label className="form-label">置信度阈值</label>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
        <input type="range" min={0.1} max={0.9} step={0.05} value={value} onChange={(e) => onChange(Number(e.target.value))} style={{ flex: 1, accentColor: color }} />
        <input className="form-input" type="number" value={value} onChange={(e) => onChange(Number(e.target.value))} step={0.05} min={0.1} max={0.9} style={{ width: 70, textAlign: 'center' }} />
      </div>
    </div>
  );

  const tagInput = (value: string, onChange: (v: string) => void, placeholder: string) => (
    <div className="form-group">
      <label className="form-label">追加 Tag</label>
      <input className="form-input" value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} />
    </div>
  );

  const modelStatusSummary = () => {
    const needed = models.filter(m => {
      if (m.crop_type === 'person') return personEnabled;
      if (m.crop_type === 'halfbody') return upperEnabled;
      if (m.crop_type === 'head') return headEnabled;
      if (m.crop_type === 'eyes') return eyesEnabled;
      return false;
    });
    const ready = needed.filter(m => m.downloaded).length;
    const total = needed.length;
    return { ready, total, allReady: ready === total && total > 0 };
  };

  const ms = modelStatusSummary();

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <ScanFace style={{ width: 28, height: 28, color: '#fb923c' }} />
          <h1 className="page-title">三分法裁切</h1>
        </div>
        <p className="page-subtitle">基于 DeepGHS 动漫专用检测模型，自动检测图片中的人物并按多个层级进行裁切</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">裁切设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">输入文件夹</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择图片所在文件夹..." value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">输出文件夹</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择输出文件夹..." value={outputPath} onChange={(e) => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              {/* 模型状态 + 下载 */}
              <div className="form-group">
                <label className="form-label">检测模型</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'center' }}>
                  <div style={{ flex: 1, padding: '8px 12px', borderRadius: 'var(--radius-md)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }}>
                    {ms.allReady ? (
                      <span style={{ color: '#4ade80' }}>✓ 所有模型已就绪 ({ms.ready}/{ms.total})</span>
                    ) : (
                      <span style={{ color: '#fbbf24' }}>⬇ 需要下载模型 ({ms.ready}/{ms.total} 已就绪)</span>
                    )}
                  </div>
                  <button className="btn btn-secondary" onClick={downloadAll} disabled={downloading || ms.allReady}
                    style={{ height: 36, padding: '0 12px', gap: 6, display: 'flex', alignItems: 'center' }}>
                    {downloading ? <Loader2 style={{ width: 15, height: 15, animation: 'spin 1s linear infinite' }} /> : <Download style={{ width: 15, height: 15 }} />}
                    {downloading ? '下载中...' : '下载模型'}
                  </button>
                </div>
                <div style={{ marginTop: 6, fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>
                  来源: <a href="https://huggingface.co/deepghs" target="_blank" rel="noreferrer" style={{ color: '#818cf8' }}>deepghs</a> — 动漫专用检测模型，每种裁切类型使用独立的专用模型
                </div>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                {([{ val: false, label: 'CPU', icon: <Cpu style={{ width: 13, height: 13 }} />, color: '#fbbf24' },
                  { val: true, label: 'GPU', icon: <Gpu style={{ width: 13, height: 13 }} />, color: '#4ade80' }] as const).map(d => (
                  <button key={d.label} onClick={() => { setUseGpu(d.val); localStorage.setItem('person_crop_gpu', String(d.val)); }} style={{
                    padding: '4px 12px', borderRadius: 'var(--radius-sm)', fontSize: 11, fontWeight: 700,
                    cursor: 'pointer', transition: 'all 0.15s',
                    display: 'flex', alignItems: 'center', gap: 4,
                    border: `1.5px solid ${useGpu === d.val ? d.color : 'var(--color-border)'}`,
                    background: useGpu === d.val ? `${d.color}12` : 'transparent',
                    color: useGpu === d.val ? d.color : 'var(--color-text-tertiary)',
                  }}>
                    {d.icon} {d.label}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {/* 检测选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">检测选项</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {detCard(personEnabled, setPersonEnabled, <PersonStanding style={{ width: 18, height: 18 }} />, '全身检测', '#4ade80', 'rgba(74, 222, 128, ', 'person', <>
                {confSlider(personConf, setPersonConf, '#4ade80')}
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(74, 222, 128, 0.06)', border: '1px solid rgba(74, 222, 128, 0.1)', marginTop: 8 }}>
                  <Info style={{ width: 13, height: 13, color: '#4ade80', marginTop: 2, minWidth: 13 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>使用专用全身检测模型，精确检测动漫角色完整身体并裁切。</span>
                </div>
              </>)}

              {detCard(upperEnabled, setUpperEnabled, <User style={{ width: 18, height: 18 }} />, '半身检测', '#818cf8', 'rgba(129, 140, 248, ', 'halfbody', <>
                {confSlider(upperConf, setUpperConf, '#818cf8')}
                {tagInput(upperTag, setUpperTag, 'upper body')}
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(129, 140, 248, 0.06)', border: '1px solid rgba(129, 140, 248, 0.1)', marginTop: 8 }}>
                  <Info style={{ width: 13, height: 13, color: '#818cf8', marginTop: 2, minWidth: 13 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>使用专用半身检测模型，直接检测上半身区域。</span>
                </div>
              </>)}

              {detCard(headEnabled, setHeadEnabled, <CircleUser style={{ width: 18, height: 18 }} />, '头部检测', '#f59e0b', 'rgba(245, 158, 11, ', 'head', <>
                {confSlider(headConf, setHeadConf, '#f59e0b')}
                <div className="form-group" style={{ marginTop: 8, marginBottom: 4 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>缩放系数</span>
                    <span style={{ fontFamily: 'monospace', color: '#f59e0b', fontSize: 'var(--font-size-sm)' }}>{headScale.toFixed(1)}x</span>
                  </label>
                  <input type="range" min="1.0" max="3.0" step="0.1" value={headScale} onChange={(e) => setHeadScale(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#f59e0b' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}><span>1.0x (仅头部)</span><span>3.0x (更多周围)</span></div>
                </div>
                {tagInput(headTag, setHeadTag, 'head view')}
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(245, 158, 11, 0.06)', border: '1px solid rgba(245, 158, 11, 0.1)', marginTop: 8 }}>
                  <Info style={{ width: 13, height: 13, color: '#f59e0b', marginTop: 2, minWidth: 13 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>检测头部后按缩放系数扩大裁切区域，值越大包含越多周围区域。</span>
                </div>
              </>)}

              {detCard(eyesEnabled, setEyesEnabled, <Eye style={{ width: 18, height: 18 }} />, '眼部检测', '#f472b6', 'rgba(244, 114, 182, ', 'eyes', <>
                {confSlider(eyesConf, setEyesConf, '#f472b6')}
                <div className="form-group" style={{ marginTop: 8, marginBottom: 4 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>缩放系数</span>
                    <span style={{ fontFamily: 'monospace', color: '#f472b6', fontSize: 'var(--font-size-sm)' }}>{eyesScale.toFixed(1)}x</span>
                  </label>
                  <input type="range" min="1.0" max="4.0" step="0.1" value={eyesScale} onChange={(e) => setEyesScale(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#f472b6' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}><span>1.0x (仅眼部)</span><span>4.0x (更多周围)</span></div>
                </div>
                {tagInput(eyesTag, setEyesTag, 'eyes view')}
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(244, 114, 182, 0.06)', border: '1px solid rgba(244, 114, 182, 0.1)', marginTop: 8 }}>
                  <Info style={{ width: 13, height: 13, color: '#f472b6', marginTop: 2, minWidth: 13 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>检测眼部后按缩放系数扩大裁切区域，值越大包含越多周围区域。</span>
                </div>
              </>)}
            </div>
          </div>

          {/* 其他选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">其他选项</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
                <input type="checkbox" checked={keepOriginalTags} onChange={(e) => setKeepOriginalTags(e.target.checked)} style={{ accentColor: '#7c5cfc', width: 16, height: 16 }} />
                <span style={{ fontWeight: 600, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-sm)' }}>保留原始标签</span>
              </label>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(124, 92, 252, 0.06)', border: '1px solid rgba(124, 92, 252, 0.1)' }}>
                <Info style={{ width: 13, height: 13, color: '#7c5cfc', marginTop: 2, minWidth: 13 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  勾选后会将原图的 .txt 标签内容一并复制到裁切后的标签文件中。
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath || !ms.allReady}
            cancelCommand="cancel_person_crop" startText="开始裁切" processingText="处理中..."
            onCancelLog={addCancelLog} />
          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
