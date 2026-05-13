import { useState, useRef, useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';

interface TagSuggestion {
  name: string;
  category: number;
  post_count: number;
  aliases: string;
  translated: string | null;
}

const CATEGORY_COLORS: Record<number, string> = {
  0: '#60a5fa', // general - blue
  1: '#f87171', // artist - red
  3: '#a78bfa', // copyright - purple
  4: '#34d399', // character - green
  5: '#fbbf24', // meta - yellow
};

const CATEGORY_LABELS: Record<number, string> = {
  0: 'general',
  1: 'artist',
  3: 'copyright',
  4: 'character',
  5: 'meta',
};

const formatCount = (n: number) => {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
};

interface TagAutocompleteProps {
  placeholder?: string;
  onSelect: (tag: string) => void;
  /** If true, clear the input after selection */
  clearOnSelect?: boolean;
  /** Style overrides for the input */
  inputStyle?: React.CSSProperties;
  /** Class for the input */
  inputClassName?: string;
  /** If true, stay open after select (for multi-add) */
  keepOpen?: boolean;
  /** Auto focus */
  autoFocus?: boolean;
  /** onBlur handler */
  onBlur?: () => void;
  /** onKeyDown passthrough */
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
}

export default function TagAutocomplete({
  placeholder,
  onSelect,
  clearOnSelect = true,
  inputStyle,
  inputClassName = 'form-input',
  keepOpen = false,
  autoFocus = false,
  onBlur,
  onKeyDown,
}: TagAutocompleteProps) {
  const { t } = useTranslation();
  const resolvedPlaceholder = placeholder || t('common.searchTags');
  const [query, setQuery] = useState('');
  const [suggestions, setSuggestions] = useState<TagSuggestion[]>([]);
  const [showDropdown, setShowDropdown] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [dropdownPos, setDropdownPos] = useState<{ top: number; left: number; width: number; flipUp: boolean }>({ top: 0, left: 0, width: 0, flipUp: false });
  const inputRef = useRef<HTMLInputElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  // 计算下拉位置（基于 input 的 getBoundingClientRect）
  const updateDropdownPosition = useCallback(() => {
    if (!inputRef.current) return;
    const rect = inputRef.current.getBoundingClientRect();
    const dropdownMaxH = 320;
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const flipUp = spaceBelow < dropdownMaxH && rect.top > spaceBelow;
    const minDropdownW = 360;
    const dropdownW = Math.max(rect.width, minDropdownW);
    // 确保不超出视口右边
    let left = rect.left;
    if (left + dropdownW > window.innerWidth - 8) {
      left = Math.max(8, window.innerWidth - dropdownW - 8);
    }
    setDropdownPos({
      top: flipUp ? rect.top - 4 : rect.bottom + 4,
      left,
      width: dropdownW,
      flipUp,
    });
  }, []);

  const search = useCallback(async (q: string) => {
    if (q.length < 1) {
      setSuggestions([]);
      return;
    }
    try {
      const targetLang = localStorage.getItem('translate_target_lang') || 'zh-CN';
      const results = await invoke<TagSuggestion[]>('search_tags', { query: q, limit: 10, targetLang });
      setSuggestions(results);
      setShowDropdown(results.length > 0);
      setActiveIndex(-1);
    } catch {
      setSuggestions([]);
    }
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const val = e.target.value;
    setQuery(val);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => search(val.trim()), 120);
  };

  const handleSelect = (tag: TagSuggestion) => {
    onSelect(tag.name);
    if (clearOnSelect) setQuery('');
    if (!keepOpen) {
      setShowDropdown(false);
      setSuggestions([]);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (showDropdown && suggestions.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setActiveIndex(prev => Math.min(prev + 1, suggestions.length - 1));
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setActiveIndex(prev => Math.max(prev - 1, -1));
        return;
      }
      if (e.key === 'Enter' && activeIndex >= 0) {
        e.preventDefault();
        handleSelect(suggestions[activeIndex]);
        return;
      }
      if (e.key === 'Tab' && activeIndex >= 0) {
        e.preventDefault();
        handleSelect(suggestions[activeIndex]);
        return;
      }
    }
    // Enter without selection: use raw input
    if (e.key === 'Enter' && query.trim()) {
      e.preventDefault();
      onSelect(query.trim().replace(/\s+/g, '_'));
      if (clearOnSelect) setQuery('');
      setShowDropdown(false);
      return;
    }
    if (e.key === 'Escape') {
      setShowDropdown(false);
    }
    onKeyDown?.(e);
  };

  // 下拉打开时更新位置，并监听滚动/resize
  useEffect(() => {
    if (!showDropdown) return;
    updateDropdownPosition();
    const onScrollOrResize = () => updateDropdownPosition();
    window.addEventListener('scroll', onScrollOrResize, true);
    window.addEventListener('resize', onScrollOrResize);
    return () => {
      window.removeEventListener('scroll', onScrollOrResize, true);
      window.removeEventListener('resize', onScrollOrResize);
    };
  }, [showDropdown, suggestions, updateDropdownPosition]);

  // Scroll active item into view
  useEffect(() => {
    if (activeIndex >= 0 && dropdownRef.current) {
      const item = dropdownRef.current.children[activeIndex] as HTMLElement;
      if (item) item.scrollIntoView({ block: 'nearest' });
    }
  }, [activeIndex]);

  // Close dropdown on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node) &&
          inputRef.current && !inputRef.current.contains(e.target as Node)) {
        setShowDropdown(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  // Portal 渲染的下拉菜单
  const dropdown = showDropdown && suggestions.length > 0 && createPortal(
    <div
      ref={dropdownRef}
      style={{
        position: 'fixed',
        top: dropdownPos.flipUp ? undefined : dropdownPos.top,
        bottom: dropdownPos.flipUp ? window.innerHeight - dropdownPos.top : undefined,
        left: dropdownPos.left,
        width: dropdownPos.width,
        minWidth: 360,
        zIndex: 99999,
        background: 'var(--color-bg-secondary)',
        border: '1px solid var(--color-border)',
        borderRadius: 8,
        boxShadow: '0 8px 32px rgba(0,0,0,0.35)',
        maxHeight: 320,
        overflowY: 'auto',
        padding: 4,
      }}
    >
      {suggestions.map((tag, i) => {
        const color = CATEGORY_COLORS[tag.category] || '#60a5fa';
        const isActive = i === activeIndex;
        return (
          <div
            key={tag.name}
            onClick={() => handleSelect(tag)}
            onMouseEnter={() => setActiveIndex(i)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '6px 10px',
              borderRadius: 6,
              cursor: 'pointer',
              background: isActive ? 'rgba(124,92,252,0.1)' : 'transparent',
              transition: 'background 0.1s',
            }}
          >
            {/* Category dot */}
            <div style={{
              width: 6, height: 6, borderRadius: '50%',
              background: color, flexShrink: 0,
            }} />
            {/* Tag name */}
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 11, fontWeight: 500, color: 'var(--color-text-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {tag.name.replace(/_/g, ' ')}
              </div>
              {tag.translated && (
                <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', marginTop: 1 }}>
                  {tag.translated}
                </div>
              )}
            </div>
            {/* Category label */}
            <span style={{
              fontSize: 8, padding: '1px 5px', borderRadius: 4,
              background: `${color}15`, color, fontWeight: 600,
              flexShrink: 0, textTransform: 'uppercase',
            }}>
              {CATEGORY_LABELS[tag.category] || 'other'}
            </span>
            {/* Post count */}
            <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', flexShrink: 0, minWidth: 30, textAlign: 'right' }}>
              {formatCount(tag.post_count)}
            </span>
          </div>
        );
      })}
    </div>,
    document.body
  );

  return (
    <div style={{ position: 'relative', width: '100%' }}>
      <input
        ref={inputRef}
        className={inputClassName}
        value={query}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        onFocus={() => { if (suggestions.length > 0) setShowDropdown(true); }}
        onBlur={() => {
          // Delay hiding to allow click on dropdown
          setTimeout(() => {
            if (!dropdownRef.current?.contains(document.activeElement)) {
              setShowDropdown(false);
              onBlur?.();
            }
          }, 150);
        }}
        placeholder={resolvedPlaceholder}
        autoFocus={autoFocus}
        style={{ fontSize: 11, height: 30, ...inputStyle }}
        autoComplete="off"
      />
      {dropdown}
    </div>
  );
}
