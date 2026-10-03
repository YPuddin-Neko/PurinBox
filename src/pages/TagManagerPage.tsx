import { useTagStats } from '../hooks/useTagStats';
import { useTagTranslation } from '../hooks/useTagTranslation';
import { useDragResize } from '../hooks/useDragResize';
import { dedupeTags, splitTagInput, sameTags } from '../utils/tagText';
import TagChipList from '../components/TagChipList';
import ImageGridColumn from '../components/ImageGridColumn';
import TagStatsPanel from '../components/TagStatsPanel';
import ScopeToggle from '../components/ScopeToggle';
import SegmentedTabs from '../components/ui/SegmentedTabs';
import { useState, useRef, useCallback, useMemo, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ensureAssetScope } from '../utils/assetScope';
import { open } from '@tauri-apps/plugin-dialog';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Modal, AlertModal } from '../components/Modal';
import { Tags, FolderOpen, Save, ChevronLeft, ChevronRight, X, Trash2, Image as ImageIcon, BarChart3, Loader2, Replace, Filter, ListPlus, PlusCircle, MinusCircle, Languages, List, CopyX, Wand2 } from 'lucide-react';
import NaturalLangTab from '../components/NaturalLangTab';
import JsonTagTab, { type JsonTagTabHandle } from '../components/JsonTagTab';
import ImageLightbox from '../components/ImageLightbox';
import ThumbImage from '../components/ThumbImage';
import RecursiveScanToggle from '../components/RecursiveScanToggle';
import Checkbox from '../components/Checkbox';
import { useTranslation } from 'react-i18next';

type ImageItem = { filename: string; path: string; tags: string[]; dirty: boolean; };
type CaptionItem = { filename: string; path: string; caption: string; dirty: boolean; };
type TagDataset = { folder: string; images: { path: string; filename: string; tags: string[] }[] };
type CaptionDataset = { folder: string; images: { path: string; filename: string; caption: string }[] };

const normalizeDanbooruTag = (tag: string) => tag.trim().toLowerCase().replace(/_/g, ' ').replace(/\s+/g, ' ').trim();

// danbooru 原形：空格→下划线、括号转义为 \( \)。
// 先把已有转义剥掉再统一重转，保证幂等（已转义的不会被转成 \\(）
const toDanbooruEscaped = (tag: string) =>
  tag.trim().toLowerCase()
    .replace(/\\([()])/g, '$1')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/ /g, '_')
    .replace(/\(/g, '\\(')
    .replace(/\)/g, '\\)');

// 数据集里已有 danbooru 原形标签（带下划线或转义括号）→ 新标签沿用同种形式。
// 不然一份 txt 里 "long hair" 和 "long_hair" 两种格式混着，训练时是两个 token
const hasEscapedTags = (tags: string[]) => tags.some(t => t.includes('_') || t.includes('\\('));

const phdr:React.CSSProperties={display:'flex',alignItems:'center',justifyContent:'space-between',flexWrap:'wrap',gap:6,padding:'10px 14px',borderBottom:'1px solid var(--color-border)',flexShrink:0};
const ptitle:React.CSSProperties={fontSize:12,fontWeight:700,color:'var(--color-text-primary)',whiteSpace:'nowrap'};

