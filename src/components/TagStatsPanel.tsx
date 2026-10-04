import { ArrowUpDown, BarChart, CheckCircle2, Filter, Hash, Search, Tags, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { useTagStats } from '../hooks/useTagStats';
import { getTagChipColor, type TagChipColor } from '../utils/tagText';
import '../styles/tags.css';

const iconButton = { padding: 0, width: 28, height: 28, flexShrink: 0 } as const;
const icon13 = { width: 13, height: 13 } as const;

/** 数据集标签统计：搜索与排序、按选中标签筛选的状态、频次列表、计数与翻译进度 */
export default function TagStatsPanel({
  stats, translations, currentTags, total, filteredCount, filterActive, onClearFilter, progress, colorOf = getTagChipColor,
}: {
  stats: ReturnType<typeof useTagStats>;
  translations: Record<string, string>;
  currentTags: ReadonlySet<string>;
  total: number;
  filteredCount: number;
  filterActive: boolean;
  onClearFilter: () => void;
  progress: { current: number; total: number } | null;
  /** 标签名与频次条的配色，默认 TXT 的 16 色 */
  colorOf?: (tag: string) => TagChipColor;
}) {
  const { t } = useTranslation();
  const progressDone = !!progress && progress.current >= progress.total;
  return <>
    <div style={{ display: 'flex', gap: 4, padding: '8px 10px', borderBottom: '1px solid var(--color-border)' }}>
      <div style={{ position: 'relative', flex: 1, minWidth: 0 }}>
        <Search size={12} style={{ position: 'absolute', left: 8, top: '50%', transform: 'translateY(-50%)', color: 'var(--color-text-tertiary)' }} />
        <input className="form-input" placeholder={t('tagEditor.searchTags')} value={stats.globalSearch}
          onChange={e => stats.setGlobalSearch(e.target.value)} style={{ minWidth: 0, paddingLeft: 26, fontSize: 11, height: 28 }} />
      </div>
      <button className="btn btn-ghost btn-sm" title={t(stats.tagSortBy === 'freq' ? 'tagEditor.sortByFreq' : 'tagEditor.sortByName')}
        onClick={() => stats.setTagSortBy(v => v === 'freq' ? 'name' : 'freq')}
        style={{ ...iconButton, color: stats.tagSortBy === 'freq' ? '#60a5fa' : '#a78bfa' }}>
        {stats.tagSortBy === 'freq' ? <BarChart style={icon13} /> : <Hash style={icon13} />}
      </button>
      <button className="btn btn-ghost btn-sm" title={t(stats.tagSortDir === 'desc' ? 'tagEditor.descOrder' : 'tagEditor.ascOrder')}
        onClick={() => stats.setTagSortDir(v => v === 'desc' ? 'asc' : 'desc')} style={{ ...iconButton, color: 'var(--color-text-tertiary)' }}>
        <ArrowUpDown style={{ ...icon13, transform: stats.tagSortDir === 'asc' ? 'scaleY(-1)' : undefined, transition: 'transform 0.2s' }} />
      </button>
    </div>
    {filterActive && (
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 10px', fontSize: 10, color: '#a78bfa',
        borderBottom: '1px solid var(--color-border)', background: 'linear-gradient(90deg,rgba(124,92,252,0.08),rgba(124,92,252,0.02))' }}>
        <Filter size={10} />
        <span style={{ flex: 1 }}>{t('tagEditor.filtering')} <b>{filteredCount}</b>/{total}</span>
        <button type="button" className="tag-chip-remove" onClick={onClearFilter} aria-label={t('tagEditor.clearFilter')}
          style={{ opacity: 1, color: '#f87171', background: 'rgba(248,113,113,0.1)' }}>
          <X size={8} />
        </button>
      </div>
    )}
    <div role="listbox" aria-multiselectable="true" aria-label={t('tagEditor.allTags')} style={{ flex: 1, overflowY: 'auto', userSelect: 'none' }}>
      {stats.filteredStats.slice(0, stats.statsLimit).map(([tag, count]) => {
        const color = colorOf(tag);
        const selected = stats.selectedTags.has(tag);
        return (
          <div key={tag} role="option" className="tag-stat-row" data-current={currentTags.has(tag)} aria-selected={selected}
            onMouseDown={e => e.preventDefault()} onClick={e => stats.toggleTagSelect(tag, e)}
            style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '7px 12px', cursor: 'pointer',
              borderBottom: '1px solid rgba(255,255,255,0.03)', borderLeft: `2px solid ${selected ? 'var(--color-accent-primary)' : 'transparent'}` }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div title={tag} style={{ fontSize: 11, fontWeight: 500, color: color.tx, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {tag}
                <span style={{ marginLeft: 4, fontSize: 10, fontWeight: 400, color: 'var(--color-text-tertiary)' }}>{translations[tag]}</span>
              </div>
              <div style={{ height: 3, borderRadius: 2, overflow: 'hidden', background: 'var(--color-bg-input)', marginTop: 3 }}>
                <div className="tag-stat-bar" style={{ height: '100%', borderRadius: 2, width: `${total ? count / total * 100 : 0}%`,
                  background: `linear-gradient(90deg,${color.bd},${color.tx})` }} />
              </div>
            </div>
            <span style={{ fontSize: 10, minWidth: 28, textAlign: 'right', flexShrink: 0, color: 'var(--color-text-tertiary)' }}>{count}</span>
            {currentTags.has(tag) && <CheckCircle2 size={12} color="#4ade80" style={{ flexShrink: 0 }} />}
          </div>
        );
      })}
      {stats.filteredStats.length > stats.statsLimit && (
        <button className="btn btn-ghost" style={{ width: '100%', height: 30, fontSize: 10, borderRadius: 0 }} onClick={() => stats.setStatsLimit(v => v + 300)}>
          {t('common.showMore', { n: stats.filteredStats.length - stats.statsLimit })}
        </button>
      )}
      {!total && (
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', gap: 8,
          color: 'var(--color-text-tertiary)', padding: 20 }}>
          <Tags size={28} style={{ opacity: 0.2 }} />
          <span style={{ fontSize: 11, opacity: 0.6 }}>{t('tagEditor.loadTagsHint')}</span>
        </div>
      )}
      {total > 0 && !stats.filteredStats.length && (
        <div style={{ padding: 20, fontSize: 11, textAlign: 'center', color: 'var(--color-text-tertiary)' }}>
          {t(stats.globalSearch ? 'tagEditor.noMatch' : 'tagEditor.noTags')}
        </div>
      )}
    </div>
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 6, padding: '6px 12px', fontSize: 10,
      color: 'var(--color-text-tertiary)', borderTop: '1px solid var(--color-border)' }}>
      <span style={{ color: stats.selectedTags.size ? '#a78bfa' : undefined }}>
        {t(stats.selectedTags.size ? 'tagEditor.selected' : 'tagEditor.nTagTypes', { n: stats.selectedTags.size || stats.tagStats.length })}
      </span>
      <span>{stats.taggedCount}/{total} {t('tagEditor.tagged')}</span>
    </div>
    {progress && (
      <div style={{ padding: '5px 12px', fontSize: 10, display: 'flex', alignItems: 'center', gap: 8,
        borderTop: '1px solid var(--color-border)', background: 'rgba(96,165,250,0.04)' }}>
        <span style={{ color: '#60a5fa', fontWeight: 600 }}>{t('tagEditor.translateProgress')}</span>
        <div style={{ flex: 1, height: 3, borderRadius: 2, background: 'var(--color-border)', overflow: 'hidden' }}>
          <div className="tag-stat-bar" style={{ width: `${progress.total ? progress.current / progress.total * 100 : 0}%`, height: '100%',
            background: progressDone ? '#4ade80' : 'linear-gradient(90deg,#7c5cfc,#00d4ff)' }} />
        </div>
        <span style={{ color: progressDone ? '#4ade80' : 'var(--color-text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>
          {progress.current}/{progress.total}
        </span>
      </div>
    )}
  </>;
}
