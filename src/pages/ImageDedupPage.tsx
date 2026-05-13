import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  FolderOpen, Trash2, Check, X, Copy, Search, ChevronLeft, ChevronRight,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from '../components/ProgressLog';
import ProcessButton from '../components/ProcessButton';

interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface DupGroup { paths: string[]; similarity: number; method: string; }
interface DedupResult { total_images: number; duplicate_groups: DupGroup[]; scan_time_ms: number; }

export default function ImageDedupPage() {
  const [inputPath, setInputPath] = useState('');
  const [dhashThreshold, setDhashThreshold] = useState(10);
  const [phashThreshold, setPhashThreshold] = useState(10);
  const [colorThreshold, setColorThreshold] = useState(0.85);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressCurrent, setProgressCurrent] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [processStartTime, setProcessStartTime] = useState(0);

  // results
  const [dupGroups, setDupGroups] = useState<DupGroup[]>([]);
  const [selectedForDelete, setSelectedForDelete] = useState<Set<string>>(new Set());
  const [totalImages, setTotalImages] = useState(0);
  const [deleting, setDeleting] = useState(false);
  const [currentPage, setCurrentPage] = useState(0);
  const GROUPS_PER_PAGE = 9;

  // lightbox state: which group + which image index
  const [lightbox, setLightbox] = useState<{ groupIdx: number; imgIdx: number } | null>(null);

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('dedup_progress', (event) => {
      if (!active) return;
      const d = event.payload;
      setProgressCurrent(d.current);
      setProgressTotal(d.total);
      if (d.total > 0) setProgress(Math.round((d.current / d.total) * 100));
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs(prev => [...prev, {
          time: getTimeStr(),
          message: d.message,
          status: d.status === 'done' ? 'info' : d.status as LogEntry['status'],
        }]);
      }
    });
    return () => { active = false; p.then(u => u()); };
  }, []);

  const pickFolder = useCallback(async () => {
    const selected = await open({ directory: true, title: '选择图片文件夹' });
    if (selected) setInputPath(selected as string);
  }, []);

  const handleStart = useCallback(async () => {
    if (!inputPath) return;
    setProcessing(true); setProgress(0); setProgressCurrent(0); setProgressTotal(0);
    setIsDone(false); setHasError(false);
    setProcessStartTime(Date.now());
    setDupGroups([]); setSelectedForDelete(new Set()); setTotalImages(0);
    setCurrentPage(0);
    setLogs([{ time: getTimeStr(), message: '开始扫描图片...', status: 'info' }]);
    try {
      const result = await invoke<DedupResult>('start_image_dedup', {
        options: {
          folder_path: inputPath,
          dhash_threshold: dhashThreshold,
          phash_threshold: phashThreshold,
          color_threshold: colorThreshold,
        },
      });
      setDupGroups(result.duplicate_groups);
      setTotalImages(result.total_images);
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: `扫描完成: ${result.total_images} 张图片, 发现 ${result.duplicate_groups.length} 组重复, 耗时 ${(result.scan_time_ms / 1000).toFixed(1)}s`,
        status: 'success',
      }]);
      // auto-select all but first in each group for deletion
      const autoSelect = new Set<string>();
      result.duplicate_groups.forEach(g => {
        g.paths.slice(1).forEach(p => autoSelect.add(p));
      });
      setSelectedForDelete(autoSelect);
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasError(true);
    } finally {
      setIsDone(true);
      setProcessing(false);
    }
  }, [inputPath, dhashThreshold, phashThreshold, colorThreshold]);

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); setProcessStartTime(0); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  const toggleSelect = (path: string) => {
    setSelectedForDelete(prev => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
  };

  const handleDelete = useCallback(async () => {
    if (selectedForDelete.size === 0) return;
    setDeleting(true);
    try {
      const result = await invoke<{ deleted: number; failed: number; errors: string[] }>('delete_dedup_files', {
        paths: Array.from(selectedForDelete),
      });
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: `删除完成: 成功 ${result.deleted}, 失败 ${result.failed}`,
        status: result.failed > 0 ? 'warning' : 'success',
      }]);
      if (result.errors.length > 0) {
        result.errors.forEach(err => {
          setLogs(prev => [...prev, { time: getTimeStr(), message: err, status: 'error' }]);
        });
      }
      // Remove deleted paths from groups
      const deletedSet = new Set(Array.from(selectedForDelete).filter(p => !result.errors.some(e => e.startsWith(p))));
      setDupGroups(prev => prev
        .map(g => ({ ...g, paths: g.paths.filter(p => !deletedSet.has(p)) }))
        .filter(g => g.paths.length > 1)
      );
      setSelectedForDelete(new Set());
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: `删除失败: ${String(e)}`, status: 'error' }]);
    } finally {
      setDeleting(false);
    }
  }, [selectedForDelete]);

  // styles
  const panel: React.CSSProperties = { background: 'var(--color-bg-card)', border: '1px solid var(--color-border)', borderRadius: 'var(--radius-lg)', padding: 20 };
  const label: React.CSSProperties = { fontSize: 12, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 6, display: 'block' };
  const slider: React.CSSProperties = { width: '100%', accentColor: '#7c5cfc' };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Copy style={{ width: 28, height: 28, color: '#14b8a6' }} />
          <h1 className="page-title">图片去重</h1>
        </div>
        <p className="page-subtitle">使用 dHash + pHash + 颜色直方图筛选重复图片</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gap: 20, minHeight: 'calc(100vh - 200px)' }}>
        {/* Left: settings */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div style={panel}>
            <label style={label}>图片文件夹</label>
            <div style={{ display: 'flex', gap: 6 }}>
              <input className="form-input" value={inputPath} onChange={e => setInputPath(e.target.value)}
                placeholder="选择文件夹..." style={{ flex: 1, fontSize: 12 }} />
              <button className="btn btn-secondary" onClick={pickFolder} style={{ flexShrink: 0 }}>
                <FolderOpen style={{ width: 14, height: 14 }} />
              </button>
            </div>
          </div>

          <div style={panel}>
            <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)', marginBottom: 12 }}>
              算法参数
            </div>

            <div style={{ marginBottom: 16 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
                <span style={label}>dHash 阈值</span>
                <span style={{ fontSize: 13, fontWeight: 700, color: '#7c5cfc', fontFamily: 'monospace' }}>{dhashThreshold}</span>
              </div>
              <input type="range" min={1} max={20} value={dhashThreshold}
                onChange={e => setDhashThreshold(Number(e.target.value))} style={slider} />
              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                <span>严格 (1)</span><span>宽松 (20)</span>
              </div>
            </div>

            <div style={{ marginBottom: 16 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
                <span style={label}>pHash 阈值</span>
                <span style={{ fontSize: 13, fontWeight: 700, color: '#7c5cfc', fontFamily: 'monospace' }}>{phashThreshold}</span>
              </div>
              <input type="range" min={1} max={20} value={phashThreshold}
                onChange={e => setPhashThreshold(Number(e.target.value))} style={slider} />
              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                <span>严格 (1)</span><span>宽松 (20)</span>
              </div>
            </div>

            <div style={{ marginBottom: 4 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
                <span style={label}>颜色阈值</span>
                <span style={{ fontSize: 13, fontWeight: 700, color: '#7c5cfc', fontFamily: 'monospace' }}>{colorThreshold.toFixed(2)}</span>
              </div>
              <input type="range" min={0} max={100} value={Math.round(colorThreshold * 100)}
                onChange={e => setColorThreshold(Number(e.target.value) / 100)} style={slider} />
              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                <span>宽松 (0)</span><span>严格 (1.0)</span>
              </div>
            </div>
          </div>

          <ProcessButton processing={processing} onStart={handleStart}
            disabled={!inputPath}
            cancelCommand="cancel_image_dedup"
            startText="执行去重扫描" processingText="扫描中..."
            onCancelLog={addCancelLog} />

          <ProgressLog progress={progress} current={progressCurrent} total={progressTotal} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} externalStartTime={processStartTime} />
        </div>
        {/* Right: results */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minHeight: 300, overflow: 'hidden' }}>
          {/* stats bar */}
          {isDone && dupGroups.length > 0 && (
            <div style={{ display: 'flex', gap: 10, flexShrink: 0 }}>
              {[
                { label: '总图片', value: totalImages, color: '#60a5fa' },
                { label: '重复组', value: dupGroups.length, color: '#f59e0b' },
                { label: '重复图片', value: dupGroups.reduce((a, g) => a + g.paths.length - 1, 0), color: '#ef4444' },
                { label: '已选删除', value: selectedForDelete.size, color: '#7c5cfc' },
              ].map(s => (
                <div key={s.label} style={{ ...panel, flex: 1, padding: '10px 14px', display: 'flex', alignItems: 'center', gap: 10 }}>
                  <span style={{ fontSize: 18, fontWeight: 800, color: s.color, fontFamily: 'monospace' }}>{s.value}</span>
                  <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{s.label}</span>
                </div>
              ))}
            </div>
          )}

          {/* empty state */}
          {dupGroups.length === 0 && (
            <div style={{ ...panel, flex: 1, minHeight: 200, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, color: 'var(--color-text-tertiary)' }}>
              <Search style={{ width: 48, height: 48, opacity: 0.15 }} />
              <span style={{ fontSize: 13 }}>{processing ? '正在扫描中...' : isDone ? '未发现重复图片' : '设置参数后点击执行扫描'}</span>
            </div>
          )}

          {/* paginated groups grid (3 cols, max 3 rows = 9 per page) */}
          {dupGroups.length > 0 && (() => {
            const totalPages = Math.ceil(dupGroups.length / GROUPS_PER_PAGE);
            const pageGroups = dupGroups.slice(currentPage * GROUPS_PER_PAGE, (currentPage + 1) * GROUPS_PER_PAGE);
            return (
              <>
                <div style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
                  <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10, alignItems: 'start' }}>
                    {pageGroups.map((group, localIdx) => {
                      const gi = currentPage * GROUPS_PER_PAGE + localIdx;
                      return (
                        <div key={gi} style={{ ...panel, padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                          {/* group header */}
                          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                              <span style={{ fontSize: 9, fontWeight: 700, color: '#fff', background: '#ef4444', borderRadius: 4, padding: '1px 6px' }}>
                                第 {gi + 1} 组
                              </span>
                              <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>
                                {group.paths.length} 张
                              </span>
                            </div>
                            <button className="btn btn-ghost" style={{ fontSize: 9, padding: '1px 6px', height: 'auto' }}
                              onClick={() => {
                                const allButFirst = group.paths.slice(1);
                                setSelectedForDelete(prev => {
                                  const next = new Set(prev);
                                  const allSelected = allButFirst.every(p => next.has(p));
                                  if (allSelected) allButFirst.forEach(p => next.delete(p));
                                  else allButFirst.forEach(p => next.add(p));
                                  return next;
                                });
                              }}>
                              全选/取消
                            </button>
                          </div>
                          {/* images grid */}
                          <div style={{ display: 'grid', gridTemplateColumns: `repeat(${Math.min(3, group.paths.length)}, 1fr)`, gap: 4 }}>
                            {group.paths.map((p, pi) => {
                              const isSelected = selectedForDelete.has(p);
                              const fname = p.split('/').pop() || p.split('\\').pop() || p;
                              return (
                                <div key={pi} style={{
                                  position: 'relative', borderRadius: 6, overflow: 'hidden',
                                  border: `2px solid ${isSelected ? '#ef4444' : 'var(--color-border)'}`,
                                  opacity: isSelected ? 0.55 : 1,
                                  transition: 'all 0.15s', cursor: 'pointer',
                                }}>
                                  <div style={{ aspectRatio: '1', background: 'rgba(0,0,0,0.1)' }}
                                    onClick={() => setLightbox({ groupIdx: gi, imgIdx: pi })}>
                                    <img src={convertFileSrc(p)} alt={fname} loading="lazy"
                                      style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                                  </div>
                                  {/* select/deselect badge */}
                                  <div onClick={(e) => { e.stopPropagation(); toggleSelect(p); }} style={{
                                    position: 'absolute', top: 4, right: 4, width: 20, height: 20, borderRadius: '50%',
                                    background: isSelected ? '#ef4444' : 'rgba(0,0,0,0.5)',
                                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                                    transition: 'all 0.15s',
                                    border: '2px solid rgba(255,255,255,0.8)',
                                  }}>
                                    {isSelected ? <Trash2 style={{ width: 8, height: 8, color: '#fff' }} /> : <Check style={{ width: 8, height: 8, color: '#fff', opacity: 0.5 }} />}
                                  </div>
                                  {pi === 0 && (
                                    <div style={{
                                      position: 'absolute', top: 4, left: 4, fontSize: 7, fontWeight: 700,
                                      color: '#fff', background: '#22c55e', borderRadius: 3, padding: '1px 4px',
                                    }}>
                                      保留
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                          {/* method info */}
                          <div style={{ fontSize: 8, color: 'var(--color-text-tertiary)', textAlign: 'center', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                            {group.method}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>

                {/* pagination + actions */}
                <div style={{ flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '4px 0' }}>
                  {/* pagination */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 30 }}
                      disabled={currentPage === 0} onClick={() => setCurrentPage(p => p - 1)}>
                      <ChevronLeft style={{ width: 14, height: 14 }} />
                    </button>
                    <span style={{ fontSize: 12, color: 'var(--color-text-secondary)', fontWeight: 600, minWidth: 80, textAlign: 'center' }}>
                      {currentPage + 1} / {totalPages}
                    </span>
                    <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 30 }}
                      disabled={currentPage >= totalPages - 1} onClick={() => setCurrentPage(p => p + 1)}>
                      <ChevronRight style={{ width: 14, height: 14 }} />
                    </button>
                    <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>
                      共 {dupGroups.length} 组
                    </span>
                  </div>
                  {/* delete actions */}
                  <div style={{ display: 'flex', gap: 8 }}>
                    <button className="btn btn-secondary" onClick={() => setSelectedForDelete(new Set())}
                      style={{ fontSize: 10, height: 30, padding: '0 10px' }}>
                      <X style={{ width: 10, height: 10 }} /> 清除选择
                    </button>
                    <button className="btn btn-danger" onClick={handleDelete} disabled={selectedForDelete.size === 0 || deleting}
                      style={{ fontSize: 10, height: 30, padding: '0 10px' }}>
                      <Trash2 style={{ width: 10, height: 10 }} /> 删除选中 ({selectedForDelete.size})
                    </button>
                  </div>
                </div>
              </>
            );
          })()}
        </div>
      </div>

      {/* Lightbox */}
      {lightbox && dupGroups[lightbox.groupIdx] && (() => {
        const group = dupGroups[lightbox.groupIdx];
        const idx = lightbox.imgIdx;
        const p = group.paths[idx];
        const fname = p.split('/').pop() || p.split('\\').pop() || p;
        const isSelected = selectedForDelete.has(p);
        return (
          <div onClick={() => setLightbox(null)} style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            background: 'rgba(0,0,0,0.85)', backdropFilter: 'blur(8px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            animation: 'fadeIn 0.15s ease',
          }}>
            {/* prev */}
            <button onClick={e => { e.stopPropagation(); setLightbox({ ...lightbox, imgIdx: idx - 1 }); }}
              disabled={idx === 0}
              style={{
                position: 'absolute', left: 20, top: '50%', transform: 'translateY(-50%)',
                width: 44, height: 44, borderRadius: '50%', border: 'none',
                background: 'rgba(255,255,255,0.1)', color: '#fff', cursor: idx === 0 ? 'default' : 'pointer',
                opacity: idx === 0 ? 0.3 : 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
                transition: 'all 0.15s',
              }}>
              <ChevronLeft style={{ width: 24, height: 24 }} />
            </button>
            {/* image */}
            <div onClick={e => e.stopPropagation()} style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 12, maxWidth: '80vw', maxHeight: '85vh' }}>
              <img src={convertFileSrc(p)} alt={fname}
                style={{ maxWidth: '80vw', maxHeight: '75vh', objectFit: 'contain', borderRadius: 8 }} />
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <span style={{ fontSize: 13, color: '#fff', fontWeight: 600 }}>{fname}</span>
                <span style={{ fontSize: 11, color: 'rgba(255,255,255,0.5)' }}>
                  第 {lightbox.groupIdx + 1} 组 · {idx + 1}/{group.paths.length}
                </span>
                {idx === 0 && (
                  <span style={{ fontSize: 10, fontWeight: 700, color: '#22c55e', background: 'rgba(34,197,94,0.15)', borderRadius: 4, padding: '1px 8px' }}>保留</span>
                )}
                <button onClick={() => toggleSelect(p)} style={{
                  padding: '3px 12px', borderRadius: 6, fontSize: 11, fontWeight: 600, border: 'none', cursor: 'pointer',
                  background: isSelected ? 'rgba(239,68,68,0.2)' : 'rgba(255,255,255,0.1)',
                  color: isSelected ? '#ef4444' : '#fff',
                  transition: 'all 0.15s',
                }}>
                  {isSelected ? '✓ 已选删除' : '选择删除'}
                </button>
              </div>
            </div>
            {/* next */}
            <button onClick={e => { e.stopPropagation(); setLightbox({ ...lightbox, imgIdx: idx + 1 }); }}
              disabled={idx >= group.paths.length - 1}
              style={{
                position: 'absolute', right: 20, top: '50%', transform: 'translateY(-50%)',
                width: 44, height: 44, borderRadius: '50%', border: 'none',
                background: 'rgba(255,255,255,0.1)', color: '#fff', cursor: idx >= group.paths.length - 1 ? 'default' : 'pointer',
                opacity: idx >= group.paths.length - 1 ? 0.3 : 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
                transition: 'all 0.15s',
              }}>
              <ChevronRight style={{ width: 24, height: 24 }} />
            </button>
            {/* close hint */}
            <div style={{ position: 'absolute', top: 20, right: 20, fontSize: 12, color: 'rgba(255,255,255,0.4)' }}>点击背景关闭</div>
          </div>
        );
      })()}
    </div>
  );
}
