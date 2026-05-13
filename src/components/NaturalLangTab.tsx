import { useState, useRef, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  Save, ChevronLeft, ChevronRight, Search,
  Image as ImageIcon, Loader2, Languages, FileText, FolderOpen, RefreshCw
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

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

  // 列宽拖拽
  const [col1W, setCol1W] = useState(220);
  const [col3W, setCol3W] = useState(320);
  const resizeRef = useRef<{ col: 'col1' | 'col3'; startX: number; startW: number } | null>(null);
  const handleResizeStart = useCallback((col: 'col1' | 'col3', e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = col === 'col1' ? col1W : col3W;
    resizeRef.current = { col, startX, startW };
    const onMove = (ev: MouseEvent) => {
      if (!resizeRef.current) return;
      const delta = ev.clientX - resizeRef.current.startX;
      const newW = Math.max(160, Math.min(500, resizeRef.current.startW + (resizeRef.current.col === 'col1' ? delta : -delta)));
      if (resizeRef.current.col === 'col1') setCol1W(newW);
      else setCol3W(newW);
    };
    const onUp = () => {
      resizeRef.current = null;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
  }, [col1W, col3W]);

  // 翻译
  const [translatedText, setTranslatedText] = useState('');
  const [translating, setTranslating] = useState(false);

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

  // 保存单个
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

  // 翻译当前描述
  const handleTranslate = async () => {
    if (!cur || !cur.caption.trim()) return;
    const provider = localStorage.getItem('translate_provider') || 'google';
    const enabled = localStorage.getItem('translate_enabled') === 'true';
    if (!enabled) {
      setTranslatedText(t('naturalLang.enableTranslationFirst'));
      return;
    }
    setTranslating(true);
    try {
      const result = await invoke<{ translations: { source: string; translated: string }[]; cached_count: number; translated_count: number }>('translate_tags', {
        tags: [cur.caption],
        targetLang: localStorage.getItem('translate_target_lang') || 'zh-CN',
        provider,
        baiduAppid: localStorage.getItem('baidu_appid') || '',
        baiduKey: localStorage.getItem('baidu_key') || '',
        youdaoAppKey: localStorage.getItem('youdao_app_key') || '',
        youdaoAppSecret: localStorage.getItem('youdao_app_secret') || '',
        bingKey: localStorage.getItem('bing_key') || '',
        bingRegion: localStorage.getItem('bing_region') || '',
        skipCache: true,
        translateMode: 'text',
      });
      if (result.translations.length > 0) {
        setTranslatedText(result.translations[0].translated);
      }
    } catch (e: any) {
      setTranslatedText(`${t('naturalLang.translateFailed')}: ${String(e)}`);
    } finally {
      setTranslating(false);
    }
  };

  // 选中图片变化时清空翻译
  useEffect(() => {
    setTranslatedText('');
  }, [selectedIdx]);

  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
      {/* ─ Col1: Images ─ */}
      <div style={{ width: col1W, minWidth: 160, maxWidth: 500, flexShrink: 0, display: 'flex', flexDirection: 'column', background: 'var(--color-bg-secondary)', borderRadius: 12, border: '1px solid var(--color-border)', overflow: 'hidden' }}>
        <div style={{ padding: 8, borderBottom: '1px solid var(--color-border)' }}>
          <div style={{ display: 'flex', gap: 4, position: 'relative', marginBottom: 6 }}>
            <div style={{ position: 'relative', flex: 1 }}>
              <Search style={{ position: 'absolute', left: 8, top: '50%', transform: 'translateY(-50%)', width: 13, height: 13, color: 'var(--color-text-tertiary)' }} />
              <input className="form-input" placeholder={t('common.search')} value={searchText} onChange={e => setSearchText(e.target.value)} style={{ paddingLeft: 28, fontSize: 11, height: 30 }} />
            </div>
            <button className="btn btn-ghost btn-sm" onClick={() => onRefresh?.()} disabled={!onRefresh} title={t('common.refresh')} style={{ width: 30, height: 30, padding: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}><RefreshCw style={{ width: 13, height: 13 }} /></button>
          </div>
          <div style={{ display: 'flex', gap: 4 }}>
            {([{ k: 'all' as const, l: t('naturalLang.all'), n: images.length }, { k: 'untagged' as const, l: t('naturalLang.untagged'), n: images.length - taggedN }, { k: 'tagged' as const, l: t('naturalLang.tagged'), n: taggedN }]).map(f => (
              <button key={f.k} onClick={() => setFilterMode(f.k)} style={{ flex: 1, padding: '3px 0', borderRadius: 6, fontSize: 10, fontWeight: 500, background: filterMode === f.k ? 'rgba(124,92,252,0.15)' : 'transparent', color: filterMode === f.k ? '#a78bfa' : 'var(--color-text-tertiary)', border: filterMode === f.k ? '1px solid rgba(124,92,252,0.25)' : '1px solid transparent' }}>{f.l} {f.n}</button>
            ))}
          </div>
        </div>
        <div style={{ flex: 1, overflowY: 'auto', padding: 6 }}>
          {images.length === 0 ? (
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', gap: 8, color: 'var(--color-text-tertiary)' }}>
              <FolderOpen style={{ width: 32, height: 32, opacity: 0.2 }} />
              <span style={{ fontSize: 11, opacity: 0.6 }}>{t('naturalLang.loadFolder')}</span>
            </div>
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 4 }}>
              {filtered.map(img => { const sel = img._i === selectedIdx; const src = convertFileSrc(img.path); return (
                <div key={img._i} onClick={() => setSelectedIdx(img._i)} style={{ position: 'relative', aspectRatio: '1', borderRadius: 8, overflow: 'hidden', cursor: 'pointer', border: `2px solid ${sel ? '#7c5cfc' : 'transparent'}`, boxShadow: sel ? '0 0 0 1px rgba(124,92,252,0.3)' : 'none', transition: 'all 0.15s', background: 'var(--color-bg-input)' }}>
                  <img src={src} alt={img.filename} style={{ width: '100%', height: '100%', objectFit: 'cover' }} loading="lazy" />
                  {img.caption.trim().length > 0 && <div style={{ position: 'absolute', bottom: 2, right: 2, minWidth: 14, height: 14, borderRadius: 7, padding: '0 3px', background: img.dirty ? 'rgba(239,68,68,0.9)' : 'rgba(124,92,252,0.85)', fontSize: 8, color: '#fff', fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>✓</div>}
                </div>
              ); })}
            </div>
          )}
        </div>
        {images.length > 0 && <div style={{ padding: '6px 10px', borderTop: '1px solid var(--color-border)', fontSize: 10, color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{filtered.length === images.length ? `${images.length} ${t('naturalLang.images')}` : `${filtered.length} / ${images.length}`}</div>}
      </div>

      {/* resize handle 1 */}
      <div onMouseDown={e => handleResizeStart('col1', e)} style={{ width: 6, cursor: 'col-resize', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }} title={t('common.dragResize')}>
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
              <img src={imgSrc} alt={cur.filename} draggable={false} style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain', pointerEvents: 'none' }} />
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, color: 'var(--color-text-tertiary)' }}>
                <ImageIcon style={{ width: 56, height: 56, opacity: 0.2 }} />
                <span style={{ fontSize: 12, opacity: 0.6 }}>{images.length === 0 ? t('naturalLang.loadFolderToShow') : t('naturalLang.selectToPreview')}</span>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* resize handle 2 */}
      <div onMouseDown={e => handleResizeStart('col3', e)} style={{ width: 6, cursor: 'col-resize', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }} title={t('common.dragResize')}>
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
            {translatedText || <span style={{ color: 'var(--color-text-tertiary)', fontStyle: 'italic' }}>{t('naturalLang.clickToTranslate')}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}
