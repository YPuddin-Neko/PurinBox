import { useRef, useState, type PointerEvent } from 'react';
import { Plus, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import TagAutocomplete from './TagAutocomplete';
import { dedupeTags, getTagChipColor, normalizeEditableTag, replaceTagAtIndex, splitTagInput } from '../utils/tagText';

export default function TagChipList({ values, onChange, translations = {}, normalize = normalizeEditableTag, color }: {
  values: string[]; onChange: (values: string[]) => void; translations?: Record<string, string>;
  normalize?: (value: string) => string; color?: string;
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
  return <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, alignItems: 'center', minHeight: 26, touchAction: 'none' }}
    onPointerMove={move} onPointerUp={() => end()} onPointerCancel={() => end(true)}>
    {values.map((tag, index) => editing === index ? <div key={index} style={{ width: 180, maxWidth: '100%' }}>
      <TagAutocomplete autoFocus initialValue={tag} onSelect={raw => { onChange(replaceTagAtIndex(values, index, raw, normalize)); setEditing(null); }}
        onBlur={() => setEditing(null)} onKeyDown={e => { if (e.key === 'Escape') setEditing(null); }} />
    </div> : <div key={index} ref={el => { elements.current[index] = el; }}
      onPointerDown={e => {
        if (e.button !== 0 || e.detail > 1 || (e.target as HTMLElement).closest('button')) return;
        // Delay capture until actual dragging: early capture/preventDefault breaks WebView2 double-click editing.
        drag.current = { index, x: e.clientX, y: e.clientY, id: e.pointerId, element: e.currentTarget, active: false, target: null };
      }}
      onDoubleClick={e => { if (!(e.target as HTMLElement).closest('button')) setEditing(index); }}
      style={{ position: 'relative', display: 'inline-flex', alignItems: 'center', gap: 3, padding: '3px 8px', borderRadius: 8,
        background: getTagChipColor(tag).bg, border: `1px solid ${getTagChipColor(tag).bd}`, color: color ?? getTagChipColor(tag).tx,
        fontSize: 11, maxWidth: '100%', cursor: 'grab', userSelect: 'none', opacity: dragging === index ? 0.35 : 1 }}>
      {target?.index === index && <span style={{ position: 'absolute', top: 0, bottom: 0, width: 2, background: 'var(--color-accent-primary)', [target.after ? 'right' : 'left']: -3 }} />}
      <span style={{ overflowWrap: 'anywhere' }}>{tag}{translations[tag] && <span style={{ color: 'var(--color-text-tertiary)', marginLeft: 3 }}>({translations[tag]})</span>}</span>
      <button type="button" title={t('tagManager.deleteSelected')} aria-label={`${t('tagManager.deleteSelected')}: ${tag}`}
        onClick={() => onChange(values.filter((_, i) => i !== index))} style={{ display: 'flex', border: 0, padding: 0, background: 'transparent', color: 'inherit', flexShrink: 0 }}><X size={12} /></button>
    </div>)}
    {editing === 'add' ? <div style={{ width: 180, maxWidth: '100%' }}><TagAutocomplete autoFocus keepOpen
      onSelect={raw => onChange(dedupeTags([...values, ...splitTagInput(raw, normalize)]).tags)}
      onBlur={() => setEditing(null)} onKeyDown={e => { if (e.key === 'Escape') setEditing(null); }} /></div>
      : <button type="button" className="btn btn-ghost btn-sm" title={t('tagManager.batchAdd')} aria-label={t('tagManager.batchAdd')}
        onClick={() => setEditing('add')} style={{ padding: 3, height: 24, minWidth: 24 }}><Plus size={13} /></button>}
  </div>;
}
