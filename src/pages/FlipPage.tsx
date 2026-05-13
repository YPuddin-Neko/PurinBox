import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useTaskQueue } from '../components/TaskContext';
import {
  FlipHorizontal2,
  FlipVertical2,
  FolderOpen,
  RotateCcw,
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
    let active = true;
    const p = listen<ProgressPayload>('flip-progress', (event) => {
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
    addTask('flip', '图片处理');
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
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

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
        </div>


        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          <ProcessButton processing={processing} onStart={handleProcess}
            disabled={!inputPath || !outputPath}
            cancelCommand="cancel_flip" startText="开始处理" processingText="处理中..."
            onCancelLog={addCancelLog} />

          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
