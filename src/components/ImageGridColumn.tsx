import { useState, type ReactNode } from 'react';
import { FolderOpen, RefreshCw, Search } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import Pager, { clampPage } from './ui/Pager';
import ThumbImage from './ThumbImage';

type Filter = 'all' | 'tagged' | 'untagged';
type Item = { path: string; filename: string; _i: number; dirty?: boolean };
export default function ImageGridColumn<T extends Item>({ width, items, total, tagged, search, onSearch, filter, onFilter, selected, onSelect, onRefresh, loading, badge }: {
  width: number; items: T[]; total: number; tagged: number; search: string; onSearch: (s: string) => void;
  filter: Filter; onFilter: (f: Filter) => void; selected: number; onSelect: (index: number) => void;
  onRefresh?: () => void; loading?: boolean; badge: (item: T) => ReactNode;
}) {
  const { t } = useTranslation();
  const [page, setPage] = useState(0);
  const pages = Math.max(1, Math.ceil(items.length / 30));
  const current = clampPage(page, pages);
  return <div style={{ width, minWidth: 160, maxWidth: 500, flexShrink: 0, display: 'flex', flexDirection: 'column', background: 'var(--color-bg-secondary)', borderRadius: 8, border: '1px solid var(--color-border)', overflow: 'hidden' }}>
    <div style={{ padding: 8, borderBottom: '1px solid var(--color-border)' }}>
      <div style={{ display: 'flex', gap: 4, marginBottom: 6 }}>
        <div style={{ position: 'relative', flex: 1, minWidth: 0 }}><Search size={13} style={{ position: 'absolute', left: 8, top: 8 }} />
          <input className="form-input" aria-label={t('common.search')} placeholder={t('common.search')} value={search}
            onChange={e => { onSearch(e.target.value); setPage(0); }} style={{ paddingLeft: 28, fontSize: 11, height: 30 }} />
        </div>
        <button className="btn btn-ghost btn-sm" onClick={onRefresh} disabled={!onRefresh || loading} title={t('common.refresh')} style={{ padding: 5 }}><RefreshCw size={13} /></button>
      </div>
      <div style={{ display: 'flex', gap: 4 }}>
        {(['all', 'untagged', 'tagged'] as const).map(value => <button key={value} onClick={() => { onFilter(value); setPage(0); }}
          style={{ flex: 1, padding: '3px 0', fontSize: 10, borderRadius: 6, color: filter === value ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', background: filter === value ? 'var(--color-bg-input)' : 'transparent' }}>
          {t(`naturalLang.${value}`)} {value === 'all' ? total : value === 'tagged' ? tagged : total - tagged}
        </button>)}
      </div>
    </div>
    <div className="image-grid-perf" style={{ flex: 1, overflowY: 'auto', padding: 6 }}>
      {!total ? <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 8, fontSize: 11, color: 'var(--color-text-tertiary)' }}><FolderOpen size={32} />{t('tagManager.loadFolderHint')}</div>
        : <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,minmax(0,1fr))', gap: 4 }}>
          {items.slice(current * 30, (current + 1) * 30).map(item => <button key={item.path} type="button" title={item.filename} aria-label={item.filename} onClick={() => onSelect(item._i)}
            style={{ position: 'relative', padding: 0, aspectRatio: '1', borderRadius: 8, overflow: 'hidden', border: `2px solid ${selected === item._i ? 'var(--color-accent-primary)' : 'transparent'}`, background: 'var(--color-bg-input)' }}>
            <ThumbImage path={item.path} alt={item.filename} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
            {badge(item) && <span style={{ position: 'absolute', bottom: 2, right: 2, minWidth: 14, padding: '0 3px', borderRadius: 6, background: item.dirty ? '#ef4444' : 'var(--color-accent-primary)', fontSize: 9, color: '#fff' }}>{badge(item)}</span>}
          </button>)}
        </div>}
    </div>
    {total > 0 && <Pager page={page} pages={pages} onChange={setPage} size="sm" footer><span style={{ fontSize: 10 }}>{items.length}/{total}</span></Pager>}
  </div>;
}
