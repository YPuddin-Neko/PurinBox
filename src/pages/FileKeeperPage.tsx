import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FileCheck2, FolderOpen, Play, Loader2, Shield } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const allExtensions = [
  { ext: 'jpg', label: 'JPG', color: '#ffa647' },
  { ext: 'jpeg', label: 'JPEG', color: '#ffa647' },
  { ext: 'png', label: 'PNG', color: '#4ade80' },
  { ext: 'webp', label: 'WebP', color: '#60a5fa' },
  { ext: 'bmp', label: 'BMP', color: '#f87171' },
  { ext: 'npz', label: 'NPZ', color: '#c084fc' },
  { ext: 'txt', label: 'TXT', color: '#fbbf24' },
];

export default function FileKeeperPage() {
  const [folderPath, setFolderPath] = useState('');
  const [keepExts, setKeepExts] = useState<Set<string>>(new Set(['jpg', 'jpeg', 'png', 'webp', 'txt']));
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('keeper-progress', (event) => {
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

  const toggleExt = (ext: string) => {
    setKeepExts((prev) => {
      const next = new Set(prev);
      if (next.has(ext)) next.delete(ext); else next.add(ext);
      return next;
    });
  };

  const selectAll = () => setKeepExts(new Set(allExtensions.map((e) => e.ext)));
  const deselectAll = () => setKeepExts(new Set());

  const selectFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择文件夹' });
    if (selected) setFolderPath(selected as string);
  };

  const handleProcess = async () => {
    if (!folderPath || keepExts.size === 0) return;
    setProcessing(true);
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: `开始处理, 保留: ${Array.from(keepExts).join(', ')}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('keep_specified_files', {
        options: { folder_path: folderPath, keep_extensions: Array.from(keepExts) },
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
          <FileCheck2 style={{ width: 28, height: 28, color: '#fbbf24' }} />
          <h1 className="page-title">保留指定文件</h1>
        </div>
        <p className="page-subtitle">勾选需要保留的文件类型，删除文件夹中其他所有文件</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">文件夹路径</span></div>
            <div className="form-group">
              <label className="form-label">选择要处理的文件夹</label>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <input className="form-input" placeholder="选择文件夹..." value={folderPath} onChange={(e) => setFolderPath(e.target.value)} style={{ flex: 1 }} />
                <button className="btn btn-secondary" onClick={selectFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
              </div>
            </div>
          </div>

          {/* 文件类型选择 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">保留的文件类型</span>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <button className="btn btn-ghost btn-sm" onClick={selectAll}>全选</button>
                <button className="btn btn-ghost btn-sm" onClick={deselectAll}>取消全选</button>
              </div>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: 'var(--space-3)' }}>
              {allExtensions.map((item) => {
                const checked = keepExts.has(item.ext);
                return (
                  <div key={item.ext} onClick={() => toggleExt(item.ext)} style={{
                    display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                    padding: 'var(--space-3) var(--space-4)',
                    borderRadius: 'var(--radius-md)',
                    border: `1px solid ${checked ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                    background: checked ? 'rgba(124, 92, 252, 0.06)' : 'var(--color-bg-input)',
                    cursor: 'pointer', transition: 'all 0.2s',
                  }}>
                    <div style={{
                      width: 18, height: 18, borderRadius: 4, minWidth: 18,
                      border: `2px solid ${checked ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`,
                      background: checked ? 'var(--color-accent-primary)' : 'transparent',
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      transition: 'all 0.15s',
                    }}>
                      {checked && <span style={{ color: 'white', fontSize: 11, fontWeight: 700 }}>✓</span>}
                    </div>
                    <span style={{
                      fontSize: 'var(--font-size-md)', fontWeight: 700,
                      color: checked ? item.color : 'var(--color-text-tertiary)',
                    }}>
                      .{item.ext}
                    </span>
                  </div>
                );
              })}
            </div>
          </div>

          {/* 说明 */}
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(251, 191, 36, 0.04)', border: '1px solid rgba(251, 191, 36, 0.1)' }}>
            <Shield style={{ width: 18, height: 18, color: '#fbbf24', marginTop: 2, minWidth: 18 }} />
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
              <strong style={{ color: '#f87171' }}>注意：</strong>此操作将<strong>永久删除</strong>文件夹中未被勾选类型的所有文件。请确保已备份重要数据。操作仅处理文件夹第一层目录，不递归子文件夹。
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>

          <button className="btn btn-danger btn-lg" style={{ width: '100%', height: 48 }} onClick={handleProcess} disabled={processing || !folderPath || keepExts.size === 0}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 处理中...</> : <><Play style={{ width: 18, height: 18 }} /> 删除其他文件</>}
          </button>

          
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
        </div>
      </div>
    </div>
  );
}
