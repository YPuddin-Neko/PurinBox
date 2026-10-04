import { useState, type CSSProperties } from 'react';
import { Plus, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import TagAutocomplete from './TagAutocomplete';
import { dedupeTags, getTagChipColor, normalizeEditableTag, replaceTagAtIndex, splitTagInput } from '../utils/tagText';
import type { TagFieldDragBinding } from '../hooks/useTagFieldDrag';
import '../styles/tags.css';

/**
 * 一组标签：双击改名、× 删除、+ 添加（逗号分隔可一次加多个）。
 * 拖动排序由外层的 useTagFieldDrag 提供，传入 fieldDrag 才能拖。
 * color 是 JSON 字段的颜色；不传时按标签名取色（TXT）。
 */
export default function TagChipList({ values, onChange, onRename, translations = {}, normalize = normalizeEditableTag, color, fieldDrag }: {
  values: string[];
  onChange: (values: string[]) => void;
  /** 双击改名生效后回调（原标签、新标签） */
  onRename?: (from: string, to: string) => void;
  translations?: Record<string, string>;
  normalize?: (value: string) => string;
  color?: string;
  fieldDrag?: TagFieldDragBinding;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<number | 'add' | null>(null);
  const dragging = fieldDrag?.dragging ?? null;
  const dropIndex = fieldDrag?.insertionIndex ?? null;
  const rename = (index: number, raw: string) => {
    setEditing(null);
    const next = replaceTagAtIndex(values, index, raw, normalize);
    if (next === values) return;
    onChange(next);
    onRename?.(values[index], normalize(raw));
  };
  const chipStyle = (tag: string, index: number): CSSProperties => color
    ? { background: `${color}1a`, borderColor: `${color}40`, color, opacity: dragging === index ? 0.35 : 1 }
    : { background: getTagChipColor(tag).bg, borderColor: getTagChipColor(tag).bd, color: getTagChipColor(tag).tx, opacity: dragging === index ? 0.35 : 1 };
  const editorHeight = color ? 24 : 26;

  return (
    <div className={`tag-chip-list${color ? ' tag-chip-list-field' : ''}`} data-tag-field={fieldDrag?.field}
      style={{ '--tag-color': color ?? '#4ade80', outline: color && dropIndex != null ? `1px dashed ${color}` : undefined, outlineOffset: 2 } as CSSProperties}>
      {values.map((tag, index) => editing === index ? (
        <div key={index} style={{ width: Math.max(60, Math.min(200, tag.length * 7 + 24)), maxWidth: '100%' }}>
          <TagAutocomplete autoFocus initialValue={tag} onSelect={raw => rename(index, raw)} placeholder={t('tagEditor.inputTag')}
            inputStyle={{ fontSize: 11, height: editorHeight, border: 'none', background: 'var(--color-bg-input)', padding: '0 8px' }}
            onBlur={() => setEditing(null)} onKeyDown={e => { if (e.key === 'Escape') setEditing(null); }} />
        </div>
      ) : (
        <div key={index} className="tag-chip" data-tag-index={index} style={chipStyle(tag, index)}
          onPointerDown={e => fieldDrag?.onPointerDown(e, index, tag)}
          onDoubleClick={e => { if (!(e.target as HTMLElement).closest('button')) setEditing(index); }}>
          {(dropIndex === index || dropIndex === values.length && index === values.length - 1) && (
            <span className="tag-chip-drop" style={dropIndex === index ? { left: -3 } : { right: -3 }} />
          )}
          <span style={{ overflowWrap: 'anywhere' }}>
            {tag}
            {translations[tag] && <span style={{ color: 'var(--color-text-tertiary)', fontSize: 10, marginLeft: 3 }}>({translations[tag]})</span>}
          </span>
          <button type="button" className="tag-chip-remove" aria-label={`${t('tagEditor.removeTag')}: ${tag}`}
            onClick={() => onChange(values.filter((_, i) => i !== index))}>
            <X size={color ? 8 : 9} />
          </button>
        </div>
      ))}
      {editing === 'add' ? (
        <div style={{ flex: '1 0 80px', minWidth: 60, maxWidth: 200 }}>
          <TagAutocomplete autoFocus keepOpen placeholder={t('tagEditor.inputTag')}
            inputStyle={{ fontSize: 11, height: editorHeight, border: 'none', background: 'transparent', padding: '0 4px' }}
            onSelect={raw => onChange(dedupeTags([...values, ...splitTagInput(raw, normalize)]).tags)}
            onBlur={() => setEditing(null)} onKeyDown={e => { if (e.key === 'Escape') setEditing(null); }} />
        </div>
      ) : values.length === 0 ? (
        <button type="button" className="tag-chip-empty" onClick={() => setEditing('add')}>
          {t(color ? 'jsonTag.noTagData' : 'tagManager.noTagsClick')}
        </button>
      ) : (
        <button type="button" className="tag-chip-add" aria-label={t('tagEditor.addTag')} onClick={() => setEditing('add')}>
          <Plus size={color ? 10 : 11} />
        </button>
      )}
    </div>
  );
}
