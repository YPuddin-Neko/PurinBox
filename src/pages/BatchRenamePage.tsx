import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  TextCursorInput,
  FolderOpen,
  Play,
  Loader2,
  Eye,
  Shuffle,
  ArrowRight,
  Hash,
  Type,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface PreviewItem { original: string; renamed: string; }

export default function BatchRenamePage() {
  const [inputPath, setInputPath] = useState('');
  const [prefix, setPrefix] = useState('img_');
  const [startNumber, setStartNumber] = useState(1);
  const [digitCount, setDigitCount] = useState(4);
  const [shuffleOrder, setShuffleOrder] = useState(false);
  const [previews, setPreviews] = useState<PreviewItem[]>([]);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('rename-progress', (event) => {
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

  const selectFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择图片文件夹' });
    if (selected) {
      setInputPath(selected as string);
      setPreviews([]);
    }
  };

  const handlePreview = async () => {
    if (!inputPath) return;
    setPreviewLoading(true);
    try {
      const result = await invoke<PreviewItem[]>('preview_rename', {
        options: { input_path: inputPath, prefix, start_number: startNumber, digit_count: digitCount, shuffle: shuffleOrder },
      });
      setPreviews(result);
    } catch (e: any) {
      setPreviews([]);
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `预览失败: ${String(e)}`, status: 'error' }]);
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleShuffle = async () => {
    if (!inputPath) return;
    setPreviewLoading(true);
    try {
      const result = await invoke<PreviewItem[]>('preview_rename', {
        options: { input_path: inputPath, prefix, start_number: startNumber, digit_count: digitCount, shuffle: true },
      });
      setPreviews(result);
      setShuffleOrder(true);
    } catch (e: any) {
      setPreviews([]);
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleExecute = async () => {
    if (!inputPath || previews.length === 0) return;
    setProcessing(true);
    setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setLogs([{ time: getTimeStr(), message: `开始重命名: 前缀="${prefix}", 起始=${startNumber}, 位数=${digitCount}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('execute_rename', {
        options: { input_path: inputPath, prefix, start_number: startNumber, digit_count: digitCount, shuffle: shuffleOrder },
      });
      setPreviews([]);
    } catch (e: any) {
      setLogs((prev) => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true); setIsDone(true);
    } finally {
      setProcessing(false);
    }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); }, []);

  // Example preview string
  const exampleNum = String(startNumber).padStart(digitCount, '0');
  const exampleName = `${prefix}${exampleNum}.png`;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <TextCursorInput style={{ width: 28, height: 28, color: '#38bdf8' }} />
          <h1 className="page-title">图片批量重命名</h1>
        </div>
        <p className="page-subtitle">按照规则批量重命名图片文件，支持自定义前缀、编号和打乱顺序</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 400px', gap: 'var(--space-6)' }}>
        {/* 左侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">图片文件夹</span></div>
            <div className="form-group">
              <label className="form-label">选择图片所在文件夹（将直接在原位重命名）</label>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <input className="form-input" placeholder="选择文件夹..." value={inputPath} onChange={(e) => setInputPath(e.target.value)} style={{ flex: 1 }} />
                <button className="btn btn-secondary" onClick={selectFolder}><FolderOpen style={{ width: 16, height: 16 }} /></button>
              </div>
            </div>
          </div>

          {/* 命名规则 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">命名规则</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              <div className="form-group">
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Type style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                  图片前缀
                </label>
                <input className="form-input" value={prefix} onChange={(e) => setPrefix(e.target.value)} placeholder="例如: img_" />
              </div>
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                    起始编号
                  </label>
                  <input className="form-input" type="number" value={startNumber} onChange={(e) => setStartNumber(Number(e.target.value))} min={0} />
                </div>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                    编号位数
                  </label>
                  <input className="form-input" type="number" value={digitCount} onChange={(e) => setDigitCount(Number(e.target.value))} min={1} max={10} />
                </div>
              </div>

              {/* 命名示例 */}
              <div style={{
                padding: 'var(--space-3) var(--space-4)',
                borderRadius: 'var(--radius-md)',
                background: 'rgba(56, 189, 248, 0.06)',
                border: '1px solid rgba(56, 189, 248, 0.12)',
                display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
              }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>命名示例:</span>
                <span style={{ fontSize: 'var(--font-size-md)', fontWeight: 700, color: '#38bdf8', fontFamily: 'monospace' }}>{exampleName}</span>
              </div>
            </div>
          </div>

          {/* 操作按钮 */}
          <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
            <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={handlePreview} disabled={!inputPath || previewLoading}>
              {previewLoading ? <Loader2 style={{ width: 16, height: 16, animation: 'spin 1s linear infinite' }} /> : <Eye style={{ width: 16, height: 16 }} />}
              生成预览
            </button>
            <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={handleShuffle} disabled={!inputPath || previewLoading}>
              <Shuffle style={{ width: 16, height: 16 }} />
              打乱并预览
            </button>
          </div>

          {/* 预览表格 */}
          {previews.length > 0 && (
            <div className="tool-panel" style={{ padding: 0, overflow: 'hidden' }}>
              <div className="tool-panel-header" style={{ padding: 'var(--space-3) var(--space-4)' }}>
                <span className="tool-panel-title">重命名预览</span>
                <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{previews.length} 个文件</span>
              </div>
              <div style={{ maxHeight: 360, overflowY: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                  <thead>
                    <tr style={{ borderBottom: '1px solid var(--color-border)' }}>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>#</th>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>原文件名</th>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'center', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', width: 30 }}></th>
                      <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>新文件名</th>
                    </tr>
                  </thead>
                  <tbody>
                    {previews.map((item, idx) => (
                      <tr key={idx} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                        <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>{idx + 1}</td>
                        <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', fontFamily: 'monospace', wordBreak: 'break-all' }}>{item.original}</td>
                        <td style={{ padding: '6px 0', textAlign: 'center' }}><ArrowRight style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /></td>
                        <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: '#38bdf8', fontFamily: 'monospace', fontWeight: 600 }}>{item.renamed}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>前缀</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: '#38bdf8', fontFamily: 'monospace' }}>{prefix || '(无)'}</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>起始编号</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: 'var(--color-text-primary)' }}>{startNumber}</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>编号位数</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: 'var(--color-text-primary)' }}>{digitCount} 位</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>打乱顺序</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: shuffleOrder ? '#fbbf24' : 'var(--color-text-tertiary)' }}>{shuffleOrder ? '是' : '否'}</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>预览数量</span>
                <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: 'var(--color-text-primary)' }}>{previews.length} 个</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>文件夹</span>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', maxWidth: 220, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', direction: 'rtl' }}>{inputPath || '未设置'}</span>
              </div>
            </div>
          </div>

          <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleExecute}
            disabled={processing || !inputPath || previews.length === 0}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 重命名中...</> : <><Play style={{ width: 18, height: 18 }} /> 执行重命名</>}
          </button>

          {previews.length === 0 && !processing && (
            <div style={{
              padding: 'var(--space-3) var(--space-4)',
              borderRadius: 'var(--radius-md)',
              background: 'rgba(56, 189, 248, 0.04)',
              border: '1px solid rgba(56, 189, 248, 0.1)',
              fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', lineHeight: 1.6,
            }}>
              💡 请先点击「生成预览」查看重命名结果，确认无误后再执行重命名。重命名将直接修改原文件名。
            </div>
          )}

          {(logs.length > 0 || processing) && (
            <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} />
          )}
        </div>
      </div>
    </div>
  );
}
