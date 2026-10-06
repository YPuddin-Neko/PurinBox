import { useState, type ReactNode } from 'react';
import { FolderOpen, RefreshCw, Search } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import Pager, { clampPage } from './ui/Pager';
import ThumbImage from './ThumbImage';
import '../styles/tags.css';

type Filter = 'all' | 'tagged' | 'untagged';
type Item = { path: string; filename: string; _i: number; dirty?: boolean };

const PAGE_SIZE = 30;
const FILTER_LABELS: Record<Filter, string> = { all: 'tagEditor.filterAll', untagged: 'tagEditor.filterUntagged', tagged: 'tagEditor.filterTagged' };

/** 标签编辑左栏：搜索、已标/空标筛选、分页的缩略图网格 */
export default function ImageGridColumn<T extends Item>({ width, items, total, tagged, search, onSearch, filter, onFilter, selected, onSelect, onRefresh, loading, badge }: {
  width: number; items: T[]; total: number; tagged: number; search: string; onSearch: (s: string) => void;
  filter: Filter; onFilter: (f: Filter) => void; selected: number; onSelect: (index: number) => void;
  onRefresh?: () => void; loading?: boolean; badge: (item: T) => ReactNode;
}) {
  const { t } = useTranslation();
  const [page, setPage] = useState(0);
  const pages = Math.max(1, Math.ceil(items.length / PAGE_SIZE));
  const current = clampPage(page, pages);
  const counts: Record<Filter, number> = { all: total, tagged, untagged: total - tagged };
  return (
    <div className="tag-card" style={{ width, minWidth: 160, maxWidth: 500, flexShrink: 0 }}>
      <div style={{ padding: 8, borderBottom: '1px solid var(--color-border)' }}>
        <div style={{ display: 'flex', gap: 4, marginBottom: 6 }}>
          <div style={{ position: 'relative', flex: 1, minWidth: 0 }}>
            <Search size={13} style={{ position: 'absolute', left: 8, top: '50%', transform: 'translateY(-50%)', color: 'var(--color-text-tertiary)' }} />
            <input className="form-input" aria-label={t('common.search')} placeholder={t('common.search')} value={search}
              onChange={e => { onSearch(e.target.value); setPage(0); }} style={{ paddingLeft: 28, fontSize: 11, height: 30 }} />
          </div>
          <button className="btn btn-ghost btn-sm" onClick={onRefresh} disabled={!onRefresh || loading} title={t('common.refresh')}
            style={{ width: 30, height: 30, padding: 0, flexShrink: 0 }}>
            <RefreshCw style={{ width: 13, height: 13, animation: loading ? 'spin 1s linear infinite' : undefined }} />
          </button>
        </div>
        <div style={{ display: 'flex', gap: 4 }}>
          {(['all', 'untagged', 'tagged'] as const).map(value => (
            <button key={value} onClick={() => { onFilter(value); setPage(0); }}
              style={{ flex: 1, padding: '3px 0', fontSize: 10, fontWeight: 500, borderRadius: 6,
                color: filter === value ? '#a78bfa' : 'var(--color-text-tertiary)',
                background: filter === value ? 'rgba(124,92,252,0.15)' : 'transparent',
                border: `1px solid ${filter === value ? 'rgba(124,92,252,0.25)' : 'transparent'}` }}>
              {t(FILTER_LABELS[value])} {counts[value]}
            </button>
          ))}
        </div>
      </div>
      <div className="image-grid-perf" style={{ flex: 1, overflowY: 'auto', padding: 6 }}>
        {!total ? (
          <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 8,
            fontSize: 11, color: 'var(--color-text-tertiary)' }}>
            <FolderOpen size={32} style={{ opacity: 0.2 }} /><span style={{ opacity: 0.6 }}>{t('tagEditor.loadFolderHint')}</span>
          </div>
        ) : (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,minmax(0,1fr))', gap: 4 }}>
            {items.slice(current * PAGE_SIZE, (current + 1) * PAGE_SIZE).map(item => (
              <button key={item.path} className="tag-image-grid-item" type="button" title={item.filename} aria-label={item.filename}
                onClick={() => onSelect(item._i)}
                style={{ position: 'relative', padding: 0, aspectRatio: '1', borderRadius: 8, overflow: 'hidden',
                  border: `2px solid ${selected === item._i ? 'var(--color-accent-primary)' : 'transparent'}`,
                  boxShadow: selected === item._i ? '0 0 0 1px rgba(124,92,252,0.3)' : 'none', background: 'var(--color-bg-input)' }}>
                <ThumbImage path={item.path} alt={item.filename} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                {badge(item) != null && (
                  <span style={{ position: 'absolute', bottom: 2, right: 2, minWidth: 14, height: 14, display: 'flex', alignItems: 'center',
                    justifyContent: 'center', padding: '0 3px', borderRadius: 7, background: item.dirty ? 'rgba(239,68,68,0.9)' : 'rgba(124,92,252,0.85)',
                    fontSize: 8, fontWeight: 700, color: '#fff' }}>
                    {badge(item)}
                  </span>
                )}
              </button>
            ))}
          </div>
        )}
      </div>
      {total > 0 && (
        <div style={{ padding: '4px 10px', borderTop: '1px solid var(--color-border)', color: 'var(--color-text-tertiary)',
          display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: 10 }}>
          <span>{t(items.length === total ? 'tagEditor.nImages' : 'tagEditor.nOfTotal', { n: items.length, total })}</span>
          {pages > 1 && <Pager className="tag-image-grid-pager" page={page} pages={pages} onChange={setPage} size="sm" style={{ gap: 4 }} formatLabel={(p, n) => `${p}/${n}`} />}
        </div>
      )}
    </div>
  );
}
