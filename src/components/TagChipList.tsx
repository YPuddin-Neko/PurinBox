import { useRef, useState, type CSSProperties, type PointerEvent } from 'react';
import { Plus, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import TagAutocomplete from './TagAutocomplete';
import { dedupeTags, getTagChipColor, normalizeEditableTag, replaceTagAtIndex, splitTagInput } from '../utils/tagText';
import type { TagFieldDragBinding } from '../hooks/useTagFieldDrag';
import '../styles/tags.css';

export default function TagChipList({ values, onChange, translations = {}, normalize = normalizeEditableTag, color, fieldDrag }: {
  values: string[]; onChange: (values: string[]) => void; translations?: Record<string, string>;
  normalize?: (value: string) => string; color?: string; fieldDrag?: TagFieldDragBinding;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<number | 'add' | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [target, setTarget] = useState<{ index: number; after: boolean } | null>(null);
  const elements = useRef<(HTMLDivElement | null)[]>([]);
  const drag = useRef<{ index: number; x: number; y: number; id: number; element: HTMLElement; active: boolean; target: { index: number; after: boolean } | null } | null>(null);
  const move = (e: PointerEvent) => {
    const current = drag.current;
    if (!current) return;
    if (!current.active && Math.abs(e.clientX - current.x) + Math.abs(e.clientY - current.y) > 5) {
      current.active = true; setDragging(current.index);
      current.element.setPointerCapture(current.id);
    }
    if (!current.active) return;
    for (let i = 0; i < values.length; i++) {
      const rect = elements.current[i]?.getBoundingClientRect();
      if (rect && i !== current.index && e.clientX >= rect.left && e.clientX <= rect.right && e.clientY >= rect.top && e.clientY <= rect.bottom) {
        current.target = { index: i, after: e.clientX > rect.left + rect.width / 2 };
        setTarget(current.target); return;
      }
    }
  };
  const end = (cancel = false) => {
    const current = drag.current;
    if (current?.active && current.target && !cancel) {
      let index = current.target.index + (current.target.after ? 1 : 0);
      if (current.index < index) index--;
      const next = [...values], [tag] = next.splice(current.index, 1);
      next.splice(index, 0, tag); onChange(next);
    }
    if (current?.element.hasPointerCapture(current.id)) current.element.releasePointerCapture(current.id);
    drag.current = null; setDragging(null); setTarget(null);
  };
  const activeIndex = fieldDrag ? fieldDrag.dragging : dragging;
  const dropIndex = fieldDrag?.insertionIndex;
  return <div className={`tag-chip-list${color ? ' tag-chip-list-field' : ''}`} data-tag-field={fieldDrag?.field}
    style={{ '--tag-color': color ?? '#4ade80',
    outline: fieldDrag?.insertionIndex != null ? `1px dashed ${color ?? 'var(--color-accent-primary)'}` : undefined, outlineOffset: 2 } as CSSProperties}
    onPointerMove={fieldDrag ? undefined : move} onPointerUp={fieldDrag ? undefined : () => end()} onPointerCancel={fieldDrag ? undefined : () => end(true)}>
    {values.map((tag, index) => editing === index ? <div key={index} style={{ width: Math.max(60, Math.min(200, tag.length * 7 + 24)), maxWidth: '100%' }}>
      <TagAutocomplete autoFocus initialValue={tag} onSelect={raw => { onChange(replaceTagAtIndex(values, index, raw, normalize)); setEditing(null); }}
        inputStyle={{ fontSize: 11, height: color ? 24 : 26, border: 'none', background: 'var(--color-bg-input)', padding: '0 8px' }}
        onBlur={() => setEditing(null)} onKeyDown={e => { if (e.key === 'Escape') setEditing(null); }} />
    </div> : <div key={index} className="tag-chip" ref={el => { elements.current[index] = el; }} data-tag-index={index}
      onPointerDown={e => {
        if (fieldDrag) { fieldDrag.onPointerDown(e, index, tag); return; }
        if (e.button !== 0 || e.detail > 1 || (e.target as HTMLElement).closest('button')) return;
        // Delay capture until actual dragging: early capture/preventDefault breaks WebView2 double-click editing.
        drag.current = { index, x: e.clientX, y: e.clientY, id: e.pointerId, element: e.currentTarget, active: false, target: null };
      }}
      onDoubleClick={e => { if (!(e.target as HTMLElement).closest('button')) setEditing(index); }}
      style={{ background: color ? `${color}1a` : getTagChipColor(tag).bg, borderColor: color ? `${color}40` : getTagChipColor(tag).bd,
        color: color ?? getTagChipColor(tag).tx, opacity: activeIndex === index ? 0.35 : 1 }}>
      {(fieldDrag ? dropIndex === index || dropIndex === values.length && index === values.length - 1 : target?.index === index) && <span style={{ position: 'absolute', top: 0, bottom: 0, width: 2, background: 'var(--color-accent-primary)', [fieldDrag ? dropIndex === index ? 'left' : 'right' : target?.after ? 'right' : 'left']: -3, pointerEvents: 'none' }} />}
      <span style={{ overflowWrap: 'anywhere' }}>{tag}{translations[tag] && <span style={{ color: 'var(--color-text-tertiary)', fontSize: 10, marginLeft: 3 }}>({translations[tag]})</span>}</span>
      <button type="button" className="tag-chip-remove" title={t('tagManager.deleteSelected')} aria-label={`${t('tagManager.deleteSelected')}: ${tag}`}
        onClick={() => onChange(values.filter((_, i) => i !== index))}><X size={color ? 8 : 9} /></button>
    </div>)}
    {editing === 'add' ? <div style={{ flex: '1 0 80px', minWidth: 60, maxWidth: 200 }}><TagAutocomplete autoFocus keepOpen
      inputStyle={{ fontSize: 11, height: color ? 24 : 26, border: 'none', background: 'transparent', padding: '0 4px' }}
      onSelect={raw => onChange(dedupeTags([...values, ...splitTagInput(raw, normalize)]).tags)}
      onBlur={() => setEditing(null)} onKeyDown={e => { if (e.key === 'Escape') setEditing(null); }} /></div>
      : values.length === 0 ? <button type="button" className="tag-chip-empty" onClick={() => setEditing('add')}>{t(color ? 'jsonTag.noTagData' : 'tagManager.noTagsClick')}</button>
      : <button type="button" className="tag-chip-add" title={t('tagManager.batchAdd')} aria-label={t('tagManager.batchAdd')}
        onClick={() => setEditing('add')}><Plus size={color ? 10 : 11} /></button>}
  </div>;
}
