import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  Scaling,
  FolderOpen,
  Play,
  ArrowUpCircle,
  ArrowDownCircle,
  Info,
  Loader2,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';

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

export default function ScalePage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [mode, setMode] = useState<'upscale' | 'downscale'>('upscale');
  const [upWidth, setUpWidth] = useState(1024);
  const [upHeight, setUpHeight] = useState(1024);
  const [downWidth, setDownWidth] = useState(512);
  const [downHeight, setDownHeight] = useState(512);
  const [processing, setProcessing] = useState(false);
  const [result, setResult] = useState<ProcessResult | null>(null);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  const targetWidth = mode === 'upscale' ? upWidth : downWidth;
  const targetHeight = mode === 'upscale' ? upHeight : downHeight;

  // Listen for progress events
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('scale-progress', (event) => {
      const p = event.payload;
      setProgressCurrent(p.current);
      setProgressTotal(p.total);
      if (p.total > 0) {
        setProgress((p.current / p.total) * 100);
      }
      if (p.status === 'done') {
        setIsDone(true);
      }
      if (p.status === 'error') {
        setHasError(true);
      }
      // Only add meaningful log entries (not processing status)
      if (p.status !== 'processing') {
        setLogs((prev) => [...prev, {
          time: getTimeStr(),
          message: p.message,
          status: p.status === 'done' ? 'info' : p.status as LogEntry['status'],
        }]);
      }
    }).then((fn) => { unlisten = fn; });

    return () => { unlisten?.(); };
  }, []);

  const selectInputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择输入文件夹' });
    if (selected) setInputPath(selected as string);
  };

  const selectOutputFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择输出文件夹' });
    if (selected) setOutputPath(selected as string);
  };

  const handleProcess = async () => {
    if (!inputPath || !outputPath) return;
    setProcessing(true);
    setResult(null);
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);
    setLogs([{ time: getTimeStr(), message: `开始${mode === 'upscale' ? '上采样' : '下采样'}处理 → ${targetWidth}×${targetHeight}`, status: 'info' }]);
    try {
      const res = await invoke<ProcessResult>('scale_images', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode,
          target_width: targetWidth,
          target_height: targetHeight,
        },
      });
      setResult(res);
    } catch (e: any) {
      setResult({ success_count: 0, fail_count: 1, total: 1, errors: [String(e)] });
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true);
      setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); setResult(null); }, []);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Scaling style={{ width: 28, height: 28, color: '#7c5cfc' }} />
          <h1 className="page-title">图片缩放</h1>
        </div>
        <p className="page-subtitle">支持单个或批量缩放图片，将图片上采样或下采样到目标分辨率</p>
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

          {/* 缩放选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">缩放选项</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {/* 上采样 */}
              <div onClick={() => setMode('upscale')} style={{
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${mode === 'upscale' ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                background: mode === 'upscale' ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                  <div style={{ width: 20, height: 20, borderRadius: '50%', border: `2px solid ${mode === 'upscale' ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {mode === 'upscale' && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <ArrowUpCircle style={{ width: 18, height: 18, color: mode === 'upscale' ? '#4ade80' : 'var(--color-text-tertiary)' }} />
                  <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>上采样到目标分辨率</span>
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">宽度 (px)</label>
                    <input className="form-input" type="number" value={upWidth} onChange={(e) => setUpWidth(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">高度 (px)</label>
                    <input className="form-input" type="number" value={upHeight} onChange={(e) => setUpHeight(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(74, 222, 128, 0.06)', border: '1px solid rgba(74, 222, 128, 0.1)' }}>
                  <Info style={{ width: 14, height: 14, color: '#4ade80', marginTop: 2, minWidth: 14 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                    将低于目标分辨率的图片放大到指定尺寸。适用于需要将小尺寸训练图片统一放大到模型要求的输入分辨率，如 SD/SDXL 训练需要 512×512 或 1024×1024 的场景。
                  </span>
                </div>
              </div>

              {/* 下采样 */}
              <div onClick={() => setMode('downscale')} style={{
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${mode === 'downscale' ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                background: mode === 'downscale' ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                  <div style={{ width: 20, height: 20, borderRadius: '50%', border: `2px solid ${mode === 'downscale' ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {mode === 'downscale' && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <ArrowDownCircle style={{ width: 18, height: 18, color: mode === 'downscale' ? '#60a5fa' : 'var(--color-text-tertiary)' }} />
                  <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>下采样到目标分辨率</span>
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">宽度 (px)</label>
                    <input className="form-input" type="number" value={downWidth} onChange={(e) => setDownWidth(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label">高度 (px)</label>
                    <input className="form-input" type="number" value={downHeight} onChange={(e) => setDownHeight(Number(e.target.value))} onClick={(e) => e.stopPropagation()} min={1} />
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(96, 165, 250, 0.06)', border: '1px solid rgba(96, 165, 250, 0.1)' }}>
                  <Info style={{ width: 14, height: 14, color: '#60a5fa', marginTop: 2, minWidth: 14 }} />
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                    将超过目标分辨率的图片缩小到指定尺寸。适用于将高分辨率原图缩小以减少显存占用、加速训练，或统一数据集中图片的最大尺寸。
                  </span>
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 摘要 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>模式</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: mode === 'upscale' ? '#4ade80' : '#60a5fa', padding: '2px 10px', borderRadius: 'var(--radius-full)', background: mode === 'upscale' ? 'rgba(74, 222, 128, 0.1)' : 'rgba(96, 165, 250, 0.1)' }}>
                  {mode === 'upscale' ? '上采样' : '下采样'}
                </span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>目标尺寸</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: 'var(--color-text-primary)' }}>{targetWidth} × {targetHeight}</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>输入</span>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl', textAlign: 'right' }}>{inputPath || '未设置'}</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>输出</span>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl', textAlign: 'right' }}>{outputPath || '未设置'}</span>
              </div>
            </div>
          </div>

          {/* 执行按钮 */}
          <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleProcess} disabled={processing || !inputPath || !outputPath}>
            {processing ? (
              <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 处理中...</>
            ) : (
              <><Play style={{ width: 18, height: 18 }} /> 开始缩放</>
            )}
          </button>

          {/* 进度条和日志 */}
          {(logs.length > 0 || processing) && (
            <ProgressLog
              progress={progress}
              current={progressCurrent}
              total={progressTotal}
              logs={logs}
              isDone={isDone}
              hasError={hasError}
              onClearLogs={clearLogs}
            />
          )}
        </div>
      </div>
    </div>
  );
}
