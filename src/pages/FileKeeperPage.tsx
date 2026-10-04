import { invoke } from '@tauri-apps/api/core';
import { FileCheck2, Shield } from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { FileKeeperOptions } from '../api/commandOptions';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import ChoiceCard from '../components/ui/ChoiceCard';
import PathInput from '../components/ui/PathInput';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';

const allExtensions = [
  { ext: 'jpg', color: '#ffa647' },
  { ext: 'jpeg', color: '#ffa647' },
  { ext: 'png', color: '#4ade80' },
  { ext: 'webp', color: '#60a5fa' },
  { ext: 'bmp', color: '#f87171' },
  { ext: 'npz', color: '#c084fc' },
  { ext: 'txt', color: '#fbbf24' },
];

export default function FileKeeperPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'keeper-progress', taskId: 'keeper' });
  const folderId = useId();
  const [folderPath, setFolderPath] = useState('');
  const [keepExts, setKeepExts] = useState<Set<string>>(new Set(['jpg', 'jpeg', 'png', 'webp', 'txt']));

  const toggleExt = (ext: string) => {
    setKeepExts((prev) => {
      const next = new Set(prev);
      if (next.has(ext)) next.delete(ext); else next.add(ext);
      return next;
    });
  };

  const selectAll = () => setKeepExts(new Set(allExtensions.map((e) => e.ext)));
  const deselectAll = () => setKeepExts(new Set());

  const handleProcess = () => {
    return task.run({
      taskName: t('fileKeeper.taskName'),
      startLog: t('fileKeeper.startMsg', { exts: Array.from(keepExts).join(', ') }),
      exec: () => invoke<ProcessResult>('keep_specified_files', {
        options: { folder_path: folderPath, keep_extensions: Array.from(keepExts) } satisfies FileKeeperOptions,
      }),
    });
  };

  return (
    <ToolPageLayout icon={FileCheck2} color="#fbbf24" title={t('fileKeeper.title')} subtitle={t('fileKeeper.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!folderPath || keepExts.size === 0}
          cancelCommand="cancel_keeper" startText={t('fileKeeper.startDelete')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('fileKeeper.folderPath')}</span></div>
        <div className="form-group">
          <label className="form-label" htmlFor={folderId}>{t('fileKeeper.selectFolder')}</label>
          <PathInput id={folderId} placeholder={t('fileKeeper.selectFolderPlaceholder')} value={folderPath} onChange={setFolderPath} />
        </div>
      </div>

      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('fileKeeper.keepTypes')}</span>
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <button className="btn btn-ghost btn-sm" onClick={selectAll}>{t('fileKeeper.selectAll')}</button>
            <button className="btn btn-ghost btn-sm" onClick={deselectAll}>{t('fileKeeper.deselectAll')}</button>
          </div>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: 'var(--space-3)' }}>
          {allExtensions.map((item) => {
            const checked = keepExts.has(item.ext);
            return (
              <ChoiceCard key={item.ext} compact selected={checked} onSelect={() => toggleExt(item.ext)} indicator="check">
                <span style={{
                  fontSize: 'var(--font-size-md)', fontWeight: 700,
                  color: checked ? item.color : 'var(--color-text-tertiary)',
                }}>
                  .{item.ext}
                </span>
              </ChoiceCard>
            );
          })}
        </div>
      </div>

      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 'var(--space-3)', padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', background: 'rgba(251, 191, 36, 0.04)', border: '1px solid rgba(251, 191, 36, 0.1)' }}>
        <Shield style={{ width: 18, height: 18, color: '#fbbf24', marginTop: 2, minWidth: 18 }} />
        <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.7 }}>
          <strong style={{ color: '#f87171' }}>{t('fileKeeper.warning')}</strong>{t('fileKeeper.warningText')}
        </div>
      </div>
    </ToolPageLayout>
  );
}
