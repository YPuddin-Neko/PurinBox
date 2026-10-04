import { invoke } from '@tauri-apps/api/core';
import {
  ArrowRight,
  Eye,
  Hash,
  Loader2,
  Play,
  Shuffle,
  TextCursorInput,
  Type
} from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildRenameOptions, RENAME_DEFAULTS, type ProcessResult, type RenameOptions } from '../api/commandOptions';
import Checkbox from '../components/Checkbox';
import DedupRenameTab from '../components/DedupRenameTab';
import ProgressLog from '../components/ProgressLog';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import Pager from '../components/ui/Pager';
import PathInput from '../components/ui/PathInput';
import SegmentedTabs from '../components/ui/SegmentedTabs';
import { useBatchTask } from '../hooks/useBatchTask';

interface PreviewItem {
  original: string;
  renamed: string;
  /** 共用这个标签文件的图片；非空时执行不重命名它 */
  shared_by?: string[];
  /** 新名已被其他文件占着；有这样的行时执行会整批拒绝 */
  blocked?: boolean;
}

export default function BatchRenamePage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'rename-progress', taskId: 'rename' });
  const d = RENAME_DEFAULTS;
  const folderId = useId();
  const [activeTab, setActiveTab] = useState<'number' | 'dedup'>('number');
  const [inputPath, setInputPath] = useState('');
  const [prefix, setPrefix] = useState(d.prefix);
  const [startNumber, setStartNumber] = useState(d.start_number);
  const [digitCount, setDigitCount] = useState(d.digit_count);
  const [renameTags, setRenameTags] = useState(d.rename_tags);
  const [previewPage, setPreviewPage] = useState(0);
  const PREVIEW_PER_PAGE = 15;
  const [previewLoading, setPreviewLoading] = useState(false);

  const [preview, setPreview] = useState<{ options: RenameOptions; items: PreviewItem[]; key: string } | null>(null);
  const previewRequest = useRef(0);
  const inputKey = JSON.stringify([inputPath, prefix, startNumber, digitCount, renameTags]);
  const previewOpts = preview?.key === inputKey ? preview.options : null;
  const previews = previewOpts ? preview!.items : [];
  useEffect(() => { previewRequest.current++; setPreview(null); setPreviewLoading(false); }, [inputKey]);

  const runPreview = async (shuffle: boolean) => {
    if (!inputPath || task.processing) return;
    const request = ++previewRequest.current;
    // 种子在预览时定下，执行时原样复用，打乱后的顺序才与预览一致
    const options = buildRenameOptions({ input_path: inputPath }, {
      prefix, start_number: startNumber, digit_count: digitCount, rename_tags: renameTags,
      shuffle, shuffle_seed: shuffle ? Date.now() % 4294967296 : undefined,
    });
    setPreview(null); setPreviewLoading(true);
    try {
      const items = await invoke<PreviewItem[]>('preview_rename', { options });
      if (request !== previewRequest.current) return;
      setPreview({ options, items, key: inputKey }); setPreviewPage(0);
    } catch (error) {
      if (request === previewRequest.current) task.logger.appendCatchError(error, t('batchRename.previewFailed'));
    } finally { if (request === previewRequest.current) setPreviewLoading(false); }
  };

  const handleExecute = async () => {
    if (!previewOpts || !previews.length) return;
    const options = previewOpts;
    await task.run({
      taskName: t('batchRename.taskName'),
      startLog: t('batchRename.startMsg', { prefix: options.prefix, start: options.start_number, digits: options.digit_count }),
      cancellable: false,
      exec: () => invoke<ProcessResult>('execute_rename', { options }),
    });
    setPreview(null);
  };

  const exampleNum = String(startNumber).padStart(digitCount, '0');
  const exampleName = `${prefix}${exampleNum}.png`;

  return (
    <div className="page" style={activeTab === 'dedup' ? { display: 'flex', flexDirection: 'column', height: '100%' } : undefined}>
      <PageHeader icon={TextCursorInput} color={'#38bdf8'} title={t('batchRename.title')} subtitle={t('batchRename.subtitle')} />

      <SegmentedTabs value={activeTab} onChange={setActiveTab} style={{ marginBottom: 'var(--space-4)' }} tabs={[
        { id: 'number', label: t('dedupRename.tabNumber') },
        { id: 'dedup', label: t('dedupRename.tabDedup') },
      ]} />

      {/* 序号重命名 */}
      <div style={{ display: activeTab === 'number' ? 'block' : 'none' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 400px', gap: 'var(--space-6)' }}>
          {/* 左侧 */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
            {/* 路径 */}
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">{t('batchRename.imageFolder')}</span></div>
              <div className="form-group">
                <label className="form-label" htmlFor={folderId}>{t('batchRename.folderDesc')}</label>
                <PathInput id={folderId} pick="folderOrImage" placeholder={t('batchRename.selectFolder')} value={inputPath} onChange={setInputPath} />
              </div>
            </div>

            {/* 命名规则 */}
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">{t('batchRename.namingRule')}</span></div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
                <div className="form-group">
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Type style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                    {t('batchRename.prefix')}
                  </label>
                  <input className="form-input" value={prefix} onChange={(e) => setPrefix(e.target.value)} placeholder={t('batchRename.prefixPlaceholder')} />
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                      {t('batchRename.startNum')}
                    </label>
                    <NumberInput className="form-input" value={startNumber} min={0} onChange={setStartNumber} fallback={1} integer />
                  </div>
                  <div className="form-group" style={{ flex: 1 }}>
                    <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <Hash style={{ width: 14, height: 14, color: 'var(--color-text-tertiary)' }} />
                      {t('batchRename.digitCount')}
                    </label>
                    <NumberInput className="form-input" value={digitCount} min={1} max={10} onChange={setDigitCount} fallback={4} integer />
                  </div>
                </div>

                {/* 同步重命名标签文件 */}
                <Checkbox checked={renameTags} onChange={setRenameTags} color="#38bdf8" size={14}
                  label={t('batchRename.renameTags')}
                  style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }} />

                {/* 命名示例 */}
                <div style={{
                  padding: 'var(--space-3) var(--space-4)',
                  borderRadius: 'var(--radius-md)',
                  background: 'rgba(56, 189, 248, 0.06)',
                  border: '1px solid rgba(56, 189, 248, 0.12)',
                  display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                }}>
                  <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)' }}>{t('batchRename.namingExample')}</span>
                  <span style={{ fontSize: 'var(--font-size-md)', fontWeight: 700, color: '#38bdf8', fontFamily: 'monospace' }}>{exampleName}</span>
                </div>
              </div>
            </div>

            {/* 操作按钮 */}
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={() => runPreview(false)} disabled={!inputPath || previewLoading || task.processing}>
                {previewLoading ? <Loader2 style={{ width: 16, height: 16, animation: 'spin 1s linear infinite' }} /> : <Eye style={{ width: 16, height: 16 }} />}
                {t('batchRename.generatePreview')}
              </button>
              <button className="btn btn-secondary" style={{ flex: 1, height: 44 }} onClick={() => runPreview(true)} disabled={!inputPath || previewLoading || task.processing}>
                <Shuffle style={{ width: 16, height: 16 }} />
                {t('batchRename.shufflePreview')}
              </button>
            </div>

            {/* 预览表格 */}
            {previews.length > 0 && (() => {
              const totalPages = Math.ceil(previews.length / PREVIEW_PER_PAGE);
              const pageItems = previews.slice(previewPage * PREVIEW_PER_PAGE, (previewPage + 1) * PREVIEW_PER_PAGE);
              const startIdx = previewPage * PREVIEW_PER_PAGE;
              const blockedCount = previews.filter(item => item.blocked).length;
              return (
                <div className="tool-panel" style={{ padding: 0, overflow: 'hidden' }}>
                  <div className="tool-panel-header" style={{ padding: 'var(--space-3) var(--space-4)' }}>
                    <span className="tool-panel-title">{t('batchRename.previewTitle')}</span>
                    <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>
                      {t('batchRename.fileCount', { count: previews.length })}
                      {blockedCount > 0 && <span style={{ color: 'var(--color-error)' }}> · {t('batchRename.conflictCount', { count: blockedCount })}</span>}
                    </span>
                  </div>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr style={{ borderBottom: '1px solid var(--color-border)' }}>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>#</th>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('batchRename.originalName')}</th>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'center', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', width: 30 }}></th>
                        <th style={{ padding: 'var(--space-2) var(--space-4)', textAlign: 'left', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontWeight: 600 }}>{t('batchRename.newName')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {pageItems.map((item, localIdx) => {
                        const idx = startIdx + localIdx;
                        const shared = item.shared_by?.length ? item.shared_by : null;
                        const note = item.blocked ? t('batchRename.targetExists')
                          : shared ? t('batchRename.sharedNotRenamed', { files: shared }) : null;
                        return (
                          <tr key={idx} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                            <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>{idx + 1}</td>
                            <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', fontFamily: 'monospace', wordBreak: 'break-all' }}>{item.original}</td>
                            <td style={{ padding: '6px 0', textAlign: 'center' }}><ArrowRight style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /></td>
                            <td style={{ padding: '6px var(--space-4)', fontSize: 'var(--font-size-sm)', color: item.blocked ? 'var(--color-error)' : shared ? 'var(--color-warning)' : '#38bdf8', fontFamily: 'monospace', fontWeight: 600 }}>
                              {item.renamed}
                              {note && (
                                <div style={{ fontSize: 'var(--font-size-xs)', fontFamily: 'inherit', fontWeight: 400, wordBreak: 'break-all' }}>
                                  {note}
                                </div>
                              )}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                  {totalPages > 1 && (
                    <Pager footer page={previewPage} pages={totalPages} onChange={setPreviewPage} />
                  )}
                </div>
              );
            })()}
          </div>

          {/* 右侧 */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
            <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleExecute}
              disabled={task.processing || !previewOpts || previews.length === 0}>
              {task.processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> {t('batchRename.executing')}</> : <><Play style={{ width: 18, height: 18 }} /> {t('batchRename.executeRename')}</>}
            </button>
            <ProgressLog {...task.progressLogProps} />
          </div>
        </div>
      </div>

      {/* 查重重命名 */}
      <div style={{ display: activeTab === 'dedup' ? 'flex' : 'none', flexDirection: 'column', flex: 1, minHeight: 0 }}>
        <DedupRenameTab />
      </div>
    </div>
  );
}
