import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FileType, FolderOpen, Play, Loader2, Info, ArrowRight } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const targetFormats = [
  { value: 'png', label: 'PNG', desc: '无损压缩，支持透明通道', color: '#4ade80' },
  { value: 'jpg', label: 'JPG', desc: '有损压缩，体积小', color: '#ffa647' },
  { value: 'jpeg', label: 'JPEG', desc: '同 JPG，有损压缩', color: '#ffa647' },
  { value: 'bmp', label: 'BMP', desc: '无压缩位图格式', color: '#f87171' },
  { value: 'webp', label: 'WebP', desc: '现代格式，体积更小', color: '#60a5fa' },
];

const sourceFormats = ['PNG', 'JPG', 'JPEG', 'WebP', 'BMP', 'TIFF', 'GIF', 'PSD'];

export default function FormatConvertPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [targetFormat, setTargetFormat] = useState('png');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('convert-progress', (event) => {
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
    setLogs([{ time: getTimeStr(), message: `开始转换到 .${targetFormat} 格式`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('convert_format', {
        options: { input_path: inputPath, output_path: outputPath, target_format: targetFormat },
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);
  const currentFormat = targetFormats.find((f) => f.value === targetFormat)!;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <FileType style={{ width: 28, height: 28, color: '#ffa647' }} />
          <h1 className="page-title">图片格式转换</h1>
        </div>
        <p className="page-subtitle">支持将主流图片格式（包括 PSD）转换到目标格式</p>
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

          {/* 目标格式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">目标格式</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {targetFormats.map((fmt) => (
                <div key={fmt.value} onClick={() => setTargetFormat(fmt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-3) var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${targetFormat === fmt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: targetFormat === fmt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 18, height: 18, borderRadius: '50%', minWidth: 18, border: `2px solid ${targetFormat === fmt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {targetFormat === fmt.value && <div style={{ width: 8, height: 8, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: targetFormat === fmt.value ? fmt.color : 'var(--color-text-tertiary)', minWidth: 50 }}>.{fmt.value}</span>
                  <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{fmt.desc}</span>
                </div>
              ))}
            </div>
          </div>

          {/* 支持的源格式 */}
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(255, 166, 71, 0.04)', border: '1px solid rgba(255, 166, 71, 0.1)' }}>
            <Info style={{ width: 18, height: 18, color: '#ffa647', marginTop: 2, minWidth: 18 }} />
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
              <strong style={{ color: 'var(--color-text-primary)' }}>支持的源格式：</strong>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 4 }}>
                {sourceFormats.map((f) => (
                  <span key={f} style={{ fontSize: 'var(--font-size-xs)', padding: '1px 8px', borderRadius: 'var(--radius-full)', background: 'rgba(255, 255, 255, 0.04)', border: '1px solid var(--color-border)', color: 'var(--color-text-secondary)' }}>{f}</span>
                ))}
              </div>
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>转换</span>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }}>多种格式</span>
                  <ArrowRight style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                  <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 700, color: currentFormat.color }}>.{targetFormat}</span>
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
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 转换中...</> : <><Play style={{ width: 18, height: 18 }} /> 开始转换</>}
          </button>

          {(logs.length > 0 || processing) && (
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
          )}
        </div>
      </div>
    </div>
  );
}
