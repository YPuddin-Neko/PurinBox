import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  FlipHorizontal2,
  FlipVertical2,
  FolderOpen,
  Play,
  Info,
  Loader2,
  RotateCcw,
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

type FlipDirection = 'horizontal' | 'vertical' | 'both';

const flipOptions: { value: FlipDirection; label: string; desc: string; icon: React.ReactNode }[] = [
  { value: 'horizontal', label: '水平翻转', desc: '将图片沿垂直中心轴进行左右镜像翻转', icon: <FlipHorizontal2 /> },
  { value: 'vertical', label: '垂直翻转', desc: '将图片沿水平中心轴进行上下镜像翻转', icon: <FlipVertical2 /> },
  { value: 'both', label: '水平 + 垂直翻转', desc: '同时进行水平和垂直镜像翻转，等同于旋转180°', icon: <RotateCcw /> },
];

export default function FlipPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [direction, setDirection] = useState<FlipDirection>('horizontal');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('flip-progress', (event) => {
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
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);
    const dirLabel = flipOptions.find((o) => o.value === direction)!.label;
    setLogs([{ time: getTimeStr(), message: `开始${dirLabel}处理`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('flip_images', {
        options: { input_path: inputPath, output_path: outputPath, direction },
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
  const currentOption = flipOptions.find((o) => o.value === direction)!;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <FlipHorizontal2 style={{ width: 28, height: 28, color: '#00d4ff' }} />
          <h1 className="page-title">图片处理</h1>
        </div>
        <p className="page-subtitle">对图片进行水平或垂直镜像翻转，支持单个或批量处理</p>
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

          {/* 翻转选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">翻转方向</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {flipOptions.map((opt) => (
                <div key={opt.value} onClick={() => setDirection(opt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${direction === opt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: direction === opt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 20, height: 20, borderRadius: '50%', minWidth: 20, border: `2px solid ${direction === opt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {direction === opt.value && <div style={{ width: 10, height: 10, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <div style={{ width: 36, height: 36, borderRadius: 'var(--radius-sm)', minWidth: 36, background: direction === opt.value ? 'rgba(0, 212, 255, 0.12)' : 'rgba(255,255,255,0.04)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: direction === opt.value ? '#00d4ff' : 'var(--color-text-tertiary)' }}>
                    {opt.icon}
                  </div>
                  <div>
                    <div style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)', marginBottom: 2 }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* 用途说明 */}
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(0, 212, 255, 0.04)', border: '1px solid rgba(0, 212, 255, 0.1)' }}>
            <Info style={{ width: 18, height: 18, color: '#00d4ff', marginTop: 2, minWidth: 18 }} />
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
              <strong style={{ color: 'var(--color-text-primary)' }}>用途说明：</strong>镜像翻转是一种常用的数据增强手段。通过对训练图片进行翻转可以增加数据多样性，帮助模型学习到更鲁棒的特征，减少过拟合风险。
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>翻转方向</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: '#00d4ff', padding: '2px 10px', borderRadius: 'var(--radius-full)', background: 'rgba(0, 212, 255, 0.1)' }}>{currentOption.label}</span>
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
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 处理中...</> : <><Play style={{ width: 18, height: 18 }} /> 开始处理</>}
          </button>

          {(logs.length > 0 || processing) && (
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
          )}
        </div>
      </div>
    </div>
  );
}
