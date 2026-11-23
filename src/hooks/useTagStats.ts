import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react';
import { splitTagInput, tagKey } from '../utils/tagText';

export function useTagStats(tagLists: readonly string[][], folderPath: string, translations: Record<string, string>) {
  const [globalSearch, setGlobalSearch] = useState('');
  const [tagSortBy, setTagSortBy] = useState<'freq' | 'name'>('freq');
  const [tagSortDir, setTagSortDir] = useState<'asc' | 'desc'>('desc');
  const [tagListMode, setTagListMode] = useState<'all' | 'common'>('all');
  const [selectedTags, setSelectedTags] = useState<Set<string>>(new Set());
  const [statsLimit, setStatsLimit] = useState(300);
  const lastClicked = useRef('');
  const taggedCount = tagLists.filter(tags => tags.length > 0).length;
  const tagStats = useMemo(() => {
    const counts = new Map<string, number>();
    tagLists.forEach(tags => new Set(tags).forEach(tag => counts.set(tag, (counts.get(tag) ?? 0) + 1)));
    return [...counts.entries()].sort((a, b) => b[1] - a[1]);
  }, [tagLists]);
  const filteredStats = useMemo(() => {
    const queries = splitTagInput(globalSearch, tagKey);
    return tagStats.filter(([tag, count]) => {
      if (tagListMode === 'common' && count !== taggedCount) return false;
      if (!queries.length) return true;
      const name = tagKey(tag);
      const translated = tagKey(translations[tag] ?? '');
      return queries.some(query => name.includes(query) || translated.includes(query));
    })
      .sort((a, b) => (tagSortBy === 'freq' ? a[1] - b[1] : a[0].localeCompare(b[0])) * (tagSortDir === 'desc' ? -1 : 1));
  }, [tagStats, globalSearch, tagListMode, taggedCount, translations, tagSortBy, tagSortDir]);
  useEffect(() => { setStatsLimit(300); }, [globalSearch, tagListMode, folderPath, tagSortBy, tagSortDir]);
  useEffect(() => { setSelectedTags(new Set()); setGlobalSearch(''); lastClicked.current = ''; }, [folderPath]);
  const toggleTagSelect = (tag: string, e: MouseEvent, filtering: boolean) => {
    const anchor = lastClicked.current;
    const additive = filtering || e.ctrlKey || e.metaKey;
    const shift = e.shiftKey;
    if (!shift) lastClicked.current = tag;
    setSelectedTags(prev => {
      if (shift && anchor && prev.size) {
        const tags = filteredStats.map(([value]) => value), a = tags.indexOf(anchor), b = tags.indexOf(tag);
        if (a >= 0 && b >= 0) return new Set([...(additive ? prev : []), ...tags.slice(Math.min(a, b), Math.max(a, b) + 1)]);
      }
      if (additive) { const next = new Set(prev); if (next.has(tag)) next.delete(tag); else next.add(tag); return next; }
      return prev.has(tag) && prev.size === 1 ? new Set() : new Set([tag]);
    });
  };
  return { globalSearch, setGlobalSearch, tagSortBy, setTagSortBy, tagSortDir, setTagSortDir, tagListMode, setTagListMode,
    selectedTags, setSelectedTags, statsLimit, setStatsLimit, taggedCount, tagStats, filteredStats, toggleTagSelect };
}
