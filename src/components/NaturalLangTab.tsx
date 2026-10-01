import { useTagTranslation } from '../hooks/useTagTranslation';
import { useDragResize } from '../hooks/useDragResize';
import ImageGridColumn from './ImageGridColumn';
import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Save, ChevronLeft, ChevronRight, Image as ImageIcon, Loader2, Languages, FileText } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ImageLightbox from './ImageLightbox';
import ThumbImage from './ThumbImage';

type ImageItem = { filename: string; path: string; caption: string; dirty: boolean; };

const phdr: React.CSSProperties = { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '10px 14px', borderBottom: '1px solid var(--color-border)', flexShrink: 0 };
const ptitle: React.CSSProperties = { fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)', textTransform: 'uppercase', letterSpacing: '0.5px' };

interface Props {
  images: ImageItem[];
  setImages: React.Dispatch<React.SetStateAction<ImageItem[]>>;
  onRefresh?: () => void;
}

export default function NaturalLangTab({ images, setImages, onRefresh }: Props) {
  const { t } = useTranslation();
  const [selectedIdx, setSelectedIdx] = useState(-1);
  const [searchText, setSearchText] = useState('');
  const [filterMode, setFilterMode] = useState<'all' | 'tagged' | 'untagged'>('all');
  const [savingSingle, setSavingSingle] = useState(false);
  const [showLargePreview, setShowLargePreview] = useState(false);

  const col1 = useDragResize({ initial: 220, min: 160, max: 500 });
  const col3 = useDragResize({ initial: 320, min: 200, max: 500, direction: -1 });
  const col1W = col1.size, col3W = col3.size;
  const handleResizeStart = (column: 'col1' | 'col3', e: React.MouseEvent) => (column === 'col1' ? col1 : col3).onMouseDown(e);
  const { translate, translating } = useTagTranslation();
  const [translation, setTranslation] = useState<{ path: string; caption: string; text: string } | null>(null);

  const cur = selectedIdx >= 0 && selectedIdx < images.length ? images[selectedIdx] : null;
  const imgSrc = cur ? convertFileSrc(cur.path) : '';
  const taggedN = images.filter(i => i.caption.trim().length > 0).length;

  const filtered = images.map((img, _i) => ({ ...img, _i })).filter(img => {
    if (filterMode === 'tagged' && img.caption.trim().length === 0) return false;
    if (filterMode === 'untagged' && img.caption.trim().length > 0) return false;
    if (searchText && !img.filename.toLowerCase().includes(searchText.toLowerCase())) return false;
    return true;
  });

  const goPrev = useCallback(() => setSelectedIdx(i => Math.max(0, i - 1)), []);
  const goNext = useCallback(() => setSelectedIdx(i => Math.min(images.length - 1, i + 1)), [images.length]);

  const handleSaveSingle = async () => {
    if (!cur) return;
    setSavingSingle(true);
    try {
      await invoke('save_caption_file', { imagePath: cur.path, content: cur.caption });
      setImages(p => p.map((img, i) => i === selectedIdx ? { ...img, dirty: false } : img));
    } catch (e: any) {
      console.error(e);
    } finally {
      setSavingSingle(false);
    }
  };

  const handleTranslate = async () => {
    if (!cur || !cur.caption.trim()) return;
    const { path, caption } = cur;
    if (localStorage.getItem('translate_enabled') !== 'true') {
      setTranslation({ path, caption, text: t('naturalLang.enableTranslationFirst') }); return;
    }
    try {
      const result = await translate([caption], 'text');
      if (result) setTranslation({ path, caption, text: result.translations[0]?.translated ?? '' });
    } catch (error) { setTranslation({ path, caption, text: t('naturalLang.translateFailed') + ': ' + String(error) }); }
  };
  const translatedText = translation?.path === cur?.path && translation?.caption === cur?.caption ? translation?.text : '';

  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
      <ImageGridColumn width={col1W} items={filtered} total={images.length} tagged={taggedN}
        search={searchText} onSearch={setSearchText} filter={filterMode} onFilter={setFilterMode}
        selected={selectedIdx} onSelect={setSelectedIdx} onRefresh={onRefresh} badge={image => image.caption.trim() ? '✓' : null} />

      {/* resize handle 1 */}
      <div onMouseDown={e => handleResizeStart('col1', e)} style={{ width: 6, cursor: 'col-resize', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
        <div style={{ width: 2, height: 32, borderRadius: 1, background: 'var(--color-border)', transition: 'background 0.15s' }} />
      </div>

      {/* ─ Col2: Preview + Caption Editor ─ */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>
        {/* preview */}
        <div style={{ flex: 3, display: 'flex', flexDirection: 'column', background: 'var(--color-bg-secondary)', borderRadius: 12, border: '1px solid var(--color-border)', overflow: 'hidden', minHeight: 80 }}>
          <div style={phdr}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <ImageIcon style={{ width: 14, height: 14, color: '#7c5cfc' }} />
              <span style={ptitle}>{t('naturalLang.preview')}</span>
              {cur && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 400 }}>{cur.filename}</span>}
            </div>
            {images.length > 0 && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <button className="btn btn-ghost btn-sm" onClick={goPrev} disabled={selectedIdx <= 0} style={{ width: 26, height: 26, padding: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 6 }}><ChevronLeft style={{ width: 14, height: 14 }} /></button>
                <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', minWidth: 50, textAlign: 'center' }}>{selectedIdx + 1}/{images.length}</span>
                <button className="btn btn-ghost btn-sm" onClick={goNext} disabled={selectedIdx >= images.length - 1} style={{ width: 26, height: 26, padding: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 6 }}><ChevronRight style={{ width: 14, height: 14 }} /></button>
              </div>
            )}
          </div>
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'rgba(0,0,0,0.15)', minHeight: 0, overflow: 'hidden' }}>
            {cur ? (
              <ThumbImage path={cur.path} maxEdge={1024} alt={cur.filename} draggable={false} onClick={() => setShowLargePreview(true)} style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain', cursor: 'zoom-in' }} />
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, color: 'var(--color-text-tertiary)' }}>
                <ImageIcon style={{ width: 56, height: 56, opacity: 0.2 }} />
                <span style={{ fontSize: 12, opacity: 0.6 }}>{images.length === 0 ? '' : t('naturalLang.selectToPreview')}</span>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* resize handle 2 */}
      <div onMouseDown={e => handleResizeStart('col3', e)} style={{ width: 6, cursor: 'col-resize', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
        <div style={{ width: 2, height: 32, borderRadius: 1, background: 'var(--color-border)', transition: 'background 0.15s' }} />
      </div>

      {/* ─ Col3: Caption + Translation ─ */}
      <div style={{ width: col3W, minWidth: 200, maxWidth: 500, flexShrink: 0, display: 'flex', flexDirection: 'column', background: 'var(--color-bg-secondary)', borderRadius: 12, border: '1px solid var(--color-border)', overflow: 'hidden' }}>
        {/* 描述内容（可编辑） */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          <div style={phdr}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <FileText style={{ width: 14, height: 14, color: '#60a5fa' }} />
              <span style={ptitle}>{t('naturalLang.captionContent')}</span>
            </div>
            <button className="btn btn-primary" style={{ fontSize: 10, gap: 4, height: 24, padding: '0 10px' }} disabled={!cur || !cur.dirty || savingSingle} onClick={handleSaveSingle}>
              {savingSingle ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} /> : <Save style={{ width: 10, height: 10 }} />} {t('common.save')}
            </button>
          </div>
          <div style={{ flex: 1, padding: '10px 14px', display: 'flex', flexDirection: 'column' }}>
            <textarea
              className="form-input"
              value={cur?.caption || ''}
              onChange={e => {
                if (selectedIdx < 0) return;
                const val = e.target.value;
                setImages(p => p.map((img, i) => i === selectedIdx ? { ...img, caption: val, dirty: true } : img));
              }}
              disabled={!cur}
              placeholder={cur ? t('naturalLang.inputCaption') : t('naturalLang.selectToEdit')}
              style={{ flex: 1, resize: 'none', fontSize: 12, fontFamily: 'monospace', lineHeight: 1.6, border: 'none', outline: 'none', boxShadow: 'none', background: 'transparent', color: 'var(--color-text-primary)', padding: 0, width: '100%' }}
            />
          </div>
        </div>

        {/* 翻译结果 */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', borderTop: '1px solid var(--color-border)' }}>
          <div style={phdr}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Languages style={{ width: 14, height: 14, color: '#4ade80' }} />
              <span style={ptitle}>{t('naturalLang.translation')}</span>
            </div>
            <button className="btn btn-primary" style={{ fontSize: 10, height: 24, padding: '0 10px', gap: 4 }}
              onClick={handleTranslate} disabled={!cur || !cur.caption.trim() || translating}>
              {translating ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} /> : <Languages style={{ width: 10, height: 10 }} />}
              {t('naturalLang.translate')}
            </button>
          </div>
          <div style={{ flex: 1, padding: '12px 14px', overflowY: 'auto', fontSize: 12, lineHeight: 1.7, color: 'var(--color-text-primary)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
            {translatedText}
          </div>
        </div>
      </div>

      {showLargePreview && cur && <ImageLightbox src={imgSrc} filename={cur.filename} onClose={() => setShowLargePreview(false)} />}
    </div>
  );
}
