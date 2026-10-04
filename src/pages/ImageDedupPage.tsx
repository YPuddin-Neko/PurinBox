import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import {
  Check,
  ChevronLeft, ChevronRight,
  Copy,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { DedupDeleteResult, DedupOptions, DedupResult, DupGroup } from '../api/commandOptions';
import HashThresholdFields from '../components/HashThresholdFields';
import LightboxShell from '../components/LightboxShell';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import RecursiveScanToggle from '../components/RecursiveScanToggle';
import ThumbImage from '../components/ThumbImage';
import PageHeader from '../components/ui/PageHeader';
import Pager, { clampPage } from '../components/ui/Pager';
import PathInput from '../components/ui/PathInput';
import StatChips from '../components/ui/StatChips';
import { useBatchTask } from '../hooks/useBatchTask';
import { ensureAssetScope } from '../utils/assetScope';

export default function ImageDedupPage() {
  const { t } = useTranslation();
  const folderId = useId();
  const task = useBatchTask({ event: 'dedup_progress', logDone: false });
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [dhashThreshold, setDhashThreshold] = useState(10);
  const [phashThreshold, setPhashThreshold] = useState(10);
  const [colorThreshold, setColorThreshold] = useState(0.85);

  const [dupGroups, setDupGroups] = useState<DupGroup[]>([]);
  const [selectedForDelete, setSelectedForDelete] = useState<Set<string>>(new Set());
  const [totalImages, setTotalImages] = useState(0);
  const [deleting, setDeleting] = useState(false);
  const [currentPage, setCurrentPage] = useState(0);
  const GROUPS_PER_PAGE = 9;

  const [lightbox, setLightbox] = useState<{ groupIdx: number; imgIdx: number } | null>(null);

  const handleStart = async () => {
    setDupGroups([]); setSelectedForDelete(new Set()); setTotalImages(0); setCurrentPage(0); setLightbox(null);
    const result = await task.run({
      startLog: t('imageDedup.scanStart'),
      exec: async () => {
        await ensureAssetScope(inputPath);
        return invoke<DedupResult>('start_image_dedup', {
          options: {
            folder_path: inputPath, dhash_threshold: dhashThreshold, phash_threshold: phashThreshold, color_threshold: colorThreshold, recursive,
          } satisfies DedupOptions,
        });
      },
    });
    if (!result) return;
    setDupGroups(result.duplicate_groups); setTotalImages(result.total_images);
    result.failed_files.forEach(file => task.logger.appendLog(file, 'warning'));
    task.logger.appendLog(t('imageDedup.scanDone', { total: result.total_images, groups: result.duplicate_groups.length, time: (result.scan_time_ms / 1000).toFixed(1) }), 'success');
    setSelectedForDelete(new Set(result.duplicate_groups.flatMap(g => g.paths.slice(1))));
  };

  const toggleSelect = (path: string) => {
    setSelectedForDelete(prev => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
  };

  const handleDelete = async () => {
    if (selectedForDelete.size === 0 || task.processing || deleting) return;
    setDeleting(true);
    try {
      const result = await invoke<DedupDeleteResult>('delete_dedup_files', { paths: Array.from(selectedForDelete) });
      task.logger.appendLog(t('imageDedup.deleteDone', { ok: result.deleted, fail: result.failed }), result.failed > 0 ? 'warning' : 'success');
      if (result.errors.length) task.logger.appendLog(result.errors.join('\n'), 'error');
      const deletedSet = new Set(Array.from(selectedForDelete).filter(p => !result.errors.some(e => e.startsWith(p + ': '))));
      setDupGroups(prev => prev.map(g => ({ ...g, paths: g.paths.filter(p => !deletedSet.has(p)) })).filter(g => g.paths.length > 1));
      setSelectedForDelete(new Set()); setLightbox(null);
    } catch (error) { task.logger.appendCatchError(error, t('imageDedup.deleteFailed')); }
    finally { setDeleting(false); }
  };

  return (
    <div className="page" style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <PageHeader icon={Copy} color={'#14b8a6'} title={t('imageDedup.title')} subtitle={t('imageDedup.subtitle')} />

      {/* 网格占满页面剩余高度：结果区在右栏内滚动，翻页栏始终可见 */}
      <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gridTemplateRows: 'minmax(0, 1fr)', gap: 20, flex: 1, minHeight: 420 }}>
        {/* 左栏：设置 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minHeight: 0, overflowY: 'auto' }}>
          <div className="tool-panel">
            <div className="form-label-row" style={{ marginBottom: 6 }}>
              <label className="form-label" htmlFor={folderId}>{t('imageDedup.imageFolder')}</label>
              <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
            </div>
            <PathInput id={folderId} size="sm" value={inputPath} onChange={setInputPath}
              placeholder={t('imageDedup.selectFolderPlaceholder')} style={{ gap: 6 }} />
          </div>

          <div className="tool-panel">
            <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)', marginBottom: 12 }}>{t('imageDedup.algoParams')}</div>
            <HashThresholdFields dhash={dhashThreshold} onDhash={setDhashThreshold} phash={phashThreshold} onPhash={setPhashThreshold} color={colorThreshold} onColor={setColorThreshold} />
          </div>

          <ProcessButton {...task.buttonProps} onStart={handleStart}
            disabled={!inputPath || deleting}
            cancelCommand="cancel_image_dedup"
            startText={t('imageDedup.startScan')} processingText={t('imageDedup.scanning')} />

          <ProgressLog {...task.progressLogProps} />
        </div>
        {/* 右栏：结果 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minHeight: 300, overflow: 'hidden' }}>
          {task.progressLogProps.isDone && dupGroups.length > 0 && (
            <StatChips size="md" items={[
              { label: t('imageDedup.totalImages'), value: totalImages, color: '#60a5fa' },
              { label: t('imageDedup.dupGroups'), value: dupGroups.length, color: '#f59e0b' },
              { label: t('imageDedup.dupImages'), value: dupGroups.reduce((a, g) => a + g.paths.length - 1, 0), color: '#ef4444' },
              { label: t('imageDedup.selectedDel'), value: selectedForDelete.size, color: '#7c5cfc' },
            ]} />
          )}

          {/* 空状态 */}
          {dupGroups.length === 0 && (
            <div className="tool-panel" style={{ flex: 1, minHeight: 200, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, color: 'var(--color-text-tertiary)' }}>
              <Search style={{ width: 48, height: 48, opacity: 0.15 }} />
              <span style={{ fontSize: 13 }}>{task.processing ? t('imageDedup.emptyScanning') : task.progressLogProps.isDone ? t('imageDedup.emptyNoDup') : ''}</span>
            </div>
          )}

          {/* 分组网格：3 列，每页最多 3 行共 9 组 */}
          {dupGroups.length > 0 && (() => {
            const totalPages = Math.ceil(dupGroups.length / GROUPS_PER_PAGE);
            const page = clampPage(currentPage, totalPages);
            const pageGroups = dupGroups.slice(page * GROUPS_PER_PAGE, (page + 1) * GROUPS_PER_PAGE);
            return (
              <>
                <div style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
                  <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10, alignItems: 'start' }}>
                    {pageGroups.map((group, localIdx) => {
                      const gi = page * GROUPS_PER_PAGE + localIdx;
                      return (
                        <div key={gi} className="tool-panel" style={{ padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                          {/* 组标题 */}
                          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                              <span style={{ fontSize: 9, fontWeight: 700, color: '#fff', background: '#ef4444', borderRadius: 4, padding: '1px 6px' }}>
                                {t('imageDedup.groupN', { n: gi + 1 })}
                              </span>
                              <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>
                                {t('imageDedup.nImages', { n: group.paths.length })}
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
                              {t('imageDedup.toggleAll')}
                            </button>
                          </div>
                          {/* 组内图片 */}
                          <div style={{ display: 'grid', gridTemplateColumns: `repeat(${Math.min(3, group.paths.length)}, 1fr)`, gap: 4 }}>
                            {group.paths.map((p, pi) => {
                              const isSelected = selectedForDelete.has(p);
                              const fname = p.split(/[\\/]/).pop() || p;
                              return (
                                <div key={pi} style={{
                                  position: 'relative', borderRadius: 6, overflow: 'hidden',
                                  border: `2px solid ${isSelected ? '#ef4444' : 'var(--color-border)'}`,
                                  opacity: isSelected ? 0.55 : 1,
                                  transition: 'all 0.15s', cursor: 'pointer',
                                }}>
                                  <div style={{ aspectRatio: '1', background: 'rgba(0,0,0,0.1)' }}
                                    onClick={() => setLightbox({ groupIdx: gi, imgIdx: pi })}>
                                    <ThumbImage path={p} alt={fname}
                                      style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                                  </div>
                                  {/* 标记删除 */}
                                  <div onClick={() => toggleSelect(p)} style={{
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
                                      {t('imageDedup.keep')}
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                          {/* 判定方式 */}
                          <div style={{ fontSize: 8, color: 'var(--color-text-tertiary)', textAlign: 'center', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                            {group.method}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>

                {/* 翻页与删除操作 */}
                <div style={{ flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '4px 0' }}>
                  <Pager page={currentPage} pages={totalPages} onChange={setCurrentPage}><span>{t('imageDedup.totalGroups', { n: dupGroups.length })}</span></Pager>
                  <div style={{ display: 'flex', gap: 8 }}>
                    <button className="btn btn-secondary" onClick={() => setSelectedForDelete(new Set())}
                      style={{ fontSize: 10, height: 30, padding: '0 10px' }}>
                      <X style={{ width: 10, height: 10 }} /> {t('imageDedup.clearSelect')}
                    </button>
                    <button className="btn btn-danger" onClick={handleDelete} disabled={selectedForDelete.size === 0 || deleting || task.processing}
                      style={{ fontSize: 10, height: 30, padding: '0 10px' }}>
                      <Trash2 style={{ width: 10, height: 10 }} /> {t('imageDedup.deleteSelected')} ({selectedForDelete.size})
                    </button>
                  </div>
                </div>
              </>
            );
          })()}
        </div>
      </div>

      {/* 大图 */}
      {lightbox && dupGroups[lightbox.groupIdx] && (() => {
        const group = dupGroups[lightbox.groupIdx];
        const idx = lightbox.imgIdx;
        const p = group.paths[idx];
        const fname = p.split(/[\\/]/).pop() || p;
        const isSelected = selectedForDelete.has(p);
        return (
          <LightboxShell onClose={() => setLightbox(null)}>
            <button onClick={() => setLightbox({ ...lightbox, imgIdx: idx - 1 })}
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
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 12, maxWidth: '80vw', maxHeight: '85vh' }}>
              <img src={convertFileSrc(p)} alt={fname}
                style={{ maxWidth: '80vw', maxHeight: '75vh', objectFit: 'contain', borderRadius: 8 }} />
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <span style={{ fontSize: 13, color: '#fff', fontWeight: 600 }}>{fname}</span>
                <span style={{ fontSize: 11, color: 'rgba(255,255,255,0.5)' }}>
                  {t('imageDedup.groupOf', { g: lightbox.groupIdx + 1, i: idx + 1, t: group.paths.length })}
                </span>
                {idx === 0 && (
                  <span style={{ fontSize: 10, fontWeight: 700, color: '#22c55e', background: 'rgba(34,197,94,0.15)', borderRadius: 4, padding: '1px 8px' }}>{t('imageDedup.keep')}</span>
                )}
                <button onClick={() => toggleSelect(p)} style={{
                  padding: '3px 12px', borderRadius: 6, fontSize: 11, fontWeight: 600, border: 'none', cursor: 'pointer',
                  background: isSelected ? 'rgba(239,68,68,0.2)' : 'rgba(255,255,255,0.1)',
                  color: isSelected ? '#ef4444' : '#fff',
                  transition: 'all 0.15s',
                }}>
                  {isSelected ? t('imageDedup.markedDel') : t('imageDedup.selectDel')}
                </button>
              </div>
            </div>
            <button onClick={() => setLightbox({ ...lightbox, imgIdx: idx + 1 })}
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
          </LightboxShell>
        );
      })()}
    </div>
  );
}
