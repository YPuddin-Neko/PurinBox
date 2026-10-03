import { ArrowUpDown, BarChart, CheckCircle2, Filter, Hash, Search, Tags, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { useTagStats } from '../hooks/useTagStats';
import { getTagChipColor } from '../utils/tagText';
import '../styles/tags.css';

export default function TagStatsPanel({ stats, translations, currentTags, total, filteredCount, filterActive, onClearFilter, progress }: {
  stats: ReturnType<typeof useTagStats>; translations: Record<string, string>; currentTags: ReadonlySet<string>; total: number;
  filteredCount: number; filterActive: boolean; onClearFilter: () => void; progress: { current: number; total: number } | null;
}) {
  const { t } = useTranslation();
  return <>
    <div style={{ display: 'flex', gap: 4, padding: '8px 10px', borderBottom: '1px solid var(--color-border)' }}>
      <div style={{ position: 'relative', flex: 1, minWidth: 0 }}><Search size={12} style={{ position: 'absolute', left: 8, top: '50%', transform: 'translateY(-50%)', color: 'var(--color-text-tertiary)' }} />
        <input className="form-input" placeholder={t('tagManager.searchTags')} value={stats.globalSearch} onChange={e => stats.setGlobalSearch(e.target.value)} style={{ minWidth: 0, paddingLeft: 26, fontSize: 11, height: 28 }} />
      </div>
      <button className="btn btn-ghost btn-sm" title={t(stats.tagSortBy === 'freq' ? 'tagManager.sortByFreq' : 'tagManager.sortByName')} onClick={() => stats.setTagSortBy(v => v === 'freq' ? 'name' : 'freq')} style={{ padding: 0, width: 28, height: 28, flexShrink: 0, color: stats.tagSortBy === 'freq' ? '#60a5fa' : '#a78bfa' }}>{stats.tagSortBy === 'freq' ? <BarChart size={13} /> : <Hash size={13} />}</button>
      <button className="btn btn-ghost btn-sm" title={t(stats.tagSortDir === 'desc' ? 'tagManager.descOrder' : 'tagManager.ascOrder')} onClick={() => stats.setTagSortDir(v => v === 'desc' ? 'asc' : 'desc')} style={{ padding: 0, width: 28, height: 28, flexShrink: 0 }}><ArrowUpDown size={13} style={{ transform: stats.tagSortDir === 'asc' ? 'scaleY(-1)' : undefined, transition: 'transform 0.2s' }} /></button>
    </div>
    {filterActive && <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 10px', fontSize: 10, color: '#a78bfa', borderBottom: '1px solid var(--color-border)', background: 'linear-gradient(90deg,rgba(124,92,252,0.08),rgba(124,92,252,0.02))' }}><Filter size={10} /><span style={{ flex: 1 }}>{t('tagManager.filtering')} {filteredCount}/{total}</span><button className="tag-chip-remove" onClick={onClearFilter} title={t('tagManager.filterByTag')} style={{ opacity: 1, color: '#f87171', background: 'rgba(248,113,113,0.1)' }}><X size={8} /></button></div>}
    <div role="listbox" aria-multiselectable="true" aria-label={t('tagManager.allTags')} style={{ flex: 1, overflowY: 'auto', userSelect: 'none' }}>
      {stats.filteredStats.slice(0, stats.statsLimit).map(([tag, count]) => <div key={tag} role="option" className="tag-stat-row" data-current={currentTags.has(tag)} aria-selected={stats.selectedTags.has(tag)} onMouseDown={e => e.preventDefault()} onClick={e => stats.toggleTagSelect(tag, e)}
        style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '7px 12px', cursor: 'pointer', borderBottom: '1px solid rgba(255,255,255,0.03)', borderLeft: `2px solid ${stats.selectedTags.has(tag) ? 'var(--color-accent-primary)' : 'transparent'}` }}>
        <div style={{ flex: 1, minWidth: 0 }}><div style={{ fontSize: 11, fontWeight: 500, color: getTagChipColor(tag).tx, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={tag}>{tag}<span style={{ marginLeft: 4, fontSize: 10, fontWeight: 400, color: 'var(--color-text-tertiary)' }}>{translations[tag]}</span></div>
          <div style={{ height: 3, borderRadius: 2, overflow: 'hidden', background: 'var(--color-bg-input)', marginTop: 3 }}><div className="tag-stat-bar" style={{ height: '100%', borderRadius: 2, width: `${total ? count / total * 100 : 0}%`, background: `linear-gradient(90deg,${getTagChipColor(tag).bd},${getTagChipColor(tag).tx})` }} /></div>
        </div><span style={{ fontSize: 10, minWidth: 28, textAlign: 'right', flexShrink: 0, color: 'var(--color-text-tertiary)' }}>{count}</span>{currentTags.has(tag) && <CheckCircle2 size={12} color="#4ade80" style={{ flexShrink: 0 }} />}
      </div>)}
      {stats.filteredStats.length > stats.statsLimit && <button className="btn btn-ghost" style={{ width: '100%', fontSize: 10 }} onClick={() => stats.setStatsLimit(v => v + 300)}>{t('common.showMore', { n: stats.filteredStats.length - stats.statsLimit })}</button>}
      {!total && <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', gap: 8, color: 'var(--color-text-tertiary)', padding: 20 }}><Tags size={28} style={{ opacity: 0.2 }} /><span style={{ fontSize: 11, opacity: 0.6 }}>{t('tagManager.loadTagsHint')}</span></div>}
      {total > 0 && !stats.filteredStats.length && <div style={{ padding: 20, fontSize: 11, textAlign: 'center', color: 'var(--color-text-tertiary)' }}>{t(stats.globalSearch ? 'tagManager.noMatch' : 'tagManager.noTags')}</div>}
    </div>
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 6, padding: '6px 12px', fontSize: 10, color: 'var(--color-text-tertiary)', borderTop: '1px solid var(--color-border)' }}>
      <span style={{ color: stats.selectedTags.size ? '#a78bfa' : undefined }}>{t(stats.selectedTags.size ? 'tagManager.selected' : 'tagManager.nTagTypes', { n: stats.selectedTags.size || stats.tagStats.length })}</span>
      <span>{stats.taggedCount}/{total} {t('tagManager.tagged')}</span>
    </div>
    {progress && <div style={{ padding: '5px 12px', fontSize: 10, display: 'flex', alignItems: 'center', gap: 8, borderTop: '1px solid var(--color-border)', background: 'rgba(96,165,250,0.04)' }}>
      <span style={{ color: '#60a5fa', fontWeight: 600 }}>{t('tagManager.translateProgress')}</span>
      <div style={{ flex: 1, height: 3, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}><div className="tag-stat-bar" style={{ width: `${progress.total ? progress.current / progress.total * 100 : 0}%`, height: '100%', background: progress.current >= progress.total ? '#4ade80' : 'linear-gradient(90deg,#7c5cfc,#00d4ff)' }} /></div>
      <span style={{ color: progress.current >= progress.total ? '#4ade80' : 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>{progress.current}/{progress.total}</span>
    </div>}
  </>;
}
