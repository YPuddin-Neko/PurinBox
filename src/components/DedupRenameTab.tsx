import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowRight, Download, FolderOpen, Loader2, Play, RotateCcw, Search } from 'lucide-react';
import { useCallback, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useBatchTask } from '../hooks/useBatchTask';
import { ensureAssetScope } from '../utils/assetScope';
import HashThresholdFields from './HashThresholdFields';
import LightboxShell from './LightboxShell';
import ProcessButton from './ProcessButton';
import ProgressLog from './ProgressLog';
import Pager from './ui/Pager';

interface DedupPair { path_a: string; name_a: string; path_b: string; name_b: string; }
interface ScanResult { pairs: DedupPair[]; total_a: number; total_b: number; unmatched_a: string[]; unmatched_b: string[]; scan_time_ms: number; failed_files: string[]; }
// direction: 'a' = B uses A's name, 'b' = A uses B's name
type Direction = 'a' | 'b';

/** srcPath 改用 nameFrom 的主名并保留自己的扩展名；otherPath 是配对中的另一个文件 */
function buildRenameAction(srcPath: string, srcName: string, nameFrom: string, otherPath: string) {
  const stem = nameFrom.replace(/\.[^.]+$/, '');
  const ext = srcName.includes('.') ? srcName.replace(/^.*\./, '.') : '';
  const targetName = stem + ext;
  const targetPath = srcPath.substring(0, srcPath.length - srcName.length) + targetName;
  return {
    src_path: srcPath,
    target_name: targetName,
    conflict_path: targetPath === otherPath && targetPath !== srcPath ? otherPath : null,
  };
}

