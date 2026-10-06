import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Loader2, Languages, FileText } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useTagTranslation } from '../hooks/useTagTranslation';
import { useDragResize } from '../hooks/useDragResize';
import { settleSaved } from '../utils/tagSave';
import ImageGridColumn from './ImageGridColumn';
import { ImagePreviewPane, PanelSaveButton, ResizeHandle, TagPanelHeader } from './TagEditorLayout';

type ImageItem = { filename: string; path: string; caption: string; dirty: boolean; };

interface Props {
  images: ImageItem[];
  setImages: React.Dispatch<React.SetStateAction<ImageItem[]>>;
  onRefresh?: () => void;
  /** 保存失败时的提示 */
  onError: (message: string) => void;
}

export default function NaturalLangTab({ images, setImages, onRefresh, onError }: Props) {
  const { t } = useTranslation();
  const [selectedIdx, setSelectedIdx] = useState(-1);
  const [searchText, setSearchText] = useState('');
  const [filterMode, setFilterMode] = useState<'all' | 'tagged' | 'untagged'>('all');
  const [savingSingle, setSavingSingle] = useState(false);

  const col1 = useDragResize({ initial: 220, min: 160, max: 500 });
  const col3 = useDragResize({ initial: 320, min: 200, max: 500, direction: -1 });
  const { translate, translating } = useTagTranslation();
  const [translation, setTranslation] = useState<{ path: string; caption: string; text: string } | null>(null);

  const cur = selectedIdx >= 0 && selectedIdx < images.length ? images[selectedIdx] : null;
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
    const sent = new Map([[cur.path, cur.caption]]);
    setSavingSingle(true);
    try {
      await invoke('save_caption_file', { imagePath: cur.path, content: cur.caption });
      setImages(previous => settleSaved(previous, sent, [], image => image.caption));
    } catch (e) {
      onError(`${t('tagEditor.saveFailed')}: ${String(e)}`);
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
    } catch (error) { setTranslation({ path, caption, text: t('tagEditor.translateFail') + ': ' + String(error) }); }
  };
  const translatedText = translation?.path === cur?.path && translation?.caption === cur?.caption ? translation?.text : '';

  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
      <ImageGridColumn width={col1.size} items={filtered} total={images.length} tagged={taggedN}
        search={searchText} onSearch={setSearchText} filter={filterMode} onFilter={setFilterMode}
        selected={selectedIdx} onSelect={setSelectedIdx} onRefresh={onRefresh} badge={image => image.caption.trim() ? '✓' : null} />

      <ResizeHandle axis="x" onMouseDown={col1.onMouseDown} />

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>
        <ImagePreviewPane path={cur?.path} filename={cur?.filename} index={selectedIdx} total={images.length}
          onPrev={goPrev} onNext={goNext} style={{ flex: 3, minHeight: 80 }} />
      </div>

      <ResizeHandle axis="x" onMouseDown={col3.onMouseDown} />

      <div className="tag-card" style={{ width: col3.size, minWidth: 200, maxWidth: 500, flexShrink: 0 }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          <TagPanelHeader icon={FileText} color="#60a5fa" title={t('naturalLang.captionContent')}>
            <PanelSaveButton saving={savingSingle} disabled={!cur || !cur.dirty} onClick={handleSaveSingle} />
          </TagPanelHeader>
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
              style={{ flex: 1, resize: 'none', fontSize: 12, fontFamily: 'monospace', lineHeight: 1.6, border: 'none', outline: 'none',
                boxShadow: 'none', background: 'transparent', color: 'var(--color-text-primary)', padding: 0, width: '100%' }}
            />
          </div>
        </div>

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', borderTop: '1px solid var(--color-border)' }}>
          <TagPanelHeader icon={Languages} color="#4ade80" title={t('naturalLang.translation')}>
            <button className="btn btn-primary tag-header-btn" onClick={handleTranslate} disabled={!cur || !cur.caption.trim() || translating}>
              {translating
                ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} />
                : <Languages style={{ width: 10, height: 10 }} />}
              {t('naturalLang.translate')}
            </button>
          </TagPanelHeader>
          <div style={{ flex: 1, padding: '12px 14px', overflowY: 'auto', fontSize: 12, lineHeight: 1.7, color: 'var(--color-text-primary)',
            whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
            {translatedText || <span style={{ color: 'var(--color-text-tertiary)', fontStyle: 'italic' }}>{t('naturalLang.clickToTranslate')}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}
