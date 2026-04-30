import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { Layers, FolderOpen, Play, Loader2, Info } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const bgOptions = [
  { value: 'white', label: '白色背景', desc: '用白色填充透明区域，适用于大部分训练场景', color: '#e8eaf0' },
  { value: 'black', label: '黑色背景', desc: '用黑色填充透明区域', color: '#5a5e78' },
];

export default function AlphaConvertPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [background, setBackground] = useState('white');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('alpha-progress', (event) => {
      const p = event.payload;
      setProgressCurrent(p.current);
      setProgressTotal(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasError(true);
      if (p.status !== 'processing') {
        setLogs((prev) => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status as LogEntry['status'] }]);
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
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: `开始检测并转换透明通道 (背景: ${background === 'white' ? '白色' : '黑色'})`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('convert_alpha', {
        options: { input_path: inputPath, output_path: outputPath, background },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Layers style={{ width: 28, height: 28, color: '#c084fc' }} />
          <h1 className="page-title">转换透明通道</h1>
        </div>
        <p className="page-subtitle">检测图片是否包含透明通道，并将透明区域转换为指定背景色</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">路径设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">输入路径</label>
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

          {/* 背景色 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">透明区域填充</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {bgOptions.map((opt) => (
                <div key={opt.value} onClick={() => setBackground(opt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${background === opt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: background === opt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 20, height: 20, borderRadius: '50%', minWidth: 20, border: `2px solid ${background === opt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {background === opt.value && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <div style={{
                    width: 28, height: 28, borderRadius: 'var(--radius-sm)', minWidth: 28,
                    background: opt.value === 'white' ? '#ffffff' : '#1a1a1a',
                    border: '1px solid var(--color-border)',
                  }} />
                  <div>
                    <div style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* 说明 */}
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(192, 132, 252, 0.04)', border: '1px solid rgba(192, 132, 252, 0.1)' }}>
            <Info style={{ width: 18, height: 18, color: '#c084fc', marginTop: 2, minWidth: 18 }} />
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
              <strong style={{ color: 'var(--color-text-primary)' }}>用途说明：</strong>许多 AI 训练模型不支持带透明通道的图片。此工具会自动检测图片是否包含透明（Alpha）通道，如果有则将透明区域用指定背景色填充并转换为不透明图片。没有透明通道的图片会原样复制到输出目录。
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>背景色</span>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <div style={{ width: 14, height: 14, borderRadius: '50%', background: background === 'white' ? '#fff' : '#1a1a1a', border: '1px solid var(--color-border)' }} />
                  <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: 'var(--color-text-primary)' }}>{background === 'white' ? '白色' : '黑色'}</span>
                </div>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>输入</span>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl' }}>{inputPath || '未设置'}</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>输出</span>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl' }}>{outputPath || '未设置'}</span>
              </div>
            </div>
          </div>

          <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleProcess} disabled={processing || !inputPath || !outputPath}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 处理中...</> : <><Play style={{ width: 18, height: 18 }} /> 开始转换</>}
          </button>

          {(logs.length > 0 || processing) && (
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
          )}
        </div>
      </div>
    </div>
  );
}