export default function TagManagerPage() {
  const { t } = useTranslation();
  const [mode, setMode] = useState<'danbooru' | 'natural' | 'json'>('danbooru');
  const [images, setImages] = useState<ImageItem[]>([]);
  const [nlImages, setNlImages] = useState<CaptionItem[]>([]);

  const [selectedIdx, setSelectedIdx] = useState(-1);
  const [searchText, setSearchText] = useState('');
  const [filterMode, setFilterMode] = useState<'all'|'tagged'|'untagged'>('all');

  const [savingSingle, setSavingSingle] = useState(false);
  const [alertMsg, setAlertMsg] = useState('');
  const [showLargePreview, setShowLargePreview] = useState(false);

  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [recursive, setRecursive] = useState(false);

  const [folderPath, setFolderPath] = useState('');
  const jsonTabRef = useRef<JsonTagTabHandle>(null);
  const [jsonDirtyCount, setJsonDirtyCount] = useState(0);
  // JsonTagTab 的 loading/saving 通过回调上报为 state（渲染期读 ref 不会触发重渲染，按钮禁用态会陈旧）
  const [jsonLoading, setJsonLoading] = useState(false);
  const [jsonSaving, setJsonSaving] = useState(false);

  const col1 = useDragResize({ initial: 220, min: 160, max: 500 });
  const col3 = useDragResize({ initial: 250, min: 160, max: 500, direction: -1 });
  const previewColumn = useRef<HTMLDivElement>(null);
  const [previewFlex, setPreviewFlex] = useState(3);
  const rowResizeCleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => rowResizeCleanup.current?.(), []);
  const col1W = col1.size, col3W = col3.size;
  const handleResizeStart = (column: 'col1' | 'col3', e: React.MouseEvent) => (column === 'col1' ? col1 : col3).onMouseDown(e);
  const handleRowResizeStart = (event: React.MouseEvent) => {
    event.preventDefault();
    rowResizeCleanup.current?.();
    const startY = event.clientY;
    const height = previewColumn.current?.getBoundingClientRect().height || 1;
    const oldCursor = document.body.style.cursor;
    const move = (e: MouseEvent) => setPreviewFlex(Math.max(0.5, Math.min(6, previewFlex + (e.clientY - startY) / height * 4)));
    const stop = () => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', stop);
      document.body.style.cursor = oldCursor;
      rowResizeCleanup.current = null;
    };
    rowResizeCleanup.current = stop;
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
    document.body.style.cursor = 'row-resize';
  };
  // ── 右栏状态 ──
  const [showAddModal, setShowAddModal] = useState(false);
  const [addTagInput, setAddTagInput] = useState('');
  const [addPosition, setAddPosition] = useState<'start'|'end'>('start');
  const [addOverwrite, setAddOverwrite] = useState(false);
  /** 批量添加/删除的应用范围：当前图片 or 全部图片 */
  const [addScope, setAddScope] = useState<'current'|'all'>('all');
  const [showDeleteModal, setShowDeleteModal] = useState(false);
  const [deleteScope, setDeleteScope] = useState<'current'|'all'>('all');
  const [showReplaceModal, setShowReplaceModal] = useState(false);
  const [replaceFrom, setReplaceFrom] = useState('');
  const [replaceTo, setReplaceTo] = useState('');
  const [replaceSearch, setReplaceSearch] = useState('');
  const [replaceDropOpen, setReplaceDropOpen] = useState(false);
  const [tagFilterActive, setTagFilterActive] = useState(false);
  const { translations, translating, translateProgress, translate } = useTagTranslation();
  const tagLists = useMemo(() => images.map(image => image.tags), [images]);
  const stats = useTagStats(tagLists, folderPath, translations);
  const { selectedTags, setSelectedTags, tagListMode, setTagListMode, filteredStats, tagStats, taggedCount, setGlobalSearch } = stats;
  const handleTranslate = async () => {
    if (localStorage.getItem('translate_enabled') !== 'true') return;
    try { await translate(tagStats.map(([tag]) => tag)); }
    catch (error) { setAlertMsg(t('tagManager.translateFail') + ': ' + String(error)); }
  };

  // ── load folder ──
  const handleLoadFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('tagManager.selectFolder') });
    if (!selected) return;
    setLoading(true);
    try {
      await ensureAssetScope(selected as string);
      if (mode === 'danbooru') {
        const result = await invoke<TagDataset>('load_tag_dataset', { folder: selected as string, recursive });
        setImages(result.images.map(img => ({ ...img, dirty: false })));
        setSelectedIdx(result.images.length > 0 ? 0 : -1);
        setSearchText(''); setFilterMode('all'); setGlobalSearch('');
      } else {
        const result = await invoke<CaptionDataset>('load_caption_dataset', { folder: selected as string, recursive });
        setNlImages(result.images.map(img => ({ ...img, dirty: false })));
      }
      setFolderPath(selected as string);
    } catch (e: any) {
      console.error('加载失败:', e);
    } finally {
      setLoading(false);
    }
  };

  // ── refresh ──
  const handleRefresh = async () => {
    if (!folderPath) return;
    setLoading(true);
    try {
      await ensureAssetScope(folderPath);
      if (mode === 'danbooru') {
        const result = await invoke<TagDataset>('load_tag_dataset', { folder: folderPath, recursive });
        setImages(result.images.map(img => ({ ...img, dirty: false })));
        if (selectedIdx >= result.images.length) setSelectedIdx(result.images.length > 0 ? 0 : -1);
      } else {
        const result = await invoke<CaptionDataset>('load_caption_dataset', { folder: folderPath, recursive });
        setNlImages(result.images.map(img => ({ ...img, dirty: false })));
      }
    } catch (e: any) {
      console.error('刷新失败:', e);
    } finally {
      setLoading(false);
    }
  };

  // ── save all dirty ──
  const handleSaveAll = async () => {
    const dirtyItems = images.filter(img => img.dirty).map(img => ({ path: img.path, tags: img.tags }));
    if (dirtyItems.length === 0) return;
    setSaving(true);
    try {
      await invoke<number>('save_all_tag_files', { items: dirtyItems });
      setImages(prev => prev.map(img => img.dirty ? { ...img, dirty: false } : img));
    } catch (e: any) {
      console.error('保存失败:', e);
    } finally {
      setSaving(false);
    }
  };

  const cur = selectedIdx >= 0 && selectedIdx < images.length ? images[selectedIdx] : null;
  const imgSrc = cur ? convertFileSrc(cur.path) : '';

  // 新标签按数据集现有格式规范化：已有 danbooru 原形标签就转义，否则保持空格形式。
  // 优先看当前图片，当前图无标签时看整个数据集
  const normalizeNewTag = (raw: string, img?: ImageItem | null) => {
    const pool = img && img.tags.length > 0 ? img.tags : images.flatMap(i => i.tags);
    return hasEscapedTags(pool) ? toDanbooruEscaped(raw) : normalizeDanbooruTag(raw);
  };

  // 一键转义：全部标签转成 danbooru 原形。转义后可能撞出重复
  // （"long hair" 和 "long_hair" 归一为同一个），顺手去重
  const handleEscapeAll = () => {
    setImages(p => p.map(img => {
      const next = img.tags.map(toDanbooruEscaped).filter((t, i, arr) => arr.indexOf(t) === i);
      const changed = next.length !== img.tags.length || next.some((t, i) => t !== img.tags[i]);
      return changed ? { ...img, tags: next, dirty: true } : img;
    }));
  };

  // ── 标签筛选图片 ──
  const filtered = useMemo(() => {
    let list = images.map((img,i) => ({...img,_i:i}));
    if (searchText) { const q=searchText.toLowerCase(); list=list.filter(img=>img.filename.toLowerCase().includes(q)||img.tags.some(t=>t.includes(q))); }
    if (filterMode==='tagged') list=list.filter(img=>img.tags.length>0);
    if (filterMode==='untagged') list=list.filter(img=>img.tags.length===0);
    if (tagFilterActive && selectedTags.size > 0) {
      list = list.filter(img => [...selectedTags].every(t => img.tags.includes(t)));
    }
    return list;
  }, [images, searchText, filterMode, tagFilterActive, selectedTags]);

  // ── 批量添加标签（按范围：当前图片 / 全部图片）──
  const handleBatchAdd = () => {
    // 新标签的格式按整个数据集的现有惯例判定（与应用范围无关）
    const tags = splitTagInput(addTagInput, value => normalizeNewTag(value));
    if (tags.length === 0) return;
    if (addScope === 'current' && selectedIdx < 0) return;
    setImages(p => p.map((img, i) => {
      if (addScope === 'current' && i !== selectedIdx) return img;
      let newTags = [...img.tags];
      // 前置不能逐个 unshift：那会把输入顺序整体反转（首 token 权重敏感）
      const toInsert: string[] = [];
      tags.forEach(t => {
        const idx = newTags.indexOf(t);
        if (idx >= 0) {
          if (addOverwrite) { newTags.splice(idx, 1); }
          else return;
        }
        toInsert.push(t);
      });
      if (addPosition === 'start') newTags = [...toInsert, ...newTags];
      else newTags = [...newTags, ...toInsert];
      return sameTags(img.tags, newTags) ? img : { ...img, tags: newTags, dirty: true };
    }));
    setShowAddModal(false); setAddTagInput('');
  };

  // ── 批量删除标签（按范围：当前图片 / 全部图片）──
  const handleBatchDelete = () => {
    if (selectedTags.size === 0) return;
    if (deleteScope === 'current' && selectedIdx < 0) return;
    setImages(p => p.map((img, i) => {
      if (deleteScope === 'current' && i !== selectedIdx) return img;
      const newTags = img.tags.filter(t => !selectedTags.has(t));
      if (newTags.length === img.tags.length) return img;
      return { ...img, tags: newTags, dirty: true };
    }));
    setSelectedTags(new Set());
    setShowDeleteModal(false);
  };

  const handleDeduplicateTags = () => {
    setImages(prev => prev.map(img => {
      const result = dedupeTags(img.tags);
      return result.changed ? { ...img, tags: result.tags, dirty: true } : img;
    }));
  };

  // ── 标签替换 ──
  const handleReplace = () => {
    const from = replaceFrom.trim();
    const to = normalizeNewTag(replaceTo);
    // 匹配统一用小写比较（replaceTo 强制小写，replaceFrom 也按小写匹配）
    const fromKey = from.toLowerCase();
    if (!from || !to || fromKey === to) return;
    setImages(p => p.map(img => {
      if (!img.tags.some(t2 => t2.toLowerCase() === fromKey)) return img;
      const replaced = img.tags.map(t2 => t2.toLowerCase() === fromKey ? to : t2);
      // 替换后可能产生重复标签，去重（保序）
      return { ...img, tags: dedupeTags(replaced).tags, dirty: true };
    }));
    setShowReplaceModal(false); setReplaceFrom(''); setReplaceTo('');
    setSelectedTags(prev => { const n = new Set(prev); [...n].forEach(t2 => { if (t2.toLowerCase() === fromKey) n.delete(t2); }); return n; });
  };

  // ── 为当前图片添加/删除选中标签 ──
  const addSelectedToCurrent = () => {
    if (!cur) return;
    setImages(p => p.map((img,i) => {
      if (i !== selectedIdx) return img;
      const newTags = [...img.tags];
      selectedTags.forEach(t => { if (!newTags.includes(t)) newTags.push(t); });
      return { ...img, tags: newTags, dirty: true };
    }));
  };
  const removeSelectedFromCurrent = () => {
    if (!cur) return;
    setImages(p => p.map((img,i) => {
      if (i !== selectedIdx) return img;
      return { ...img, tags: img.tags.filter(t => !selectedTags.has(t)), dirty: true };
    }));
  };
  const curHasAllSelected = cur ? [...selectedTags].every(t => cur.tags.includes(t)) : false;

  // ── nav ──
  const goPrev = useCallback(()=>{setSelectedIdx(i=>Math.max(0,i-1));},[]);
  const goNext = useCallback(()=>{setSelectedIdx(i=>Math.min(images.length-1,i+1));},[images.length]);


  // ── save single ──
  const handleSaveSingle = async () => {
    if (!cur) return;
    setSavingSingle(true);
    try {
      await invoke('save_single_tag_file', { imagePath: cur.path, tags: cur.tags });
      setImages(p => p.map((img, i) => i === selectedIdx ? { ...img, dirty: false } : img));
    } catch (e) { console.error('保存失败:', e); }
    finally { setSavingSingle(false); }
  };

  const handleKeyDown = useCallback((e:React.KeyboardEvent)=>{
    if (mode !== 'danbooru') return;
    // 焦点在输入框、文本域或 contenteditable 里时，方向键留给编辑控件，不切换图片
    const el = e.target as HTMLElement | null;
    if(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el?.isContentEditable) return;
    if(e.key==='ArrowLeft'){e.preventDefault();goPrev();}
    if(e.key==='ArrowRight'){e.preventDefault();goNext();}
  },[goPrev,goNext,mode]);

  const dirtyCount = images.filter(i=>i.dirty).length;

  const handleSaveAllCaptions = async () => {
    setSaving(true);
    try {
      const items = nlImages.filter(i => i.dirty).map(i => ({ path: i.path, content: i.caption }));
      await invoke('save_all_caption_files', { items });
      setNlImages(p => p.map(img => img.dirty ? { ...img, dirty: false } : img));
    } catch (e) { console.error(e); } finally { setSaving(false); }
  };

  const topBar = mode === 'json'
    ? { loading: jsonLoading, saving: jsonSaving, dirty: jsonDirtyCount, onLoad: () => jsonTabRef.current?.loadFolder(), onSave: () => jsonTabRef.current?.saveAll() }
    : mode === 'natural'
      ? { loading, saving, dirty: nlImages.filter(i => i.dirty).length, onLoad: handleLoadFolder, onSave: handleSaveAllCaptions }
      : { loading, saving, dirty: dirtyCount, onLoad: handleLoadFolder, onSave: handleSaveAll };

  return (
    <>
    <div className="page" style={{height:'100%',display:'flex',flexDirection:'column',overflow:'hidden',gap:0}} onKeyDown={handleKeyDown} tabIndex={0}>
      {/* ═ Page Header ═ */}
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <List style={{ width: 28, height: 28, color: '#7c5cfc' }} />
          <h1 className="page-title">{t('tagManager.title')}</h1>
        </div>
        <p className="page-subtitle">{t('tagManager.subtitle')}</p>
      </div>

      {/* Tab Bar + Actions */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)', flexShrink: 0 }}>
        <SegmentedTabs value={mode} onChange={setMode} tabs={[
          { id: 'danbooru', label: t('tagManager.danbooruTab') },
          { id: 'natural', label: t('tagManager.naturalTab') },
          { id: 'json', label: t('tagManager.jsonTab') },
        ]} />
        <div style={{display:'flex',gap:8,alignItems:'center'}}>
          <RecursiveScanToggle checked={recursive} onChange={setRecursive} style={{ height: 34 }} />
          <button className="btn btn-secondary" style={{gap:6,height:34,fontSize:12}} onClick={topBar.onLoad} disabled={topBar.loading}>
            {topBar.loading?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<FolderOpen style={{width:14,height:14}} />} {topBar.loading?t('tagManager.loading'):t('tagManager.loadFolder')}
          </button>
          <button className="btn btn-primary" style={{gap:6,height:34,fontSize:12}} disabled={topBar.dirty===0||topBar.saving} onClick={topBar.onSave}>
            {topBar.saving?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<Save style={{width:14,height:14}} />} {t('tagManager.saveAll')}{topBar.dirty>0?` (${topBar.dirty})`:''}
          </button>
        </div>
      </div>

      {mode === 'danbooru' && (
      <div style={{flex:1,display:'flex',overflow:'hidden',minHeight:0}}>

        <ImageGridColumn key={folderPath} width={col1W} items={filtered} total={images.length} tagged={taggedCount}
          search={searchText} onSearch={setSearchText} filter={filterMode} onFilter={setFilterMode}
          selected={selectedIdx} onSelect={setSelectedIdx} onRefresh={folderPath ? handleRefresh : undefined} loading={loading}
          badge={image => image.tags.length || null} />

        {/* resize handle 1 */}
        <div onMouseDown={e=>handleResizeStart('col1',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}>
          <div style={{width:2,height:32,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
        </div>

        {/* ─ Col2: Preview + Tags ─ */}
        <div ref={previewColumn} style={{flex:1,display:'flex',flexDirection:'column',minWidth:0,overflow:'hidden'}}>
          {/* preview */}
          <div style={{flex:previewFlex,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden',minHeight:80}}>
            <div style={phdr}>
              <div style={{display:'flex',alignItems:'center',gap:8}}>
                <ImageIcon style={{width:14,height:14,color:'#7c5cfc'}} />
                <span style={ptitle}>{t('tagManager.preview')}</span>
                {cur&&<span style={{fontSize:11,color:'var(--color-text-tertiary)',fontWeight:400}}>{cur.filename}</span>}
              </div>
              {images.length>0&&(
                <div style={{display:'flex',alignItems:'center',gap:6}}>
                  <button className="btn btn-ghost btn-sm" onClick={goPrev} disabled={selectedIdx<=0} style={{width:26,height:26,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6}}><ChevronLeft style={{width:14,height:14}} /></button>
                  <span style={{fontSize:11,color:'var(--color-text-tertiary)',minWidth:50,textAlign:'center'}}>{selectedIdx+1}/{images.length}</span>
                  <button className="btn btn-ghost btn-sm" onClick={goNext} disabled={selectedIdx>=images.length-1} style={{width:26,height:26,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6}}><ChevronRight style={{width:14,height:14}} /></button>
                </div>
              )}
            </div>
            <div style={{flex:1,display:'flex',alignItems:'center',justifyContent:'center',background:'rgba(0,0,0,0.15)',minHeight:0,overflow:'hidden'}}>
              {cur?(
                <ThumbImage path={cur.path} maxEdge={1024} alt={cur.filename} draggable={false} onClick={()=>setShowLargePreview(true)} style={{maxWidth:'100%',maxHeight:'100%',objectFit:'contain',cursor:'zoom-in'}} />
              ):(
                <div style={{display:'flex',flexDirection:'column',alignItems:'center',gap:8,color:'var(--color-text-tertiary)'}}>
                  <ImageIcon style={{width:56,height:56,opacity:0.2}} />
                  <span style={{fontSize:12,opacity:0.6}}>{images.length===0?'':t('tagManager.selectToPreview')}</span>
                </div>
              )}
            </div>
          </div>

          {/* row resize handle */}
          <div onMouseDown={handleRowResizeStart} style={{height:6,cursor:'row-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}>
            <div style={{width:32,height:2,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
          </div>

          {/* tag editor */}
          <div style={{flex:1,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden',minHeight:60}}>
            <div style={phdr}>
              <div style={{display:'flex',alignItems:'center',gap:8}}>
                <Tags style={{width:14,height:14,color:'#4ade80'}} />
                <span style={ptitle}>{t('tagManager.imageTags')}</span>
                <span style={{fontSize:10,padding:'1px 8px',borderRadius:10,background:cur?.dirty?'rgba(239,68,68,0.1)':'rgba(74,222,128,0.1)',color:cur?.dirty?'#ef4444':'#4ade80',fontWeight:600}}>{cur?.tags.length||0}</span>
              </div>
              <button className="btn btn-primary" style={{fontSize:10,gap:4,height:24,padding:'0 10px'}} disabled={!cur||!cur.dirty||savingSingle} onClick={handleSaveSingle}>
                {savingSingle?<Loader2 style={{width:10,height:10,animation:'spin 1s linear infinite'}} />:<Save style={{width:10,height:10}} />} {t('tagManager.save')}
              </button>
            </div>
            <div style={{ flex: 1, padding: '10px 14px', overflowY: 'auto' }}>
              {!cur && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', fontStyle: 'italic' }}>{t('tagManager.selectToEdit')}</span>}
              {cur && <TagChipList key={cur.path} values={cur.tags} translations={translations} normalize={raw => normalizeNewTag(raw, cur)}
                onChange={tags => setImages(previous => previous.map(image => image.path === cur.path && !sameTags(image.tags, tags) ? { ...image, tags, dirty: true } : image))} />}
            </div>
          </div>
        </div>

        {/* resize handle 2 */}
        <div onMouseDown={e=>handleResizeStart('col3',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}>
          <div style={{width:2,height:32,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
        </div>

        {/* ─ Col3: Global Tags + Tool sidebar ─ */}
        <div style={{width:col3W,minWidth:160,maxWidth:500,flexShrink:0,display:'flex',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden'}}>
          {/* 标签列表 */}
          <div style={{flex:1,display:'flex',flexDirection:'column',overflow:'hidden'}}>
            <div style={phdr}>
              <div style={{display:'flex',alignItems:'center',gap:8}}>
                <BarChart3 style={{width:14,height:14,color:'#60a5fa'}} />
                <span style={ptitle}>{tagListMode==='common'?t('tagManager.commonTags'):t('tagManager.allTags')}</span>
                <span style={{fontSize:10,padding:'1px 8px',borderRadius:10,background:'rgba(96,165,250,0.1)',color:'#60a5fa',fontWeight:600}}>{filteredStats.length}</span>
              </div>
              <div style={{display:'flex',alignItems:'center',gap:4}}>
                <button className="btn btn-ghost btn-sm" title={t('tagManager.escapeAllTip')} style={{width:22,height:22,padding:0,display:'flex',alignItems:'center',justifyContent:'center'}} onClick={handleEscapeAll} disabled={images.length===0}>
                  <Wand2 style={{width:12,height:12}} />
                </button>
                <button className="btn btn-ghost btn-sm" title={t('tagManager.translateTags')} style={{width:22,height:22,padding:0,display:'flex',alignItems:'center',justifyContent:'center',color:Object.keys(translations).length>0?'#60a5fa':undefined}} onClick={handleTranslate} disabled={images.length===0||localStorage.getItem('translate_enabled')!=='true'||translating}>
                  <Languages style={{width:12,height:12}} />
                </button>
                <button className="btn btn-ghost btn-sm" style={{fontSize:9,height:22,padding:'0 6px'}} onClick={()=>setTagListMode(m=>m==='all'?'common':'all')} disabled={images.length===0}>
                  {tagListMode==='all'?t('tagManager.commonLabel'):t('tagManager.allLabel')}
                </button>
              </div>
            </div>
            <TagStatsPanel stats={stats} translations={translations} currentTags={new Set(cur?.tags ?? [])} total={images.length}
              filteredCount={filtered.length} filterActive={tagFilterActive} onClearFilter={() => setTagFilterActive(false)} progress={translateProgress} />
          </div>

          {/* 工具栏 */}
          <div style={{display:'flex',flexDirection:'column',gap:2,padding:'8px 4px',borderLeft:'1px solid var(--color-border)',alignItems:'center'}}>
            {[
              {icon:<Filter style={{width:14,height:14}} />,tip:t('tagManager.filterByTag'),onClick:()=>setTagFilterActive(v=>!v),disabled:images.length===0,color:tagFilterActive?'#7c5cfc':undefined},
              {icon:<Replace style={{width:14,height:14}} />,tip:t('tagManager.replaceTag'),onClick:()=>{setReplaceTo('');if(selectedTags.size===1){setReplaceFrom([...selectedTags][0]);}else{setReplaceFrom('');}setShowReplaceModal(true);},disabled:images.length===0},
              {icon:<ListPlus style={{width:14,height:14}} />,tip:t('tagManager.batchAdd'),onClick:()=>{setAddTagInput('');setAddPosition('start');setAddOverwrite(false);setAddScope('all');setShowAddModal(true);},disabled:images.length===0},
              {icon:<CopyX style={{width:14,height:14}} />,tip:t('tagManager.dedupeTags'),onClick:handleDeduplicateTags,disabled:images.length===0,color:'#f59e0b'},
              {icon:<Trash2 style={{width:14,height:14}} />,tip:t('tagManager.deleteSelected'),onClick:()=>{setDeleteScope('all');setShowDeleteModal(true);},disabled:selectedTags.size===0,color:selectedTags.size>0?'#f87171':undefined},
              {icon:<PlusCircle style={{width:14,height:14}} />,tip:t('tagManager.addToCurrent'),onClick:addSelectedToCurrent,disabled:!cur||selectedTags.size===0},
              {icon:<MinusCircle style={{width:14,height:14}} />,tip:t('tagManager.removeFromCurrent'),onClick:removeSelectedFromCurrent,disabled:!cur||selectedTags.size===0||!curHasAllSelected,color:curHasAllSelected&&selectedTags.size>0?'#f87171':undefined},
            ].map((item,i)=>(
              <button key={i} className="btn btn-ghost" title={item.tip} disabled={item.disabled}
                onClick={item.onClick}
                style={{width:30,height:30,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6,color:item.color}}>
                {item.icon}
              </button>
            ))}
          </div>
        </div>
      </div>
      )}

      {showLargePreview&&cur&&<ImageLightbox src={imgSrc} filename={cur.filename} onClose={()=>setShowLargePreview(false)} />}

      {mode === 'natural' && (
        <NaturalLangTab images={nlImages} setImages={setNlImages} onRefresh={folderPath ? handleRefresh : undefined} />
      )}

      <div style={{display:mode==='json'?'flex':'none',flex:1,overflow:'hidden',minHeight:0}}>
        <JsonTagTab ref={jsonTabRef} recursive={recursive} onDirtyChange={setJsonDirtyCount} onLoadingChange={setJsonLoading} onSavingChange={setJsonSaving} />
      </div>

      {/* ═ 批量添加弹窗 ═ */}
      <Modal open={showAddModal} onClose={() => setShowAddModal(false)} title={t('tagManager.batchAddTitle')}>

          <label style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:4,display:'block'}}>{t('tagManager.tagContent')}</label>
          <input className="form-input" placeholder="1girl, solo, smile" value={addTagInput}
            onChange={e=>setAddTagInput(e.target.value)}
            style={{fontSize:12,marginBottom:12}} />
          <div style={{display:'flex',gap:12,marginBottom:12}}>
            <label style={{fontSize:11,color:'var(--color-text-secondary)',display:'flex',alignItems:'center',gap:4,cursor:'pointer'}}>
              <input type="radio" checked={addPosition==='start'} onChange={()=>setAddPosition('start')} /> {t('tagManager.prepend')}
            </label>
            <label style={{fontSize:11,color:'var(--color-text-secondary)',display:'flex',alignItems:'center',gap:4,cursor:'pointer'}}>
              <input type="radio" checked={addPosition==='end'} onChange={()=>setAddPosition('end')} /> {t('tagManager.append')}
            </label>
          </div>
          <ScopeToggle value={addScope} onChange={setAddScope} hasCurrent={!!cur} />

          <Checkbox checked={addOverwrite} onChange={setAddOverwrite} size={14}
            label={t('tagManager.overwriteIfExist')}
            style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:16}} />
          <div style={{display:'flex',gap:8,justifyContent:'flex-end'}}>
            <button className="btn btn-secondary" style={{height:30,fontSize:11}} onClick={()=>setShowAddModal(false)}>{t('tagManager.cancel')}</button>
            <button className="btn btn-primary" style={{height:30,fontSize:11}} onClick={handleBatchAdd} disabled={!addTagInput.trim()||(addScope==='current'&&!cur)}>
              {addScope==='all'?t('tagManager.addToAll'):t('tagManager.addToCurrentOne')}
            </button>
          </div>
      </Modal>
      {/* ═ 批量删除弹窗 ═ */}
      <Modal open={showDeleteModal} onClose={() => setShowDeleteModal(false)} title={t('tagManager.batchDeleteTitle')}>

          <div style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:10}}>
            {t('tagManager.deleteTagsHint',{n:selectedTags.size})}
          </div>
          <div style={{display:'flex',flexWrap:'wrap',gap:4,marginBottom:12,maxHeight:96,overflowY:'auto',overscrollBehavior:'contain'}}>
            {[...selectedTags].map(tag=>(
              <span key={tag} style={{fontSize:10,padding:'1px 7px',borderRadius:4,border:'1px solid rgba(248,113,113,0.45)',background:'rgba(248,113,113,0.06)',color:'var(--color-text-secondary)'}}>{tag}</span>
            ))}
          </div>
          <ScopeToggle value={deleteScope} onChange={setDeleteScope} hasCurrent={!!cur} />

          <div style={{display:'flex',gap:8,justifyContent:'flex-end'}}>
            <button className="btn btn-secondary" style={{height:30,fontSize:11}} onClick={()=>setShowDeleteModal(false)}>{t('tagManager.cancel')}</button>
            <button className="btn btn-primary" style={{height:30,fontSize:11,background:'#ef4444',borderColor:'#ef4444'}} onClick={handleBatchDelete} disabled={deleteScope==='current'&&!cur}>
              {deleteScope==='all'?t('tagManager.deleteFromAll'):t('tagManager.deleteFromCurrentOne')}
            </button>
          </div>
      </Modal>
      {/* ═ 替换弹窗 ═ */}
      <Modal open={showReplaceModal} onClose={() => setShowReplaceModal(false)} title={t('tagManager.replaceTitle')}>

          <label style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:4,display:'block'}}>{t('tagManager.originalTag')}</label>
          <div style={{position:'relative',marginBottom:10}}>
            <input className="form-input" placeholder={t('tagManager.searchAndSelect')} value={replaceDropOpen ? replaceSearch : replaceFrom}
              onFocus={()=>{setReplaceDropOpen(true);setReplaceSearch('');}}
              onChange={e=>{setReplaceSearch(e.target.value);setReplaceDropOpen(true);}}
              style={{fontSize:12}} />
            {replaceFrom && !replaceDropOpen && (
              <button onClick={()=>{setReplaceFrom('');setReplaceSearch('');}} style={{position:'absolute',right:8,top:'50%',transform:'translateY(-50%)',background:'none',border:'none',cursor:'pointer',color:'var(--color-text-tertiary)',padding:0,display:'flex'}}>
                <X style={{width:12,height:12}} />
              </button>
            )}
            {replaceDropOpen && (
              <div style={{position:'absolute',top:'100%',left:0,right:0,maxHeight:180,overflow:'auto',background:'var(--color-bg-elevated)',border:'1px solid var(--color-border)',borderRadius:6,marginTop:2,zIndex:10,boxShadow:'var(--shadow-md)'}}>
                {tagStats
                  .filter(([tag])=>!replaceSearch || tag.includes(replaceSearch.toLowerCase()))
                  .slice(0,50)
                  .map(([tag,count])=>(
                    <div key={tag} onClick={()=>{setReplaceFrom(tag);setReplaceSearch('');setReplaceDropOpen(false);}}
                      style={{padding:'6px 10px',fontSize:11,cursor:'pointer',display:'flex',justifyContent:'space-between',alignItems:'center',
                        background:replaceFrom===tag?'rgba(124,92,252,0.1)':'transparent',
                        color:'var(--color-text-primary)'}}
                      onMouseEnter={e=>e.currentTarget.style.background='var(--color-bg-hover)'}
                      onMouseLeave={e=>e.currentTarget.style.background=replaceFrom===tag?'rgba(124,92,252,0.1)':'transparent'}>
                      <span style={{overflow:'hidden',textOverflow:'ellipsis',whiteSpace:'nowrap'}}>{tag}</span>
                      <span style={{fontSize:9,color:'var(--color-text-tertiary)',flexShrink:0,marginLeft:8}}>{count}</span>
                    </div>
                  ))}
                {tagStats.filter(([tag])=>!replaceSearch || tag.includes(replaceSearch.toLowerCase())).length===0 && (
                  <div style={{padding:'12px 10px',fontSize:11,color:'var(--color-text-tertiary)',textAlign:'center'}}>{t('tagManager.noMatch')}</div>
                )}
              </div>
            )}
          </div>
          <label style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:4,display:'block'}}>{t('tagManager.replaceTo')}</label>
          <input className="form-input" placeholder={t('tagManager.newTag')} value={replaceTo}
            onChange={e=>setReplaceTo(e.target.value)}
            style={{fontSize:12,marginBottom:16}} />
          <div style={{display:'flex',gap:8,justifyContent:'flex-end'}}>
            <button className="btn btn-secondary" style={{height:30,fontSize:11}} onClick={()=>{setShowReplaceModal(false);setReplaceDropOpen(false);}}>{t('tagManager.cancel')}</button>
            <button className="btn btn-primary" style={{height:30,fontSize:11}} onClick={handleReplace} disabled={!replaceFrom.trim()||!replaceTo.trim()}>{t('tagManager.replaceAll')}</button>
          </div>
      </Modal>    </div>

      <AlertModal
        open={!!alertMsg}
        onClose={() => setAlertMsg('')}
        title={t('tagManager.errorTitle')}
        message={alertMsg}
      />
    </>
  );
}