export default function DedupRenameTab() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'dedup-rename-progress', taskId: 'dedup-rename', logDone: false });
  const [scanFolders, setScanFolders] = useState({ a: '', b: '' });
  const [exportError, setExportError] = useState('');

  const [folderA, setFolderA] = useState('');
  const [folderB, setFolderB] = useState('');
  const [dhash, setDhash] = useState(10);
  const [phash, setPhash] = useState(10);
  const [colorTh, setColorTh] = useState(0.85);

  const [executing, setExecuting] = useState(false);

  const [pairs, setPairs] = useState<DedupPair[]>([]);
  const [directions, setDirections] = useState<Direction[]>([]);
  const [totalA, setTotalA] = useState(0);
  const [totalB, setTotalB] = useState(0);
  const [unmatchedA, setUnmatchedA] = useState<string[]>([]);
  const [unmatchedB, setUnmatchedB] = useState<string[]>([]);
  const [unmatchModal, setUnmatchModal] = useState<'a' | 'b' | null>(null);
  const [lightbox, setLightbox] = useState<{ idx: number } | null>(null);
  const [pairPage, setPairPage] = useState(0);
  const PAIRS_PER_PAGE = 15;

  const pickFolder = useCallback(async (setter: (v: string) => void) => {
    const sel = await open({ directory: true, title: t('pages.selectInputTitle') });
    if (sel) setter(sel as string);
  }, [t]);

  const handleScan = async () => {
    setPairs([]); setDirections([]); setUnmatchedA([]); setUnmatchedB([]); setLightbox(null); setUnmatchModal(null);
    const result = await task.run({
      taskName: t('dedupRename.tabDedup'), startLog: t('dedupRename.scanStart'), exec: async () => {
        await ensureAssetScope(folderA); await ensureAssetScope(folderB);
        return invoke<ScanResult>('scan_dedup_rename', { options: { folder_a: folderA, folder_b: folderB, dhash_threshold: dhash, phash_threshold: phash, color_threshold: colorTh } });
      }
    });
    if (!result) return;
    setPairs(result.pairs); setDirections(result.pairs.map(() => 'a')); setPairPage(0);
    setTotalA(result.total_a); setTotalB(result.total_b); setUnmatchedA(result.unmatched_a); setUnmatchedB(result.unmatched_b);
    setScanFolders({ a: folderA, b: folderB });
    result.failed_files.forEach(file => task.logger.appendLog(file, 'warning'));
    task.logger.appendLog(t('dedupRename.scanDone', { a: result.total_a, b: result.total_b, pairs: result.pairs.length, time: (result.scan_time_ms / 1000).toFixed(1) }), 'success');
  };

  const toggleDirection = (idx: number) => {
    setDirections(prev => { const n = [...prev]; n[idx] = n[idx] === 'a' ? 'b' : 'a'; return n; });
  };
  const setAllDirection = (d: Direction) => setDirections(prev => prev.map(() => d));

  const handleExecute = async () => {
    if (!pairs.length || task.processing) return;
    setExecuting(true);
    try {
      const result = await task.run({
        taskName: t('dedupRename.tabDedup'), startLog: t('dedupRename.execStart', { count: pairs.length }), keepLogs: true, cancellable: false, exec: () => {
          const actions = pairs.map((p, i) => directions[i] === 'a'
            ? buildRenameAction(p.path_b, p.name_b, p.name_a, p.path_a)
            : buildRenameAction(p.path_a, p.name_a, p.name_b, p.path_b));
          return invoke<{ success_count: number; fail_count: number; errors: string[] }>('execute_dedup_rename', { actions });
        }
      });
      if (!result) return;
      task.logger.appendLog(t('dedupRename.execDone', { ok: result.success_count, fail: result.fail_count }), result.fail_count ? 'warning' : 'success');
      if (result.errors.length) task.logger.appendLog(result.errors.join('\n'), 'error');
      setPairs([]); setLightbox(null);
    } finally { setExecuting(false); }
  };

  const panel: React.CSSProperties = { background: 'var(--color-bg-card)', border: '1px solid var(--color-border)', borderRadius: 'var(--radius-lg)', padding: 20 };
  const label: React.CSSProperties = { fontSize: 12, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 6, display: 'block' };

  return (
    // 网格占满宿主页剩余高度：配对列表在右栏内滚动，翻页栏始终可见
    <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gridTemplateRows: 'minmax(0, 1fr)', gap: 20, flex: 1, minHeight: 420 }}>
      {/* Left: settings */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minHeight: 0, overflowY: 'auto' }}>
        <div style={panel}>
          <label style={label}>{t('dedupRename.folderA')}</label>
          <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
            <input className="form-input" value={folderA} onChange={e => setFolderA(e.target.value)}
              placeholder={t('pages.selectInputFolder')} style={{ flex: 1, fontSize: 12 }} />
            <button className="btn btn-secondary" onClick={() => pickFolder(setFolderA)} style={{ flexShrink: 0 }}>
              <FolderOpen style={{ width: 14, height: 14 }} />
            </button>
          </div>
          <label style={label}>{t('dedupRename.folderB')}</label>
          <div style={{ display: 'flex', gap: 6 }}>
            <input className="form-input" value={folderB} onChange={e => setFolderB(e.target.value)}
              placeholder={t('pages.selectInputFolder')} style={{ flex: 1, fontSize: 12 }} />
            <button className="btn btn-secondary" onClick={() => pickFolder(setFolderB)} style={{ flexShrink: 0 }}>
              <FolderOpen style={{ width: 14, height: 14 }} />
            </button>
          </div>
        </div>

        <div style={panel}><div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)', marginBottom: 12 }}>{t('dedupRename.matchParams')}</div><HashThresholdFields dhash={dhash} onDhash={setDhash} phash={phash} onPhash={setPhash} color={colorTh} onColor={setColorTh} /></div>

        <ProcessButton {...task.buttonProps} onStart={handleScan}
          disabled={!folderA || !folderB || task.processing}
          cancelCommand="cancel_dedup_rename"
          startText={t('dedupRename.startScan')} processingText={t('dedupRename.scanning')}
        />

        <ProgressLog {...task.progressLogProps} />
      </div>

      {/* Right: results */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minHeight: 300, overflow: 'hidden' }}>
        {pairs.length === 0 ? (
          <div style={{ ...panel, flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, color: 'var(--color-text-tertiary)' }}>
            <Search style={{ width: 48, height: 48, opacity: 0.15 }} />
            <span style={{ fontSize: 13 }}>{task.processing && !executing ? t('dedupRename.scanning') : task.progressLogProps.isDone ? t('dedupRename.noMatch') : ''}</span>
          </div>
        ) : (
          <>
            {/* Stats bar */}
            <div style={{ display: 'flex', gap: 8, flexShrink: 0 }}>
              {[
                { label: t('dedupRename.totalA'), value: totalA, color: '#60a5fa', click: null },
                { label: t('dedupRename.totalB'), value: totalB, color: '#38bdf8', click: null },
                { label: t('dedupRename.matchCount'), value: pairs.length, color: '#4ade80', click: null },
                { label: t('dedupRename.unmatchA'), value: unmatchedA.length, color: '#f59e0b', click: unmatchedA.length > 0 ? () => setUnmatchModal('a') : null },
                { label: t('dedupRename.unmatchB'), value: unmatchedB.length, color: '#ef4444', click: unmatchedB.length > 0 ? () => setUnmatchModal('b') : null },
              ].map(s => (
                <div key={s.label}
                  onClick={s.click ?? undefined}
                  style={{
                    ...panel, flex: 1, padding: '8px 10px', display: 'flex', alignItems: 'center', gap: 6,
                    cursor: s.click ? 'pointer' : 'default',
                    transition: 'all 0.15s',
                    ...(s.click ? { border: `1px solid ${s.color}33` } : {}),
                  }}>
                  <span style={{ fontSize: 16, fontWeight: 800, color: s.color, fontFamily: 'monospace' }}>{s.value}</span>
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', fontWeight: 600, lineHeight: 1.2 }}>{s.label}</span>
                  {s.click && <span style={{ marginLeft: 'auto', fontSize: 9, color: s.color, opacity: 0.6 }}>↗</span>}
                </div>
              ))}
            </div>

            {/* Action bar */}
            <div style={{ display: 'flex', gap: 8, flexShrink: 0, alignItems: 'center' }}>
              <button className="btn btn-secondary" style={{ fontSize: 10, padding: '6px 10px' }}
                onClick={() => setAllDirection('a')}>
                {t('dedupRename.allUseA')}
              </button>
              <button className="btn btn-secondary" style={{ fontSize: 10, padding: '6px 10px' }}
                onClick={() => setAllDirection('b')}>
                {t('dedupRename.allUseB')}
              </button>
              <div style={{ flex: 1 }} />
              <button className="btn btn-primary" style={{ padding: '6px 20px', fontSize: 13 }}
                onClick={handleExecute} disabled={task.processing || pairs.length === 0}>
                {executing ? <><Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> {t('dedupRename.executing')}</> : <><Play style={{ width: 14, height: 14 }} /> {t('dedupRename.execute')}</>}
              </button>
            </div>

            {/* Pairs table */}
            <div style={{ ...panel, flex: 1, padding: 0, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
              <div style={{ padding: '10px 16px', borderBottom: '1px solid var(--color-border)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span style={{ fontSize: 13, fontWeight: 700 }}>{t('dedupRename.matchList')}</span>
              </div>
              <div style={{ flex: 1, overflowY: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                  <thead>
                    <tr style={{ borderBottom: '1px solid var(--color-border)' }}>
                      <th style={{ padding: '8px 12px', textAlign: 'left', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600, width: 30 }}>#</th>
                      <th style={{ padding: '8px 12px', textAlign: 'left', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('dedupRename.imageA')}</th>
                      <th style={{ padding: '8px 12px', textAlign: 'center', fontSize: 11, width: 80 }}>{t('dedupRename.direction')}</th>
                      <th style={{ padding: '8px 12px', textAlign: 'left', fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('dedupRename.imageB')}</th>
                      <th style={{ padding: '8px 12px', textAlign: 'center', fontSize: 11, color: 'var(--color-text-tertiary)', width: 60 }}>{t('dedupRename.preview')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {pairs.slice(pairPage * PAIRS_PER_PAGE, (pairPage + 1) * PAIRS_PER_PAGE).map((pair, localIdx) => {
                      const idx = pairPage * PAIRS_PER_PAGE + localIdx;
                      const dir = directions[idx];
                      const isASource = dir === 'a';
                      return (
                        <tr key={idx} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                          <td style={{ padding: '6px 12px', fontSize: 11, color: 'var(--color-text-tertiary)' }}>{idx + 1}</td>
                          <td style={{ padding: '6px 12px' }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                              <span style={{
                                fontSize: 12, fontFamily: 'monospace', fontWeight: isASource ? 700 : 400,
                                color: isASource ? '#4ade80' : 'var(--color-text-secondary)',
                                wordBreak: 'break-all',
                              }}>{pair.name_a}</span>
                              {isASource && <span style={{ fontSize: 8, padding: '1px 4px', borderRadius: 3, background: 'rgba(74,222,128,0.15)', color: '#4ade80', fontWeight: 700, whiteSpace: 'nowrap' }}>{t('dedupRename.source')}</span>}
                            </div>
                          </td>
                          <td style={{ padding: '6px 8px', textAlign: 'center' }}>
                            <button className="btn btn-ghost" onClick={() => toggleDirection(idx)}
                              style={{ padding: '2px 8px', height: 24, fontSize: 10, display: 'inline-flex', alignItems: 'center', gap: 4 }}
                              title={t('dedupRename.toggleDir')}>
                              {isASource ? <ArrowRight style={{ width: 12, height: 12 }} /> : <RotateCcw style={{ width: 10, height: 10 }} />}
                              {isASource ? 'A→B' : 'B→A'}
                            </button>
                          </td>
                          <td style={{ padding: '6px 12px' }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                              <span style={{
                                fontSize: 12, fontFamily: 'monospace', fontWeight: !isASource ? 700 : 400,
                                color: !isASource ? '#4ade80' : 'var(--color-text-secondary)',
                                wordBreak: 'break-all',
                              }}>{pair.name_b}</span>
                              {!isASource && <span style={{ fontSize: 8, padding: '1px 4px', borderRadius: 3, background: 'rgba(74,222,128,0.15)', color: '#4ade80', fontWeight: 700, whiteSpace: 'nowrap' }}>{t('dedupRename.source')}</span>}
                            </div>
                          </td>
                          <td style={{ padding: '6px 8px', textAlign: 'center' }}>
                            <button className="btn btn-ghost" onClick={() => setLightbox({ idx })}
                              style={{ padding: '2px 6px', height: 22, fontSize: 9 }}>
                              {t('dedupRename.view')}
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
              {/* Pagination */}
              {Math.ceil(pairs.length / PAIRS_PER_PAGE) > 1 && (
                <Pager footer page={pairPage} pages={Math.ceil(pairs.length / PAIRS_PER_PAGE)} onChange={setPairPage} />
              )}
            </div>
          </>
        )}
      </div>

      {/* Lightbox */}
      {lightbox && pairs[lightbox.idx] && createPortal((() => {
        const pair = pairs[lightbox.idx];
        const pathA = pair.path_a, pathB = pair.path_b;
        return (
          <LightboxShell onClose={() => setLightbox(null)}>
            <div onClick={e => e.stopPropagation()} style={{ display: 'flex', gap: 24, maxWidth: '90vw', maxHeight: '85vh' }}>
              {[{ path: pathA, name: pair.name_a, label: 'A' }, { path: pathB, name: pair.name_b, label: 'B' }].map(item => (
                <div key={item.label} style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8 }}>
                  <img src={convertFileSrc(item.path)} alt={item.name}
                    style={{ maxWidth: '40vw', maxHeight: '70vh', objectFit: 'contain', borderRadius: 8 }} />
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span style={{ fontSize: 10, fontWeight: 700, color: '#fff', background: '#7c5cfc', borderRadius: 4, padding: '1px 6px' }}>{item.label}</span>
                    <span style={{ fontSize: 12, color: '#fff', fontWeight: 600, fontFamily: 'monospace' }}>{item.name}</span>
                  </div>
                </div>
              ))}
            </div>

          </LightboxShell>
        );
      })(), document.body)}

      {/* Unmatched modal */}
      {unmatchModal && createPortal((() => {
        const isA = unmatchModal === 'a';
        const items = isA ? unmatchedA : unmatchedB;
        const title = isA ? t('dedupRename.unmatchATitle') : t('dedupRename.unmatchBTitle');
        const color = isA ? '#f59e0b' : '#ef4444';
        return (
          <div onClick={() => { setExportError(''); setUnmatchModal(null); }} style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            background: 'rgba(0,0,0,0.75)', backdropFilter: 'blur(6px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            animation: 'fadeIn 0.15s ease',
          }}>
            <div onClick={e => e.stopPropagation()} style={{
              background: 'var(--color-bg-card)', border: '1px solid var(--color-border)',
              borderRadius: 12, width: 420, maxHeight: '70vh', display: 'flex', flexDirection: 'column',
              boxShadow: '0 20px 60px rgba(0,0,0,0.5)',
            }}>
              <div style={{ padding: '16px 20px', borderBottom: '1px solid var(--color-border)', display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ fontSize: 16, fontWeight: 800, color, fontFamily: 'monospace' }}>{items.length}</span>
                <span style={{ fontSize: 14, fontWeight: 700, color: 'var(--color-text-primary)' }}>{title}</span>
                <div style={{ flex: 1 }} />
                <button className="btn btn-secondary" style={{ padding: '4px 12px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}
                  onClick={async () => {
                    const dest = await open({ directory: true, title: t('dedupRename.exportSelectDest') });
                    if (!dest) return;
                    try {
                      const sourceFolder = isA ? scanFolders.a : scanFolders.b;
                      if (sourceFolder.replace(/[\\/]+$/, '') === String(dest).replace(/[\\/]+$/, '')) {
                        const message = t('dedupRename.exportSameFolder');
                        setExportError(message); task.logger.appendLog(message, 'warning'); return;
                      }
                      setExportError('');
                      const result = await invoke<{ success_count: number; fail_count: number; errors: string[] }>('export_unmatched_files', {
                        sourceFolder, filenames: items, destFolder: dest as string,
                      });
                      task.logger.appendLog(t('dedupRename.exportDone', { success: result.success_count, fail: result.fail_count }), result.fail_count > 0 ? 'warning' : 'success');
                      setUnmatchModal(null);
                    } catch (e: any) {
                      task.logger.appendLog(`${t('dedupRename.exportFail')}: ${String(e)}`, 'error');
                    }
                  }}>
                  <Download style={{ width: 12, height: 12 }} />
                  {t('dedupRename.export')}
                </button>
                <button className="btn btn-ghost" onClick={() => setUnmatchModal(null)}
                  style={{ padding: '2px 8px', fontSize: 11 }}>✕</button>
              </div>
              <div style={{ flex: 1, overflowY: 'auto', padding: '8px 0' }}>
                {items.map((name, i) => (
                  <div key={i} style={{
                    padding: '6px 20px', fontSize: 12, fontFamily: 'monospace',
                    color: 'var(--color-text-secondary)',
                    borderBottom: '1px solid rgba(255,255,255,0.03)',
                    display: 'flex', alignItems: 'center', gap: 8,
                  }}>
                    <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums', minWidth: 24 }}>{i + 1}</span>
                    <span style={{ wordBreak: 'break-all' }}>{name}</span>
                  </div>
                ))}
              </div>
              {exportError && <div role="alert" style={{ padding: '10px 20px', color: 'var(--color-error)' }}>{exportError}</div>}
            </div>
          </div>
        );
      })(), document.body)}
    </div>
  );
}
