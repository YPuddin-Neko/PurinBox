import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import {
  Crop,
  FolderOpen,
  Maximize2,
  RatioIcon,
  Scissors,
  Info,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult {
  success_count: number;
  fail_count: number;
  total: number;
  errors: string[];
}

interface ProgressPayload {
  current: number;
  total: number;
  filename: string;
  status: string;
  message: string;
}

const PRESETS = [
  { label: '1:1', w: 1, h: 1 },
  { label: '3:4', w: 3, h: 4 },
  { label: '4:3', w: 4, h: 3 },
  { label: '9:16', w: 9, h: 16 },
  { label: '16:9', w: 16, h: 9 },
  { label: '2:3', w: 2, h: 3 },
];

export default function CropPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [mode, setMode] = useState<'center' | 'aspect' | 'edges'>('center');
  // center crop
  const [centerW, setCenterW] = useState(1024);
  const [centerH, setCenterH] = useState(1024);
  // aspect ratio crop
  const [ratioW, setRatioW] = useState(1);
  const [ratioH, setRatioH] = useState(1);
  // edges crop
  const [cropTop, setCropTop] = useState(0);
  const [cropBottom, setCropBottom] = useState(0);
  const [cropLeft, setCropLeft] = useState(0);
  const [cropRight, setCropRight] = useState(0);
  // process state
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('crop-progress', (event) => {
      if (!active) return;
      const p = event.payload;
      setProgressCurrent(p.current);
      setProgressTotal(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasError(true);
      if (p.status !== 'processing') {
        setLogs((prev) => [...prev, {
          time: getTimeStr(),
          message: p.message,
          status: p.status === 'done' ? 'info' : p.status as LogEntry['status'],
        }]);
      }
    });
    return () => { active = false; p.then(fn => fn()); };
  }, []);

  const selectInputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择输入文件夹' });
    if (selected) setInputPath(selected as string);
  };

  const selectOutputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择输出文件夹' });
    if (selected) setOutputPath(selected as string);
  };

  const { addTask } = useTaskQueue();

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true);
    addTask('crop', '图片裁切');
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);

    const modeLabel = mode === 'center' ? '中心裁切' : mode === 'aspect' ? '比例裁切' : '边缘裁切';
    setLogs([{ time: getTimeStr(), message: `开始${modeLabel}处理`, status: 'info' }]);

    try {
      await invoke<ProcessResult>('crop_images', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode,
          target_width: centerW,
          target_height: centerH,
          aspect_ratio: ratioW / ratioH,
          crop_top: cropTop,
          crop_bottom: cropBottom,
          crop_left: cropLeft,
          crop_right: cropRight,
        },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true);
      setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  const modeCards: { key: 'center' | 'aspect' | 'edges'; icon: React.ReactNode; label: string; desc: string; color: string; colorAlpha: string }[] = [
    { key: 'center', icon: <Maximize2 style={{ width: 18, height: 18 }} />, label: '中心裁切', desc: '从图片中心裁切到指定尺寸。适用于统一数据集分辨率，只保留中心区域内容。', color: '#4ade80', colorAlpha: 'rgba(74, 222, 128, ' },
    { key: 'aspect', icon: <RatioIcon style={{ width: 18, height: 18 }} />, label: '比例裁切', desc: '按指定宽高比从中心裁切，去除多余边缘。适用于统一数据集的宽高比，如全部裁切为 1:1 正方形或 3:4 竖版。', color: '#818cf8', colorAlpha: 'rgba(129, 140, 248, ' },
    { key: 'edges', icon: <Scissors style={{ width: 18, height: 18 }} />, label: '边缘裁切', desc: '指定上下左右各裁切多少像素。适用于去除图片边框、水印区域或黑边。', color: '#f59e0b', colorAlpha: 'rgba(245, 158, 11, ' },
  ];

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Crop style={{ width: 28, height: 28, color: '#34d399' }} />
          <h1 className="page-title">图片裁切</h1>
        </div>
        <p className="page-subtitle">支持批量裁切图片，提供中心裁切、比例裁切和边缘裁切三种模式</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">路径设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">输入路径（文件夹或单张图片）</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择图片所在文件夹..." value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group">
                <label className="form-label">输出路径</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择输出文件夹..." value={outputPath} onChange={(e) => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          {/* 裁切模式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">裁切模式</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {modeCards.map((mc) => (
                <div key={mc.key} onClick={() => setMode(mc.key)} style={{
                  padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${mode === mc.key ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: mode === mc.key ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                    <div style={{ width: 20, height: 20, borderRadius: '50%', border: `2px solid ${mode === mc.key ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                      {mode === mc.key && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                    </div>
                    <span style={{ color: mode === mc.key ? mc.color : 'var(--color-text-tertiary)' }}>{mc.icon}</span>
                    <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{mc.label}</span>
                  </div>

                  {/* Mode-specific options */}
                  {mc.key === 'center' && mode === 'center' && (
                    <div style={{ marginBottom: 'var(--space-3)' }}>
                      <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">目标宽度 (px)</label>
                          <input className="form-input" type="number" value={centerW} onChange={(e) => setCenterW(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                        </div>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">目标高度 (px)</label>
                          <input className="form-input" type="number" value={centerH} onChange={(e) => setCenterH(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                        </div>
                      </div>
                    </div>
                  )}

                  {mc.key === 'aspect' && mode === 'aspect' && (
                    <div style={{ marginBottom: 'var(--space-3)' }}>
                      <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)', alignItems: 'flex-end' }}>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">宽度比</label>
                          <input className="form-input" type="number" value={ratioW} onChange={(e) => setRatioW(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                        </div>
                        <span style={{ paddingBottom: 10, fontWeight: 700, color: 'var(--color-text-tertiary)', fontSize: 18 }}>:</span>
                        <div className="form-group" style={{ flex: 1 }}>
                          <label className="form-label">高度比</label>
                          <input className="form-input" type="number" value={ratioH} onChange={(e) => setRatioH(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                        </div>
                      </div>
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                        {PRESETS.map((p) => (
                          <button key={p.label} className="btn btn-ghost" onClick={(e) => { e.stopPropagation(); setRatioW(p.w); setRatioH(p.h); }}
                            style={{
                              fontSize: 11, height: 26, padding: '0 10px', borderRadius: 13,
                              background: ratioW === p.w && ratioH === p.h ? 'rgba(129, 140, 248, 0.15)' : undefined,
                              color: ratioW === p.w && ratioH === p.h ? '#818cf8' : undefined,
                              border: ratioW === p.w && ratioH === p.h ? '1px solid rgba(129, 140, 248, 0.3)' : undefined,
                            }}>
                            {p.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {mc.key === 'edges' && mode === 'edges' && (
                    <div style={{ marginBottom: 'var(--space-3)' }}>
                      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-3)' }}>
                        <div className="form-group">
                          <label className="form-label">上边距 (px)</label>
                          <input className="form-input" type="number" value={cropTop} onChange={(e) => setCropTop(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={0} />
                        </div>
                        <div className="form-group">
                          <label className="form-label">下边距 (px)</label>
                          <input className="form-input" type="number" value={cropBottom} onChange={(e) => setCropBottom(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={0} />
                        </div>
                        <div className="form-group">
                          <label className="form-label">左边距 (px)</label>
                          <input className="form-input" type="number" value={cropLeft} onChange={(e) => setCropLeft(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={0} />
                        </div>
                        <div className="form-group">
                          <label className="form-label">右边距 (px)</label>
                          <input className="form-input" type="number" value={cropRight} onChange={(e) => setCropRight(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={0} />
                        </div>
                      </div>
                    </div>
                  )}

                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: mc.colorAlpha + '0.06)', border: '1px solid ' + mc.colorAlpha + '0.1)' }}>
                    <Info style={{ width: 14, height: 14, color: mc.color, marginTop: 2, minWidth: 14 }} />
                    <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>{mc.desc}</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_crop" startText="开始裁切" processingText="处理中..."
            onCancelLog={addCancelLog} />

          <ProgressLog
            progress={progress}
            current={progressCurrent}
            total={progressTotal}
            logs={logs}
            isDone={isDone}
            hasError={hasError}
            onClearLogs={clearLogs}
          />
        </div>
      </div>
    </div>
  );
}
