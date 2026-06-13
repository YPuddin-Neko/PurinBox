import { useState, useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { convertFileSrc } from '@tauri-apps/api/core';
import { FolderOpen, ArrowRight, Play, Loader2, Search, RotateCcw, ChevronLeft, ChevronRight, Download } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';
import ProcessButton from './ProcessButton';
import { useTaskQueue } from './TaskContext';
import { useTranslation } from 'react-i18next';

interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface DedupPair { path_a: string; name_a: string; path_b: string; name_b: string; similarity: number; method: string; }
interface ScanResult { pairs: DedupPair[]; total_a: number; total_b: number; unmatched_a: string[]; unmatched_b: string[]; scan_time_ms: number; failed_files?: string[]; }
// direction: 'a' = B uses A's name, 'b' = A uses B's name
type Direction = 'a' | 'b';

export default function DedupRenameTab() {
  const { t } = useTranslation();
  const { addTask, updateTask } = useTaskQueue();
  const [folderA, setFolderA] = useState('');
  const [folderB, setFolderB] = useState('');
  const [dhash, setDhash] = useState(10);
  const [phash, setPhash] = useState(10);
  const [colorTh, setColorTh] = useState(0.85);
  const [scanning, setScanning] = useState(false);
  const [executing, setExecuting] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [startTime, setStartTime] = useState(0);
  const [pairs, setPairs] = useState<DedupPair[]>([]);
  const [directions, setDirections] = useState<Direction[]>([]);
  const [totalA, setTotalA] = useState(0);
  const [totalB, setTotalB] = useState(0);
  const [unmatchedA, setUnmatchedA] = useState<string[]>([]);
  const [unmatchedB, setUnmatchedB] = useState<string[]>([]);
  const [unmatchModal, setUnmatchModal] = useState<'a' | 'b' | null>(null);
  const [lightbox, setLightbox] = useState<{ idx: number; side: 'a' | 'b' } | null>(null);
  const [pairPage, setPairPage] = useState(0);
  const PAIRS_PER_PAGE = 15;

  useEffect(() => {
    let active = true;
    const p = listen<ProgressPayload>('dedup-rename-progress', (e) => {
      if (!active) return;
      const d = e.payload;
      setPCur(d.current); setPTot(d.total);
      if (d.total > 0) setProgress(Math.round((d.current / d.total) * 100));
      if (d.status === 'done') setIsDone(true);
      if (d.status === 'error') setHasError(true);
      if (d.status !== 'processing') {
        setLogs(prev => [...prev, { time: getTimeStr(), message: d.message, status: d.status === 'done' ? 'info' : d.status as LogEntry['status'] }]);
      }
    });
    return () => { active = false; p.then(u => u()); };
  }, []);

  const pickFolder = useCallback(async (setter: (v: string) => void) => {
    const sel = await open({ directory: true, title: t('pages.selectInputTitle') });
    if (sel) setter(sel as string);
  }, [t]);

  const handleScan = useCallback(async () => {
    if (!folderA || !folderB) return;
    setScanning(true); setProgress(0); setPCur(0); setPTot(0);
    setIsDone(false); setHasError(false); setStartTime(Date.now());
    setPairs([]); setDirections([]);
    setLogs([{ time: getTimeStr(), message: t('dedupRename.scanStart'), status: 'info' }]);
    addTask('dedup-rename', t('dedupRename.tabDedup'));
    try {
      const result = await invoke<ScanResult>('scan_dedup_rename', {
        options: { folder_a: folderA, folder_b: folderB, dhash_threshold: dhash, phash_threshold: phash, color_threshold: colorTh },
      });
      setPairs(result.pairs);
      setDirections(result.pairs.map(() => 'a' as Direction));
      setPairPage(0);
      setTotalA(result.total_a);
      setTotalB(result.total_b);
      setUnmatchedA(result.unmatched_a);
      setUnmatchedB(result.unmatched_b); // default: B uses A's name
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: t('dedupRename.scanDone', { a: result.total_a, b: result.total_b, pairs: result.pairs.length, time: (result.scan_time_ms / 1000).toFixed(1) }),
        status: 'success',
      }]);
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: String(e), status: 'error' }]);
      updateTask('dedup-rename', { status: /已取消|cancel/i.test(String(e)) ? 'cancelled' : 'error', message: String(e) });
      setHasError(true);
    } finally { setIsDone(true); setScanning(false); }
  }, [folderA, folderB, dhash, phash, colorTh, t]);

  const toggleDirection = (idx: number) => {
    setDirections(prev => { const n = [...prev]; n[idx] = n[idx] === 'a' ? 'b' : 'a'; return n; });
  };
  const setAllDirection = (d: Direction) => setDirections(prev => prev.map(() => d));

  const handleExecute = useCallback(async () => {
    if (pairs.length === 0) return;
    setExecuting(true); setProgress(0); setPCur(0); setPTot(0);
    setIsDone(false); setHasError(false);
    setLogs(prev => [...prev, { time: getTimeStr(), message: t('dedupRename.execStart', { count: pairs.length }), status: 'info' }]);
    addTask('dedup-rename', t('dedupRename.tabDedup'));
    try {
      const actions = pairs.map((p, i) => {
        const dir = directions[i];
        if (dir === 'a') {
          // B uses A's name: rename B's file to A's name stem + B's extension
          const aStem = p.name_a.replace(/\.[^.]+$/, '');
          const bExt = p.name_b.includes('.') ? p.name_b.replace(/^.*\./, '.') : '';
          const targetName = aStem + bExt;
          // Check if the target already exists in B's folder (same folder as B)
          const bDir = p.path_b.substring(0, p.path_b.length - p.name_b.length);
          const targetPath = bDir + targetName;
          const needConflict = targetPath !== p.path_b && folderA === folderB;
          return {
            src_path: p.path_b,
            target_name: targetName,
            conflict_path: needConflict ? p.path_a : null,
          };
        } else {
          // A uses B's name
          const bStem = p.name_b.replace(/\.[^.]+$/, '');
          const aExt = p.name_a.includes('.') ? p.name_a.replace(/^.*\./, '.') : '';
          const targetName = bStem + aExt;
          const aDir = p.path_a.substring(0, p.path_a.length - p.name_a.length);
          const targetPath = aDir + targetName;
          const needConflict = targetPath !== p.path_a && folderA === folderB;
          return {
            src_path: p.path_a,
            target_name: targetName,
            conflict_path: needConflict ? p.path_b : null,
          };
        }
      });
      const result = await invoke<{ success_count: number; fail_count: number; errors: string[] }>('execute_dedup_rename', { actions });
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: t('dedupRename.execDone', { ok: result.success_count, fail: result.fail_count }),
        status: result.fail_count > 0 ? 'warning' : 'success',
      }]);
      result.errors.forEach(err => setLogs(prev => [...prev, { time: getTimeStr(), message: err, status: 'error' }]));
      setPairs([]);
    } catch (e: any) {
      setLogs(prev => [...prev, { time: getTimeStr(), message: String(e), status: 'error' }]);
      updateTask('dedup-rename', { status: /已取消|cancel/i.test(String(e)) ? 'cancelled' : 'error', message: String(e) });
      setHasError(true);
    } finally { setExecuting(false); setIsDone(true); }
  }, [pairs, directions, folderA, folderB, t]);

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasError(false); setStartTime(0); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

  const panel: React.CSSProperties = { background: 'var(--color-bg-card)', border: '1px solid var(--color-border)', borderRadius: 'var(--radius-lg)', padding: 20 };
  const label: React.CSSProperties = { fontSize: 12, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 6, display: 'block' };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gap: 20, minHeight: 'calc(100vh - 260px)' }}>
      {/* Left: settings */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <div style={panel}>
          <label style={label}>{t('dedupRename.folderA')}</label>
          <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
            <input className="form-input" value={folderA} onChange={e => setFolderA(e.target.value)}
              placeholder={t('dedupRename.folderAHint')} style={{ flex: 1, fontSize: 12 }} />
            <button className="btn btn-secondary" onClick={() => pickFolder(setFolderA)} style={{ flexShrink: 0 }}>
              <FolderOpen style={{ width: 14, height: 14 }} />
            </button>
          </div>
          <label style={label}>{t('dedupRename.folderB')}</label>
          <div style={{ display: 'flex', gap: 6 }}>
            <input className="form-input" value={folderB} onChange={e => setFolderB(e.target.value)}
              placeholder={t('dedupRename.folderBHint')} style={{ flex: 1, fontSize: 12 }} />
            <button className="btn btn-secondary" onClick={() => pickFolder(setFolderB)} style={{ flexShrink: 0 }}>
              <FolderOpen style={{ width: 14, height: 14 }} />
            </button>
          </div>
        </div>

        <div style={panel}>
          <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)', marginBottom: 12 }}>{t('dedupRename.matchParams')}</div>
          {[
            { label: t('dedupRename.dhash'), value: dhash, set: setDhash, min: 1, max: 20, isInt: true },
            { label: t('dedupRename.phash'), value: phash, set: setPhash, min: 1, max: 20, isInt: true },
          ].map(s => (
            <div key={s.label} style={{ marginBottom: 12 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={label}>{s.label}</span>
                <span style={{ fontSize: 13, fontWeight: 700, color: '#7c5cfc', fontFamily: 'monospace' }}>{s.value}</span>
              </div>
              <input type="range" min={s.min} max={s.max} value={s.value}
                onChange={e => s.set(Number(e.target.value))}
                style={{ width: '100%', accentColor: '#7c5cfc' }} />
            </div>
          ))}
          <div style={{ marginBottom: 4 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
              <span style={label}>{t('dedupRename.colorTh')}</span>
              <span style={{ fontSize: 13, fontWeight: 700, color: '#7c5cfc', fontFamily: 'monospace' }}>{colorTh.toFixed(2)}</span>
            </div>
            <input type="range" min={0} max={100} value={Math.round(colorTh * 100)}
              onChange={e => setColorTh(Number(e.target.value) / 100)}
              style={{ width: '100%', accentColor: '#7c5cfc' }} />
          </div>
        </div>

        <ProcessButton processing={scanning} onStart={handleScan}
          disabled={!folderA || !folderB}
          cancelCommand="cancel_dedup_rename"
          startText={t('dedupRename.startScan')} processingText={t('dedupRename.scanning')}
          onCancelLog={addCancelLog} />

        <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasError} onClearLogs={clearLogs} externalStartTime={startTime} />
      </div>

      {/* Right: results */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minHeight: 300, overflow: 'hidden' }}>
        {pairs.length === 0 ? (
          <div style={{ ...panel, flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, color: 'var(--color-text-tertiary)' }}>
            <Search style={{ width: 48, height: 48, opacity: 0.15 }} />
            <span style={{ fontSize: 13 }}>{scanning ? t('dedupRename.scanning') : isDone ? t('dedupRename.noMatch') : t('dedupRename.hint')}</span>
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
                onClick={handleExecute} disabled={executing || pairs.length === 0}>
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
                            <button className="btn btn-ghost" onClick={() => setLightbox({ idx, side: 'a' })}
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
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8, padding: '8px 0', borderTop: '1px solid var(--color-border)' }}>
                  <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 28 }}
                    disabled={pairPage === 0} onClick={() => setPairPage(p => p - 1)}>
                    <ChevronLeft style={{ width: 14, height: 14 }} />
                  </button>
                  <span style={{ fontSize: 12, color: 'var(--color-text-secondary)', fontWeight: 600, minWidth: 60, textAlign: 'center' }}>
                    {pairPage + 1} / {Math.ceil(pairs.length / PAIRS_PER_PAGE)}
                  </span>
                  <button className="btn btn-ghost" style={{ padding: '4px 8px', height: 28 }}
                    disabled={pairPage >= Math.ceil(pairs.length / PAIRS_PER_PAGE) - 1} onClick={() => setPairPage(p => p + 1)}>
                    <ChevronRight style={{ width: 14, height: 14 }} />
                  </button>
                </div>
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
          <div onClick={() => setLightbox(null)} style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            background: 'rgba(0,0,0,0.85)', backdropFilter: 'blur(8px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 24,
            animation: 'fadeIn 0.15s ease',
          }}>
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
            <div style={{ position: 'absolute', top: 20, right: 20, fontSize: 12, color: 'rgba(255,255,255,0.4)' }}>{t('dedupRename.closeBg')}</div>
          </div>
        );
      })(), document.body)}

      {/* Unmatched modal */}
      {unmatchModal && createPortal((() => {
        const isA = unmatchModal === 'a';
        const items = isA ? unmatchedA : unmatchedB;
        const title = isA ? t('dedupRename.unmatchATitle') : t('dedupRename.unmatchBTitle');
        const color = isA ? '#f59e0b' : '#ef4444';
        return (
          <div onClick={() => setUnmatchModal(null)} style={{
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
                      const sourceFolder = isA ? folderA : folderB;
                      const result = await invoke<{ success_count: number; fail_count: number; errors: string[] }>('export_unmatched_files', {
                        sourceFolder, filenames: items, destFolder: dest as string,
                      });
                      setLogs(prev => [...prev, {
                        time: getTimeStr(),
                        message: t('dedupRename.exportDone', { success: result.success_count, fail: result.fail_count }),
                        status: result.fail_count > 0 ? 'warning' : 'success',
                      }]);
                      setUnmatchModal(null);
                    } catch (e: any) {
                      setLogs(prev => [...prev, { time: getTimeStr(), message: `${t('dedupRename.exportFail')}: ${String(e)}`, status: 'error' }]);
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
              <div style={{ padding: '10px 20px', borderTop: '1px solid var(--color-border)', fontSize: 11, color: 'var(--color-text-tertiary)' }}>
                {t('dedupRename.unmatchHint')}
              </div>
            </div>
          </div>
        );
      })(), document.body)}
    </div>
  );
}
