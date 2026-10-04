import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowRight, Download, Loader2, Play, RotateCcw, Search } from 'lucide-react';
import { useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type {
  DedupPair, DedupRenameOptions, DedupRenameResult, DedupRenameScanResult, ExportUnmatchedArgs, RenameAction,
} from '../api/commandOptions';
import { useBatchTask } from '../hooks/useBatchTask';
import { ensureAssetScope } from '../utils/assetScope';
import HashThresholdFields from './HashThresholdFields';
import LightboxShell from './LightboxShell';
import { Modal } from './Modal';
import ProcessButton from './ProcessButton';
import ProgressLog from './ProgressLog';
import Pager from './ui/Pager';
import PathInput from './ui/PathInput';
import ResultTable from './ui/ResultTable';
import StatChips from './ui/StatChips';

// 'a'：B 改用 A 的名字；'b'：A 改用 B 的名字
type Direction = 'a' | 'b';

/** srcPath 改用 nameFrom 的主名并保留自己的扩展名；otherPath 是配对中的另一个文件 */
function buildRenameAction(srcPath: string, srcName: string, nameFrom: string, otherPath: string): RenameAction {
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

const sourceBadgeStyle = { fontSize: 8, padding: '1px 4px', borderRadius: 3, background: 'rgba(74,222,128,0.15)', color: '#4ade80', fontWeight: 700, whiteSpace: 'nowrap' } as const;

export default function DedupRenameTab() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'dedup-rename-progress', taskId: 'dedup-rename', logDone: false });
  const folderAId = useId();
  const folderBId = useId();
  const [scanFolders, setScanFolders] = useState({ a: '', b: '' });

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
  /** 未匹配导出框里显示的错误（同一文件夹、后端报错、逐个文件的失败原因） */
  const [exportErrors, setExportErrors] = useState<string[]>([]);
  const [exportingUnmatched, setExportingUnmatched] = useState(false);
  /** 每次关闭导出框加一：导出返回时框已关闭（或已重新扫描）就不再写回错误 */
  const unmatchedSession = useRef(0);
  const [lightbox, setLightbox] = useState<{ idx: number } | null>(null);
  const [pairPage, setPairPage] = useState(0);
  const PAIRS_PER_PAGE = 15;

  const closeUnmatched = () => {
    unmatchedSession.current += 1;
    setUnmatchModal(null);
    setExportErrors([]);
  };

  const handleScan = async () => {
    setPairs([]); setDirections([]); setUnmatchedA([]); setUnmatchedB([]); setLightbox(null);
    closeUnmatched();
    const result = await task.run({
      taskName: t('dedupRename.tabDedup'),
      startLog: t('dedupRename.scanStart'),
      exec: async () => {
        await ensureAssetScope(folderA); await ensureAssetScope(folderB);
        return invoke<DedupRenameScanResult>('scan_dedup_rename', {
          options: { folder_a: folderA, folder_b: folderB, dhash_threshold: dhash, phash_threshold: phash, color_threshold: colorTh } satisfies DedupRenameOptions,
        });
      },
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
        taskName: t('dedupRename.tabDedup'),
        startLog: t('dedupRename.execStart', { count: pairs.length }),
        keepLogs: true,
        cancellable: false,
        exec: () => {
          const actions = pairs.map((p, i) => directions[i] === 'a'
            ? buildRenameAction(p.path_b, p.name_b, p.name_a, p.path_a)
            : buildRenameAction(p.path_a, p.name_a, p.name_b, p.path_b));
          return invoke<DedupRenameResult>('execute_dedup_rename', { actions });
        },
      });
      if (!result) return;
      task.logger.appendLog(t('dedupRename.execDone', { ok: result.success_count, fail: result.fail_count }), result.fail_count ? 'warning' : 'success');
      if (result.errors.length) task.logger.appendLog(result.errors.join('\n'), 'error');
      setPairs([]); setLightbox(null);
    } finally { setExecuting(false); }
  };

  const exportUnmatched = async (side: 'a' | 'b', filenames: string[]) => {
    const dest = await open({ directory: true, title: t('dedupRename.exportSelectDest') });
    if (typeof dest !== 'string') return;
    const sourceFolder = side === 'a' ? scanFolders.a : scanFolders.b;
    if (sourceFolder.replace(/[\\/]+$/, '') === dest.replace(/[\\/]+$/, '')) {
      const message = t('dedupRename.exportSameFolder');
      setExportErrors([message]);
      task.logger.appendLog(message, 'warning');
      return;
    }
    const session = unmatchedSession.current;
    setExportErrors([]);
    setExportingUnmatched(true);
    try {
      const result = await invoke<DedupRenameResult>('export_unmatched_files', {
        sourceFolder, filenames, destFolder: dest,
      } satisfies ExportUnmatchedArgs);
      const summary = t('dedupRename.exportDone', { success: result.success_count, fail: result.fail_count });
      const failed = result.fail_count > 0 || result.errors.length > 0;
      task.logger.appendLog(summary, failed ? 'warning' : 'success');
      if (result.errors.length) task.logger.appendLog(result.errors.join('\n'), 'error');
      if (session !== unmatchedSession.current) return;
      if (failed) setExportErrors([summary, ...result.errors]);
      else closeUnmatched();
    } catch (e) {
      const message = `${t('dedupRename.exportFail')}: ${String(e)}`;
      task.logger.appendLog(message, 'error');
      if (session === unmatchedSession.current) setExportErrors([message]);
    } finally {
      setExportingUnmatched(false);
    }
  };

  const pairPages = Math.ceil(pairs.length / PAIRS_PER_PAGE);
  const unmatchedItems = unmatchModal === 'a' ? unmatchedA : unmatchedB;
  const lightboxPair = lightbox ? pairs[lightbox.idx] : undefined;

  return (
    // 网格占满宿主页剩余高度：配对列表在右栏内滚动，翻页栏始终可见
    <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gridTemplateRows: 'minmax(0, 1fr)', gap: 20, flex: 1, minHeight: 420 }}>
      {/* 左栏：设置 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minHeight: 0, overflowY: 'auto' }}>
        <div className="tool-panel">
          <label className="form-label" htmlFor={folderAId} style={{ display: 'block', marginBottom: 6 }}>{t('dedupRename.folderA')}</label>
          <PathInput id={folderAId} size="sm" value={folderA} onChange={setFolderA} style={{ gap: 6, marginBottom: 12 }} />
          <label className="form-label" htmlFor={folderBId} style={{ display: 'block', marginBottom: 6 }}>{t('dedupRename.folderB')}</label>
          <PathInput id={folderBId} size="sm" value={folderB} onChange={setFolderB} style={{ gap: 6 }} />
        </div>

        <div className="tool-panel">
          <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)', marginBottom: 12 }}>{t('dedupRename.matchParams')}</div>
          <HashThresholdFields dhash={dhash} onDhash={setDhash} phash={phash} onPhash={setPhash} color={colorTh} onColor={setColorTh} />
        </div>

        <ProcessButton {...task.buttonProps} onStart={handleScan}
          disabled={!folderA || !folderB || task.processing}
          cancelCommand="cancel_dedup_rename"
          startText={t('dedupRename.startScan')} processingText={t('dedupRename.scanning')}
        />

        <ProgressLog {...task.progressLogProps} />
      </div>

      {/* 右栏：结果 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minHeight: 300, overflow: 'hidden' }}>
        {pairs.length === 0 ? (
          <div className="tool-panel" style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 12, color: 'var(--color-text-tertiary)' }}>
            <Search style={{ width: 48, height: 48, opacity: 0.15 }} />
            <span style={{ fontSize: 13 }}>{task.processing && !executing ? t('dedupRename.scanning') : task.progressLogProps.isDone ? t('dedupRename.noMatch') : ''}</span>
          </div>
        ) : (
          <>
            <StatChips items={[
              { label: t('dedupRename.totalA'), value: totalA, color: '#60a5fa' },
              { label: t('dedupRename.totalB'), value: totalB, color: '#38bdf8' },
              { label: t('dedupRename.matchCount'), value: pairs.length, color: '#4ade80' },
              { label: t('dedupRename.unmatchA'), value: unmatchedA.length, color: '#f59e0b', onClick: unmatchedA.length > 0 ? () => setUnmatchModal('a') : undefined },
              { label: t('dedupRename.unmatchB'), value: unmatchedB.length, color: '#ef4444', onClick: unmatchedB.length > 0 ? () => setUnmatchModal('b') : undefined },
            ]} />

            {/* 操作栏 */}
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

            <ResultTable title={t('dedupRename.matchList')}
              columns={[
                { label: '#', width: 30 },
                { label: t('dedupRename.imageA') },
                { label: t('dedupRename.direction'), align: 'center', width: 80 },
                { label: t('dedupRename.imageB') },
                { label: t('dedupRename.preview'), align: 'center', width: 60 },
              ]}
              footer={pairPages > 1 && <Pager footer page={pairPage} pages={pairPages} onChange={setPairPage} />}>
              {pairs.slice(pairPage * PAIRS_PER_PAGE, (pairPage + 1) * PAIRS_PER_PAGE).map((pair, localIdx) => {
                const idx = pairPage * PAIRS_PER_PAGE + localIdx;
                const isASource = directions[idx] === 'a';
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
                        {isASource && <span style={sourceBadgeStyle}>{t('dedupRename.source')}</span>}
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
                        {!isASource && <span style={sourceBadgeStyle}>{t('dedupRename.source')}</span>}
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
            </ResultTable>
          </>
        )}
      </div>

      {lightboxPair && (
        <LightboxShell onClose={() => setLightbox(null)}>
          <div style={{ display: 'flex', gap: 24, maxWidth: '90vw', maxHeight: '85vh' }}>
            {[{ path: lightboxPair.path_a, name: lightboxPair.name_a, label: 'A' }, { path: lightboxPair.path_b, name: lightboxPair.name_b, label: 'B' }].map(item => (
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
      )}

      <Modal open={unmatchModal !== null} onClose={closeUnmatched}
        title={unmatchModal === 'a' ? t('dedupRename.unmatchATitle') : t('dedupRename.unmatchBTitle')}
        sectioned width={420} maxWidth={420}
        dialogStyle={{ borderRadius: 12, maxHeight: '70vh' }}
        titleStyle={{ fontSize: 14, fontWeight: 700 }}
        headerIcon={<span style={{ fontSize: 16, fontWeight: 800, color: unmatchModal === 'a' ? '#f59e0b' : '#ef4444', fontFamily: 'monospace' }}>{unmatchedItems.length}</span>}
        headerExtra={
          <button className="btn btn-secondary" style={{ padding: '4px 12px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}
            disabled={exportingUnmatched} onClick={() => unmatchModal && void exportUnmatched(unmatchModal, unmatchedItems)}>
            {exportingUnmatched ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <Download style={{ width: 12, height: 12 }} />}
            {t('dedupRename.export')}
          </button>
        }
        bodyStyle={{ padding: 0, lineHeight: 1.5, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '8px 0' }}>
          {unmatchedItems.map((name, i) => (
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
        {exportErrors.length > 0 && (
          <div role="alert" style={{
            flexShrink: 0, maxHeight: 132, overflowY: 'auto', padding: '10px 20px',
            borderTop: '1px solid var(--color-border)', fontSize: 12, color: 'var(--color-error)', overflowWrap: 'anywhere',
          }}>
            {exportErrors.map((message, i) => <div key={i}>{message}</div>)}
          </div>
        )}
      </Modal>
    </div>
  );
}
