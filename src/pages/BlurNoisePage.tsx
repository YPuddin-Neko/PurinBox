import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import {
  Sparkles,
  FolderOpen,
  Info,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

export default function BlurNoisePage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [blurRadius, setBlurRadius] = useState(2.0);
  const [noiseStrength, setNoiseStrength] = useState(15);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('blur-noise-progress', (event) => {
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
    setProcessing(true); addTask('blur-noise', '模糊/噪点');
    setProgress(0); setProgressCurrent(0); setProgressTotal(0); setIsDone(false); setHasError(false);
    const parts = [];
    if (blurRadius > 0) parts.push(`模糊半径: ${blurRadius.toFixed(1)}`);
    if (noiseStrength > 0) parts.push(`噪点强度: ${noiseStrength}`);
    setLogs([{ time: getTimeStr(), message: `开始处理 | ${parts.join(' | ')}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('blur_noise_images', {
        options: { input_path: inputPath, output_path: outputPath, blur_radius: blurRadius, noise_strength: noiseStrength },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Sparkles style={{ width: 28, height: 28, color: '#60a5fa' }} />
          <h1 className="page-title">模糊 / 噪点</h1>
        </div>
        <p className="page-subtitle">为图片添加高斯模糊和随机噪点</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">参数设置</span></div>
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
                  <input className="form-input" placeholder="选择输出目标文件夹..." value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>

              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)' }}>
                <div className="form-group" style={{ marginBottom: 0 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>模糊半径</span>
                    <span style={{ fontFamily: 'monospace', color: '#60a5fa', fontSize: 'var(--font-size-sm)' }}>{blurRadius.toFixed(1)}</span>
                  </label>
                  <input type="range" min="0" max="10" step="0.5" value={blurRadius} onChange={(e) => setBlurRadius(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#60a5fa' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    <span>0 (不模糊)</span>
                    <span>10.0 (强模糊)</span>
                  </div>
                </div>

                <div className="form-group" style={{ marginBottom: 0 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>噪点强度</span>
                    <span style={{ fontFamily: 'monospace', color: '#a78bfa', fontSize: 'var(--font-size-sm)' }}>{noiseStrength}</span>
                  </label>
                  <input type="range" min="0" max="100" step="1" value={noiseStrength} onChange={(e) => setNoiseStrength(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#a78bfa' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    <span>0 (无噪点)</span>
                    <span>100 (强噪点)</span>
                  </div>
                </div>
              </div>

              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(96, 165, 250, 0.06)', border: '1px solid rgba(96, 165, 250, 0.1)' }}>
                <Info style={{ width: 13, height: 13, color: '#60a5fa', marginTop: 2, minWidth: 13 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-secondary)', lineHeight: 1.5 }}>
                  模糊和噪点可以同时使用，也可以只用其中一种（将另一个设为 0）。推荐模糊半径 1.0~3.0，噪点强度 5~25。
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath || (blurRadius <= 0 && noiseStrength <= 0)}
            cancelCommand="cancel_blur_noise" startText="开始处理" processingText="处理中..."
            onCancelLog={addCancelLog} />
          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
