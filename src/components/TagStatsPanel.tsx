import { ArrowUpDown, BarChart, CheckCircle2, Hash, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { useTagStats } from '../hooks/useTagStats';
import { getTagChipColor } from '../utils/tagText';

export default function TagStatsPanel({ stats, translations, currentTags, total, filteredCount, filterActive, onClearFilter, progress }: {
  stats: ReturnType<typeof useTagStats>; translations: Record<string, string>; currentTags: ReadonlySet<string>; total: number;
  filteredCount: number; filterActive: boolean; onClearFilter: () => void; progress: { current: number; total: number } | null;
}) {
  const { t } = useTranslation();
  return <>
    <div style={{ display: 'flex', gap: 4, padding: '8px 10px', borderBottom: '1px solid var(--color-border)' }}>
      <input className="form-input" placeholder={t('tagManager.searchTags')} value={stats.globalSearch} onChange={e => stats.setGlobalSearch(e.target.value)} style={{ minWidth: 0, fontSize: 11, height: 28 }} />
      <button className="btn btn-ghost btn-sm" title={t(stats.tagSortBy === 'freq' ? 'tagManager.sortByFreq' : 'tagManager.sortByName')} onClick={() => stats.setTagSortBy(v => v === 'freq' ? 'name' : 'freq')} style={{ padding: 4 }}>{stats.tagSortBy === 'freq' ? <BarChart size={13} /> : <Hash size={13} />}</button>
      <button className="btn btn-ghost btn-sm" title={t(stats.tagSortDir === 'desc' ? 'tagManager.descOrder' : 'tagManager.ascOrder')} onClick={() => stats.setTagSortDir(v => v === 'desc' ? 'asc' : 'desc')} style={{ padding: 4 }}><ArrowUpDown size={13} /></button>
    </div>
    {filterActive && <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 10px', fontSize: 10 }}><span style={{ flex: 1 }}>{t('tagManager.filtering')} {filteredCount}/{total}</span><button className="btn btn-ghost btn-sm" onClick={onClearFilter} title={t('tagManager.filterByTag')}><X size={12} /></button></div>}
    <div role="listbox" aria-multiselectable="true" aria-label={t('tagManager.allTags')} style={{ flex: 1, overflowY: 'auto', userSelect: 'none' }}>
      {stats.filteredStats.slice(0, stats.statsLimit).map(([tag, count]) => <div key={tag} role="option" aria-selected={stats.selectedTags.has(tag)} onMouseDown={e => e.preventDefault()} onClick={e => stats.toggleTagSelect(tag, e)}
        style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '7px 12px', cursor: 'pointer', borderLeft: `2px solid ${stats.selectedTags.has(tag) ? 'var(--color-accent-primary)' : 'transparent'}`, background: stats.selectedTags.has(tag) ? 'var(--color-bg-input)' : 'transparent' }}>
        <div style={{ flex: 1, minWidth: 0 }}><div style={{ fontSize: 11, color: getTagChipColor(tag).tx, overflowWrap: 'anywhere' }}>{tag}<span style={{ marginLeft: 4, fontSize: 10, color: 'var(--color-text-tertiary)' }}>{translations[tag]}</span></div>
          <div style={{ height: 3, background: 'var(--color-bg-input)', marginTop: 3 }}><div style={{ height: '100%', width: `${total ? count / total * 100 : 0}%`, background: getTagChipColor(tag).tx }} /></div>
        </div><span style={{ fontSize: 10 }}>{count}</span>{currentTags.has(tag) && <CheckCircle2 size={12} color="#4ade80" />}
      </div>)}
      {stats.filteredStats.length > stats.statsLimit && <button className="btn btn-ghost" style={{ width: '100%', fontSize: 10 }} onClick={() => stats.setStatsLimit(v => v + 300)}>{t('common.showMore', { n: stats.filteredStats.length - stats.statsLimit })}</button>}
      {total > 0 && !stats.filteredStats.length && <div style={{ padding: 20, fontSize: 11 }}>{t(stats.globalSearch ? 'tagManager.noMatch' : 'tagManager.noTags')}</div>}
    </div>
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 6, padding: '6px 12px', fontSize: 10, borderTop: '1px solid var(--color-border)' }}>
      <span>{t(stats.selectedTags.size ? 'tagManager.selected' : 'tagManager.nTagTypes', { n: stats.selectedTags.size || stats.tagStats.length })}</span>
      <span>{stats.taggedCount}/{total} {t('tagManager.tagged')}</span>
    </div>
    {progress && <div style={{ padding: '5px 12px', fontSize: 10 }}>{t('tagManager.translateProgress')} {progress.current}/{progress.total}</div>}
  </>;
}
