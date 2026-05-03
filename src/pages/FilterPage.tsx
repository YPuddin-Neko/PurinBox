import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ScanSearch,
  FolderOpen,
  Play,
  Info,
  Loader2,
  Trash2,
  Copy,
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

type ConditionType = 'min_width' | 'min_height' | 'below_resolution' | 'above_resolution';
type ActionType = 'copy' | 'delete';

const conditionOptions: { value: ConditionType; label: string; desc: string }[] = [
  { value: 'min_width', label: '最小宽度', desc: '筛选宽度低于设定值的图片' },
  { value: 'min_height', label: '最小高度', desc: '筛选高度低于设定值的图片' },
  { value: 'below_resolution', label: '低于指定分辨率', desc: '筛选宽度或高度低于设定值的图片' },
  { value: 'above_resolution', label: '高于指定分辨率', desc: '筛选宽度或高度高于设定值的图片' },
];

export default function FilterPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [action, setAction] = useState<ActionType>('copy');
  const [condition, setCondition] = useState<ConditionType>('below_resolution');
  const [width, setWidth] = useState(512);
  const [height, setHeight] = useState(512);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  const needsOutput = action === 'copy';
  const needsWidth = condition !== 'min_height';
  const needsHeight = condition !== 'min_width';

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('filter-progress', (event) => {
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
    const selected = await open({ directory: true, multiple: false, title: '选择保存文件夹' });
    if (selected) setOutputPath(selected as string);
  };

  const handleProcess = async () => {
    if (!inputPath) return;
    if (action === 'copy' && !outputPath) return;
    setProcessing(true);
    setProgress(0);
    setProgressCurrent(0);
    setProgressTotal(0);
    setIsDone(false);
    setHasError(false);
    const condLabel = conditionOptions.find((c) => c.value === condition)!.label;
    setLogs([{ time: getTimeStr(), message: `开始筛选: 条件=${condLabel}, 操作=${action === 'copy' ? '输出' : '删除'}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('filter_by_resolution', {
        options: { input_path: inputPath, output_path: outputPath || inputPath, action, condition, width, height },
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

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <ScanSearch style={{ width: 28, height: 28, color: '#ff6b9d' }} />
          <h1 className="page-title">分辨率筛选</h1>
        </div>
        <p className="page-subtitle">根据分辨率条件筛选图片，支持输出或删除符合条件的图片</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">路径设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label">输入图片文件夹</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择图片所在文件夹..." value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={selectInputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              {needsOutput && (
                <div className="form-group">
                  <label className="form-label">保存路径（输出匹配的图片）</label>
                  <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                    <input className="form-input" placeholder="选择保存文件夹..." value={outputPath} onChange={(e) => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                    <button className="btn btn-secondary" onClick={selectOutputFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* 操作方式 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">操作方式</span></div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-3)' }}>
              <div onClick={() => setAction('copy')} style={{
                display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)',
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${action === 'copy' ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                background: action === 'copy' ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <Copy style={{ width: 24, height: 24, color: action === 'copy' ? '#4ade80' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>输出</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', textAlign: 'center' }}>将匹配的图片复制到指定目录</span>
              </div>
              <div onClick={() => setAction('delete')} style={{
                display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)',
                padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                border: `1px solid ${action === 'delete' ? 'rgba(248, 113, 113, 0.5)' : 'var(--color-border)'}`,
                background: action === 'delete' ? 'rgba(248, 113, 113, 0.06)' : 'var(--color-bg-input)',
                cursor: 'pointer', transition: 'all 0.2s',
              }}>
                <Trash2 style={{ width: 24, height: 24, color: action === 'delete' ? '#f87171' : 'var(--color-text-tertiary)' }} />
                <span style={{ fontWeight: 700, fontSize: 'var(--font-size-md)', color: 'var(--color-text-primary)' }}>删除</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', textAlign: 'center' }}>直接删除匹配的图片</span>
              </div>
            </div>
            {action === 'delete' && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)', padding: 'var(--space-2) var(--space-3)', borderRadius: 'var(--radius-sm)', background: 'rgba(248, 113, 113, 0.06)', border: '1px solid rgba(248, 113, 113, 0.15)', marginTop: 'var(--space-3)' }}>
                <Info style={{ width: 14, height: 14, color: '#f87171', minWidth: 14 }} />
                <span style={{ fontSize: 'var(--font-size-xs)', color: '#f87171' }}>注意：删除操作不可撤销，请确认后再执行</span>
              </div>
            )}
          </div>

          {/* 筛选条件 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">筛选条件</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {conditionOptions.map((opt) => (
                <div key={opt.value} onClick={() => setCondition(opt.value)} style={{
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                  padding: 'var(--space-3) var(--space-4)', borderRadius: 'var(--radius-md)',
                  border: `1px solid ${condition === opt.value ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: condition === opt.value ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                  cursor: 'pointer', transition: 'all 0.2s',
                }}>
                  <div style={{ width: 18, height: 18, borderRadius: '50%', minWidth: 18, border: `2px solid ${condition === opt.value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {condition === opt.value && <div style={{ width: 8, height: 8, borderRadius: '50%', background: 'var(--color-accent-primary)' }} />}
                  </div>
                  <div>
                    <div style={{ fontWeight: 600, fontSize: 'var(--font-size-base)', color: 'var(--color-text-primary)' }}>{opt.label}</div>
                    <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
                  </div>
                </div>
              ))}
            </div>

            <div style={{ display: 'flex', gap: 'var(--space-3)', marginTop: 'var(--space-4)' }}>
              {needsWidth && (
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{condition === 'min_width' ? '最小宽度 (px)' : '宽度 (px)'}</label>
                  <input className="form-input" type="number" value={width} onChange={(e) => setWidth(Number(e.target.value))} min={1} />
                </div>
              )}
              {needsHeight && (
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">{condition === 'min_height' ? '最小高度 (px)' : '高度 (px)'}</label>
                  <input className="form-input" type="number" value={height} onChange={(e) => setHeight(Number(e.target.value))} min={1} />
                </div>
              )}
            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>


          <button className={`btn ${action === 'delete' ? 'btn-danger' : 'btn-primary'} btn-lg`} style={{ width: '100%', height: 48 }}
            onClick={handleProcess} disabled={processing || !inputPath || (action === 'copy' && !outputPath)}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 处理中...</> :
              <><Play style={{ width: 18, height: 18 }} /> {action === 'delete' ? '开始筛选并删除' : '开始筛选并输出'}</>}
          </button>

          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
