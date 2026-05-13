import { useState, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { listen } from '@tauri-apps/api/event';
import { convertFileSrc } from '@tauri-apps/api/core';
import { AlertModal } from '../components/Modal';
import {
  Tags, FolderOpen, Save, ChevronLeft, ChevronRight, X, Plus, Search,
  Trash2, Image as ImageIcon, BarChart3, CheckCircle2, Loader2,
  Replace, Filter, ListPlus, PlusCircle, MinusCircle, Languages, RefreshCw,
  ArrowUpDown, Hash, BarChart, List
} from 'lucide-react';
import NaturalLangTab from '../components/NaturalLangTab';
import JsonTagTab, { type JsonTagTabHandle } from '../components/JsonTagTab';
import TagAutocomplete from '../components/TagAutocomplete';

type ImageItem = { filename: string; path: string; tags: string[]; dirty: boolean; };
type CaptionItem = { filename: string; path: string; caption: string; dirty: boolean; };
type TagDataset = { folder: string; images: { path: string; filename: string; tags: string[] }[] };
type CaptionDataset = { folder: string; images: { path: string; filename: string; caption: string }[] };

// 翻译映射（由 translate_tags 命令填充）
const getTranslation = (tag: string, translations: Record<string, string>) => translations[tag] || '';

// 标签配色：hash 分配，同一标签始终同色
const chipColors = [
  {bg:'rgba(124,92,252,0.10)',bd:'rgba(124,92,252,0.25)',tx:'#a78bfa'},
  {bg:'rgba(96,165,250,0.10)',bd:'rgba(96,165,250,0.25)',tx:'#60a5fa'},
  {bg:'rgba(74,222,128,0.10)',bd:'rgba(74,222,128,0.25)',tx:'#4ade80'},
  {bg:'rgba(251,191,36,0.10)',bd:'rgba(251,191,36,0.25)',tx:'#fbbf24'},
  {bg:'rgba(248,113,113,0.10)',bd:'rgba(248,113,113,0.25)',tx:'#f87171'},
  {bg:'rgba(192,132,252,0.10)',bd:'rgba(192,132,252,0.25)',tx:'#c084fc'},
  {bg:'rgba(45,212,191,0.10)',bd:'rgba(45,212,191,0.25)',tx:'#2dd4bf'},
  {bg:'rgba(251,146,60,0.10)',bd:'rgba(251,146,60,0.25)',tx:'#fb923c'},
  {bg:'rgba(236,72,153,0.10)',bd:'rgba(236,72,153,0.25)',tx:'#ec4899'},
  {bg:'rgba(132,204,22,0.10)',bd:'rgba(132,204,22,0.25)',tx:'#84cc16'},
  {bg:'rgba(14,165,233,0.10)',bd:'rgba(14,165,233,0.25)',tx:'#0ea5e9'},
  {bg:'rgba(234,179,8,0.10)',bd:'rgba(234,179,8,0.25)',tx:'#eab308'},
  {bg:'rgba(168,85,247,0.10)',bd:'rgba(168,85,247,0.25)',tx:'#a855f7'},
  {bg:'rgba(20,184,166,0.10)',bd:'rgba(20,184,166,0.25)',tx:'#14b8a6'},
  {bg:'rgba(239,68,68,0.10)',bd:'rgba(239,68,68,0.25)',tx:'#ef4444'},
  {bg:'rgba(34,197,94,0.10)',bd:'rgba(34,197,94,0.25)',tx:'#22c55e'},
];
function getChipColor(tag: string) {
  let h = 0;
  for (let i = 0; i < tag.length; i++) h = ((h << 5) - h + tag.charCodeAt(i)) | 0;
  return chipColors[Math.abs(h) % chipColors.length];
}

const phdr:React.CSSProperties={display:'flex',alignItems:'center',justifyContent:'space-between',padding:'10px 14px',borderBottom:'1px solid var(--color-border)',flexShrink:0};
const ptitle:React.CSSProperties={fontSize:12,fontWeight:700,color:'var(--color-text-primary)',textTransform:'uppercase',letterSpacing:'0.5px'};

export default function TagManagerPage() {
  const [mode, setMode] = useState<'danbooru' | 'natural' | 'json'>('danbooru');
  const [images, setImages] = useState<ImageItem[]>([]);
  const [nlImages, setNlImages] = useState<CaptionItem[]>([]);

  const [selectedIdx, setSelectedIdx] = useState(-1);
  const [searchText, setSearchText] = useState('');
  const [filterMode, setFilterMode] = useState<'all'|'tagged'|'untagged'>('all');

  const [globalSearch, setGlobalSearch] = useState('');
  const [dragIdx, setDragIdx] = useState<number|null>(null);
  const [dragOverIdx, setDragOverIdx] = useState<number|null>(null);
  const [dropSide, setDropSide] = useState<'before'|'after'>('before');
  const [savingSingle, setSavingSingle] = useState(false);
  const [alertMsg, setAlertMsg] = useState('');
  const [editingDanbooru, setEditingDanbooru] = useState(false);

  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const [folderPath, setFolderPath] = useState('');
  const jsonTabRef = useRef<JsonTagTabHandle>(null);
  const [tagSortBy, setTagSortBy] = useState<'freq'|'name'>('freq');
  const [tagSortDir, setTagSortDir] = useState<'asc'|'desc'>('desc');

  // ── 列宽拖拽 ──
  const [col1W, setCol1W] = useState(220);
  const [col3W, setCol3W] = useState(250);
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

  // ── 预览/标签区高度拖拽 ──
  const col2Ref = useRef<HTMLDivElement>(null);
  const [previewFlex, setPreviewFlex] = useState(3); // flex ratio: preview=3, tags=1
  const rowResizeRef = useRef<{ startY: number; startFlex: number } | null>(null);
  const handleRowResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    rowResizeRef.current = { startY: e.clientY, startFlex: previewFlex };
    const onMove = (ev: MouseEvent) => {
      if (!rowResizeRef.current || !col2Ref.current) return;
      const containerH = col2Ref.current.getBoundingClientRect().height;
      const delta = ev.clientY - rowResizeRef.current.startY;
      const deltaRatio = (delta / containerH) * 4; // total flex = previewFlex + 1
      const newFlex = Math.max(0.5, Math.min(6, rowResizeRef.current.startFlex + deltaRatio));
      setPreviewFlex(newFlex);
    };
    const onUp = () => {
      rowResizeRef.current = null;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'row-resize';
  }, [previewFlex]);
  // ── 右栏新功能状态 ──
  const [tagListMode, setTagListMode] = useState<'all'|'common'>('all');
  const [selectedTags, setSelectedTags] = useState<Set<string>>(new Set());
  const [showAddModal, setShowAddModal] = useState(false);
  const [addTagInput, setAddTagInput] = useState('');
  const [addPosition, setAddPosition] = useState<'start'|'end'>('end');
  const [addOverwrite, setAddOverwrite] = useState(false);
  const [showReplaceModal, setShowReplaceModal] = useState(false);
  const [replaceFrom, setReplaceFrom] = useState('');
  const [replaceTo, setReplaceTo] = useState('');
  const [replaceSearch, setReplaceSearch] = useState('');
  const [replaceDropOpen, setReplaceDropOpen] = useState(false);
  const [tagFilterActive, setTagFilterActive] = useState(false);
  const [translations, setTranslations] = useState<Record<string, string>>({});
  const [translating, setTranslating] = useState(false);
  const [translateProgress, setTranslateProgress] = useState<{ current: number; total: number } | null>(null);
  const [showTranslateBar, setShowTranslateBar] = useState(false);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── 手动翻译标签 ──
  const handleTranslate = useCallback(async () => {
    const enabled = localStorage.getItem('translate_enabled') === 'true';
    if (!enabled) return;
    const allTags = [...new Set(images.flatMap(img => img.tags))];
    if (allTags.length === 0) return;
    const provider = localStorage.getItem('translate_provider') || 'google';
    setTranslating(true);
    setTranslateProgress({ current: 0, total: allTags.length });
    setShowTranslateBar(true);
    if (hideTimerRef.current) { clearTimeout(hideTimerRef.current); hideTimerRef.current = null; }

    const unlisten = await listen<{ current: number; total: number }>('translate-progress', (e) => {
      setTranslateProgress({ current: e.payload.current, total: e.payload.total });
    });

    try {
      const result = await invoke<{ translations: { source: string; translated: string }[]; cached_count: number; translated_count: number }>('translate_tags', {
        tags: allTags,
        targetLang: localStorage.getItem('translate_target_lang') || 'zh-CN',
        provider,
        baiduAppid: localStorage.getItem('baidu_appid') || '',
        baiduKey: localStorage.getItem('baidu_key') || '',
        youdaoAppKey: localStorage.getItem('youdao_app_key') || '',
        youdaoAppSecret: localStorage.getItem('youdao_app_secret') || '',
        bingKey: localStorage.getItem('bing_key') || '',
        bingRegion: localStorage.getItem('bing_region') || '',
      });
      setTranslations(prev => {
        const next = { ...prev };
        result.translations.forEach(item => { if (item.translated) next[item.source] = item.translated; });
        return next;
      });
      setTranslateProgress({ current: allTags.length, total: allTags.length });
      hideTimerRef.current = setTimeout(() => { setShowTranslateBar(false); setTranslateProgress(null); }, 3000);
    } catch (e: any) {
      console.error('翻译失败:', e);
      setAlertMsg(`翻译失败:\n${e?.message || e}`);
      setShowTranslateBar(false);
      setTranslateProgress(null);
    } finally {
      setTranslating(false);
      unlisten();
    }
  }, [images]);

  // ── load folder ──
  const handleLoadFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择数据集文件夹' });
    if (!selected) return;
    setLoading(true);
    try {
      if (mode === 'danbooru') {
        const result = await invoke<TagDataset>('load_tag_dataset', { folder: selected as string });
        setImages(result.images.map(img => ({ ...img, dirty: false })));
        setSelectedIdx(result.images.length > 0 ? 0 : -1);
        setSearchText(''); setFilterMode('all'); setGlobalSearch('');
      } else {
        const result = await invoke<CaptionDataset>('load_caption_dataset', { folder: selected as string });
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
      if (mode === 'danbooru') {
        const result = await invoke<TagDataset>('load_tag_dataset', { folder: folderPath });
        setImages(result.images.map(img => ({ ...img, dirty: false })));
        if (selectedIdx >= result.images.length) setSelectedIdx(result.images.length > 0 ? 0 : -1);
      } else {
        const result = await invoke<CaptionDataset>('load_caption_dataset', { folder: folderPath });
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
  const taggedN = images.filter(i=>i.tags.length>0).length;
  const imgSrc = cur ? convertFileSrc(cur.path) : '';

  // ── tag stats ──
  const tagStats = useMemo(() => {
    const m:Record<string,number>={};
    images.forEach(img=>img.tags.forEach(t=>{m[t]=(m[t]||0)+1;}));
    return Object.entries(m).sort((a,b)=>b[1]-a[1]);
  }, [images]);

  const taggedCount = useMemo(() => images.filter(i => i.tags.length > 0).length, [images]);
  const filteredStats = useMemo(() => {
    const base = tagListMode === 'common'
      ? tagStats.filter(([,c]) => taggedCount > 0 && c === taggedCount)
      : tagStats;
    // 排序
    let sorted = [...base];
    if (tagSortBy === 'freq') {
      sorted.sort((a,b) => tagSortDir === 'desc' ? b[1] - a[1] : a[1] - b[1]);
    } else {
      sorted.sort((a,b) => tagSortDir === 'desc' ? b[0].localeCompare(a[0]) : a[0].localeCompare(b[0]));
    }
    if (!globalSearch) return sorted;
    const q=globalSearch.toLowerCase();
    return sorted.filter(([t])=>t.includes(q)||getTranslation(t,translations).includes(q));
  }, [tagStats, globalSearch, tagListMode, taggedCount, translations, tagSortBy, tagSortDir]);

  // ── 标签筛选图片 ──
  const filtered = useMemo(() => {
    let list = images.map((img,i) => ({...img,_i:i}));
    if (searchText) { const q=searchText.toLowerCase(); list=list.filter(img=>img.filename.toLowerCase().includes(q)||img.tags.some(t=>t.includes(q))); }
    if (filterMode==='tagged') list=list.filter(img=>img.tags.length>0);
    if (filterMode==='untagged') list=list.filter(img=>img.tags.length===0);
    // 按选中标签筛选
    if (tagFilterActive && selectedTags.size > 0) {
      list = list.filter(img => [...selectedTags].every(t => img.tags.includes(t)));
    }
    return list;
  }, [images, searchText, filterMode, tagFilterActive, selectedTags]);

  // ── 标签选择 ──
  const lastClickedTag = useRef<string>('');
  const toggleTagSelect = (tag: string, e: React.MouseEvent) => {
    const isCtrl = e.metaKey || e.ctrlKey;
    const isShift = e.shiftKey;
    setSelectedTags(prev => {
      if (isShift && lastClickedTag.current && filteredStats.length > 0) {
        // Shift: 范围选择
        const tags = filteredStats.map(([t]) => t);
        const lastIdx = tags.indexOf(lastClickedTag.current);
        const curIdx = tags.indexOf(tag);
        if (lastIdx >= 0 && curIdx >= 0) {
          const from = Math.min(lastIdx, curIdx);
          const to = Math.max(lastIdx, curIdx);
          const next = new Set(isCtrl ? prev : []);
          for (let i = from; i <= to; i++) next.add(tags[i]);
          return next;
        }
      }
      if (isCtrl) {
        // Ctrl/Cmd: 切换单个
        const next = new Set(prev);
        if (next.has(tag)) next.delete(tag); else next.add(tag);
        lastClickedTag.current = tag;
        return next;
      }
      // 普通点击: 单选
      lastClickedTag.current = tag;
      if (prev.has(tag) && prev.size === 1) return new Set();
      return new Set([tag]);
    });
    if (!e.shiftKey) lastClickedTag.current = tag;
  };

  // ── 批量添加标签 ──
  const handleBatchAdd = () => {
    const tags = addTagInput.split(',').map(t => t.trim().toLowerCase().replace(/_/g,' ')).filter(Boolean);
    if (tags.length === 0) return;
    setImages(p => p.map(img => {
      let newTags = [...img.tags];
      tags.forEach(t => {
        const idx = newTags.indexOf(t);
        if (idx >= 0) {
          if (addOverwrite) { newTags.splice(idx, 1); }
          else return;
        }
        if (addPosition === 'start') newTags.unshift(t);
        else newTags.push(t);
      });
      return { ...img, tags: newTags, dirty: true };
    }));
    setShowAddModal(false); setAddTagInput('');
  };

  // ── 批量删除标签 ──
  const handleBatchDelete = () => {
    if (selectedTags.size === 0) return;
    setImages(p => p.map(img => {
      const newTags = img.tags.filter(t => !selectedTags.has(t));
      if (newTags.length === img.tags.length) return img;
      return { ...img, tags: newTags, dirty: true };
    }));
    setSelectedTags(new Set());
  };

  // ── 标签替换 ──
  const handleReplace = () => {
    const from = replaceFrom.trim();
    const to = replaceTo.trim().toLowerCase();
    if (!from || !to || from === to) return;
    setImages(p => p.map(img => {
      const idx = img.tags.indexOf(from);
      if (idx < 0) return img;
      const newTags = [...img.tags];
      newTags[idx] = to;
      return { ...img, tags: newTags, dirty: true };
    }));
    setShowReplaceModal(false); setReplaceFrom(''); setReplaceTo('');
    setSelectedTags(prev => { const n = new Set(prev); n.delete(from); return n; });
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


  const removeTag = (tag:string) => {
    setImages(p=>p.map((img,i)=>i===selectedIdx?{...img,tags:img.tags.filter(t=>t!==tag),dirty:true}:img));
  };


  // ── drag reorder (mouse-event based, no HTML5 DnD) ──
  const chipRefs = useRef<(HTMLDivElement|null)[]>([]);
  const dragState = useRef<{active:boolean,fromIdx:number,startX:number,startY:number}>({active:false,fromIdx:-1,startX:0,startY:0});

  const moveTag = (fromIdx: number, toIdx: number) => {
    if (fromIdx === toIdx) return;
    setImages(p => p.map((img, i) => {
      if (i !== selectedIdx) return img;
      const tags = [...img.tags];
      const [moved] = tags.splice(fromIdx, 1);
      tags.splice(toIdx, 0, moved);
      return { ...img, tags, dirty: true };
    }));
  };

  const handlePointerDown = (e: React.PointerEvent, idx: number) => {
    if ((e.target as HTMLElement).closest('button')) return;
    e.preventDefault(); // 防止选中文字
    dragState.current = { active: false, fromIdx: idx, startX: e.clientX, startY: e.clientY };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    const ds = dragState.current;
    if (ds.fromIdx < 0) return;
    const dx = e.clientX - ds.startX, dy = e.clientY - ds.startY;
    if (!ds.active && Math.abs(dx) + Math.abs(dy) > 5) {
      ds.active = true;
      setDragIdx(ds.fromIdx);
    }
    if (!ds.active) return;
    const els = chipRefs.current;
    for (let i = 0; i < els.length; i++) {
      const el = els[i];
      if (!el || i === ds.fromIdx) continue;
      const r = el.getBoundingClientRect();
      if (e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom) {
        const mid = r.left + r.width / 2;
        const side = e.clientX < mid ? 'before' : 'after';
        if (dragOverIdx !== i || dropSide !== side) {
          setDragOverIdx(i);
          setDropSide(side);
        }
        return;
      }
    }
  };

  const handlePointerUp = () => {
    const ds = dragState.current;
    if (ds.active && dragOverIdx !== null && dragOverIdx !== ds.fromIdx) {
      // 计算实际插入位置
      let toIdx = dragOverIdx;
      if (dropSide === 'after') toIdx += 1;
      // 如果从前面拖到后面，splice后索引要减1
      if (ds.fromIdx < toIdx) toIdx -= 1;
      moveTag(ds.fromIdx, toIdx);
    }
    dragState.current = { active: false, fromIdx: -1, startX: 0, startY: 0 };
    setDragIdx(null);
    setDragOverIdx(null);
  };

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
    if(e.target instanceof HTMLInputElement) return;
    if(e.key==='ArrowLeft'){e.preventDefault();goPrev();}
    if(e.key==='ArrowRight'){e.preventDefault();goNext();}
  },[goPrev,goNext]);

  const dirtyCount = images.filter(i=>i.dirty).length;

  return (
    <>
    <div className="page" style={{height:'100%',display:'flex',flexDirection:'column',overflow:'hidden',gap:0}} onKeyDown={handleKeyDown} tabIndex={0}>
      {/* ═ Page Header ═ */}
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <List style={{ width: 28, height: 28, color: '#7c5cfc' }} />
          <h1 className="page-title">标签管理</h1>
        </div>
        <p className="page-subtitle">可视化编辑训练图片的标签文件，支持 Danbooru 标签和自然语言</p>
      </div>

      {/* Tab Bar + Actions */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)', flexShrink: 0 }}>
        <div style={{
          display: 'flex', gap: 2,
          background: 'var(--color-bg-card)', borderRadius: 'var(--radius-lg)',
          padding: 3, border: '1px solid var(--color-border)',
          width: 'fit-content',
        }}>
          <button onClick={()=>setMode('danbooru')} style={{
            padding: '8px 20px', borderRadius: 'var(--radius-md)', border: 'none',
            cursor: 'pointer', fontSize: 'var(--font-size-sm)', fontWeight: 600,
            transition: 'all 0.2s', fontFamily: 'inherit',
            background: mode==='danbooru' ? 'var(--color-accent-primary)' : 'transparent',
            color: mode==='danbooru' ? '#fff' : 'var(--color-text-tertiary)',
          }}>Danbooru 格式 (TXT)</button>
          <button onClick={()=>setMode('natural')} style={{
            padding: '8px 20px', borderRadius: 'var(--radius-md)', border: 'none',
            cursor: 'pointer', fontSize: 'var(--font-size-sm)', fontWeight: 600,
            transition: 'all 0.2s', fontFamily: 'inherit',
            background: mode==='natural' ? 'var(--color-accent-primary)' : 'transparent',
            color: mode==='natural' ? '#fff' : 'var(--color-text-tertiary)',
          }}>自然语言格式 (TXT)</button>
          <button onClick={()=>setMode('json')} style={{
            padding: '8px 20px', borderRadius: 'var(--radius-md)', border: 'none',
            cursor: 'pointer', fontSize: 'var(--font-size-sm)', fontWeight: 600,
            transition: 'all 0.2s', fontFamily: 'inherit',
            background: mode==='json' ? 'var(--color-accent-primary)' : 'transparent',
            color: mode==='json' ? '#fff' : 'var(--color-text-tertiary)',
          }}>Danbooru+自然语言 (JSON)</button>
        </div>
        <div style={{display:'flex',gap:8,alignItems:'center'}}>
          {mode!=='json'&&<button className="btn btn-secondary" style={{gap:6,height:34,fontSize:12}} onClick={handleLoadFolder} disabled={loading}>
            {loading?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<FolderOpen style={{width:14,height:14}} />} {loading?'加载中...':'加载文件夹'}
          </button>}
          {mode==='json'&&<button className="btn btn-secondary" style={{gap:6,height:34,fontSize:12}} onClick={()=>jsonTabRef.current?.loadFolder()} disabled={jsonTabRef.current?.loading}>
            {jsonTabRef.current?.loading?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<FolderOpen style={{width:14,height:14}} />} {jsonTabRef.current?.loading?'加载中...':'加载文件夹'}
          </button>}
          {mode==='danbooru'&&<button className="btn btn-primary" style={{gap:6,height:34,fontSize:12}} disabled={dirtyCount===0||saving} onClick={handleSaveAll}>
            {saving?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<Save style={{width:14,height:14}} />} 全部保存{dirtyCount>0?` (${dirtyCount})`:''}
          </button>}
          {mode==='natural'&&(()=>{const nlDirty=nlImages.filter(i=>i.dirty).length;return(
            <button className="btn btn-primary" style={{gap:6,height:34,fontSize:12}} disabled={nlDirty===0||saving} onClick={async()=>{
              setSaving(true);
              try{
                const items=nlImages.filter(i=>i.dirty).map(i=>({path:i.path,content:i.caption}));
                await invoke('save_all_caption_files',{items});
                setNlImages(p=>p.map(img=>img.dirty?{...img,dirty:false}:img));
              }catch(e){console.error(e);}finally{setSaving(false);}
            }}>
              {saving?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<Save style={{width:14,height:14}} />} 全部保存{nlDirty>0?` (${nlDirty})`:''}
            </button>
          );})()}
          {mode==='json'&&(()=>{const jd=jsonTabRef.current?.dirtyCount||0;return(
            <button className="btn btn-primary" style={{gap:6,height:34,fontSize:12}} disabled={jd===0||jsonTabRef.current?.saving} onClick={()=>jsonTabRef.current?.saveAll()}>
              {jsonTabRef.current?.saving?<Loader2 style={{width:14,height:14,animation:'spin 1s linear infinite'}} />:<Save style={{width:14,height:14}} />} 全部保存{jd>0?` (${jd})`:''}
            </button>
          );})()}
        </div>
      </div>

      {mode === 'danbooru' && (
      <div style={{flex:1,display:'flex',overflow:'hidden',minHeight:0}}>

        {/* ─ Col1: Images ─ */}
        <div style={{width:col1W,minWidth:160,maxWidth:500,flexShrink:0,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden'}}>
          <div style={{padding:8,borderBottom:'1px solid var(--color-border)'}}>
            <div style={{display:'flex',gap:4,position:'relative',marginBottom:6}}>
              <div style={{position:'relative',flex:1}}>
                <Search style={{position:'absolute',left:8,top:'50%',transform:'translateY(-50%)',width:13,height:13,color:'var(--color-text-tertiary)'}} />
                <input className="form-input" placeholder="搜索..." value={searchText} onChange={e=>setSearchText(e.target.value)} style={{paddingLeft:28,fontSize:11,height:30}} />
              </div>
              <button className="btn btn-ghost btn-sm" onClick={handleRefresh} disabled={!folderPath||loading} title="刷新" style={{width:30,height:30,padding:0,display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}><RefreshCw style={{width:13,height:13,animation:loading?'spin 1s linear infinite':undefined}} /></button>
            </div>
            <div style={{display:'flex',gap:4}}>
              {([{k:'all' as const,l:'全部',n:images.length},{k:'untagged' as const,l:'空标',n:images.length-taggedN},{k:'tagged' as const,l:'已标',n:taggedN}]).map(t=>(
                <button key={t.k} onClick={()=>setFilterMode(t.k)} style={{flex:1,padding:'3px 0',borderRadius:6,fontSize:10,fontWeight:500,background:filterMode===t.k?'rgba(124,92,252,0.15)':'transparent',color:filterMode===t.k?'#a78bfa':'var(--color-text-tertiary)',border:filterMode===t.k?'1px solid rgba(124,92,252,0.25)':'1px solid transparent'}}>{t.l} {t.n}</button>
              ))}
            </div>
          </div>
          <div style={{flex:1,overflowY:'auto',padding:6}}>
            {images.length===0?(
              <div style={{display:'flex',flexDirection:'column',alignItems:'center',justifyContent:'center',height:'100%',gap:8,color:'var(--color-text-tertiary)'}}>
                <FolderOpen style={{width:32,height:32,opacity:0.2}} />
                <span style={{fontSize:11,opacity:0.6}}>加载文件夹以开始</span>
              </div>
            ):(
              <div style={{display:'grid',gridTemplateColumns:'repeat(3,1fr)',gap:4}}>
                {filtered.map(img=>{const sel=img._i===selectedIdx;const src=convertFileSrc(img.path);return(
                  <div key={img._i} onClick={()=>setSelectedIdx(img._i)} style={{position:'relative',aspectRatio:'1',borderRadius:8,overflow:'hidden',cursor:'pointer',border:`2px solid ${sel?'#7c5cfc':'transparent'}`,boxShadow:sel?'0 0 0 1px rgba(124,92,252,0.3)':'none',transition:'all 0.15s',background:'var(--color-bg-input)'}}>
                    <img src={src} alt={img.filename} style={{width:'100%',height:'100%',objectFit:'cover'}} loading="lazy" />
                    {img.tags.length>0&&<div style={{position:'absolute',bottom:2,right:2,minWidth:14,height:14,borderRadius:7,padding:'0 3px',background:img.dirty?'rgba(239,68,68,0.9)':'rgba(124,92,252,0.85)',fontSize:8,color:'#fff',fontWeight:700,display:'flex',alignItems:'center',justifyContent:'center'}}>{img.tags.length}</div>}
                  </div>
                );})}
              </div>
            )}
          </div>
          {images.length>0&&<div style={{padding:'6px 10px',borderTop:'1px solid var(--color-border)',fontSize:10,color:'var(--color-text-tertiary)',textAlign:'center'}}>{filtered.length===images.length?`${images.length} 张`:`${filtered.length} / ${images.length}`}</div>}
        </div>

        {/* resize handle 1 */}
        <div onMouseDown={e=>handleResizeStart('col1',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}} title="拖拽调整宽度">
          <div style={{width:2,height:32,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
        </div>

        {/* ─ Col2: Preview + Tags ─ */}
        <div ref={col2Ref} style={{flex:1,display:'flex',flexDirection:'column',minWidth:0,overflow:'hidden'}}>
          {/* preview */}
          <div style={{flex:previewFlex,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden',minHeight:80}}>
            <div style={phdr}>
              <div style={{display:'flex',alignItems:'center',gap:8}}>
                <ImageIcon style={{width:14,height:14,color:'#7c5cfc'}} />
                <span style={ptitle}>预览</span>
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
                <img src={imgSrc} alt={cur.filename} draggable={false} style={{maxWidth:'100%',maxHeight:'100%',objectFit:'contain',pointerEvents:'none'}} />
              ):(
                <div style={{display:'flex',flexDirection:'column',alignItems:'center',gap:8,color:'var(--color-text-tertiary)'}}>
                  <ImageIcon style={{width:56,height:56,opacity:0.2}} />
                  <span style={{fontSize:12,opacity:0.6}}>{images.length===0?'加载文件夹后显示图片':'选择图片以预览'}</span>
                </div>
              )}
            </div>
          </div>

          {/* row resize handle */}
          <div onMouseDown={handleRowResizeStart} style={{height:6,cursor:'row-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}} title="拖拽调整高度">
            <div style={{width:32,height:2,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
          </div>

          {/* tag editor */}
          <div style={{flex:1,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden',minHeight:60}}>
            <div style={phdr}>
              <div style={{display:'flex',alignItems:'center',gap:8}}>
                <Tags style={{width:14,height:14,color:'#4ade80'}} />
                <span style={ptitle}>图片标签</span>
                <span style={{fontSize:10,padding:'1px 8px',borderRadius:10,background:cur?.dirty?'rgba(239,68,68,0.1)':'rgba(74,222,128,0.1)',color:cur?.dirty?'#ef4444':'#4ade80',fontWeight:600}}>{cur?.tags.length||0}</span>
              </div>
              <button className="btn btn-primary" style={{fontSize:10,gap:4,height:24,padding:'0 10px'}} disabled={!cur||!cur.dirty||savingSingle} onClick={handleSaveSingle}>
                {savingSingle?<Loader2 style={{width:10,height:10,animation:'spin 1s linear infinite'}} />:<Save style={{width:10,height:10}} />} 保存
              </button>
            </div>
            <div style={{flex:1,padding:'10px 14px',display:'flex',flexWrap:'wrap',gap:5,alignContent:'flex-start',overflowY:'auto',touchAction:'none'}}
              onPointerMove={handlePointerMove} onPointerUp={handlePointerUp}>
              {cur?.tags.map((tag,ti)=>{
                const c=getChipColor(tag);const tr=getTranslation(tag,translations);
                const isDragging=dragIdx===ti;
                const isOverBefore=dragOverIdx===ti && dropSide==='before';
                const isOverAfter=dragOverIdx===ti && dropSide==='after';
                return(
                  <div key={ti} ref={el=>{chipRefs.current[ti]=el;}} style={{position:'relative',display:'inline-flex'}}
                    onPointerDown={e=>handlePointerDown(e,ti)}>
                    {isOverBefore&&<div style={{position:'absolute',left:-3,top:2,bottom:2,width:2,borderRadius:1,background:'#7c5cfc',zIndex:1}} />}
                    <div style={{display:'inline-flex',alignItems:'center',gap:3,padding:'3px 8px 3px 8px',borderRadius:16,background:c.bg,border:`1px solid ${c.bd}`,fontSize:11,color:c.tx,lineHeight:1.2,cursor:'grab',transition:'opacity 0.12s',opacity:isDragging?0.35:1,userSelect:'none'}}>
                      <span>{tag}{tr&&<span style={{color:'var(--color-text-tertiary)',fontSize:10,marginLeft:3}}>({tr})</span>}</span>
                      <button onClick={()=>removeTag(tag)} style={{display:'flex',alignItems:'center',justifyContent:'center',width:14,height:14,borderRadius:'50%',background:'transparent',color:c.tx,opacity:0.4,transition:'all 0.12s',flexShrink:0}}
                        onMouseEnter={e=>{e.currentTarget.style.opacity='1';e.currentTarget.style.background='rgba(248,113,113,0.15)';e.currentTarget.style.color='#f87171';}}
                        onMouseLeave={e=>{e.currentTarget.style.opacity='0.4';e.currentTarget.style.background='transparent';e.currentTarget.style.color=c.tx;}}
                      ><X style={{width:9,height:9}} /></button>
                    </div>
                    {isOverAfter&&<div style={{position:'absolute',right:-3,top:2,bottom:2,width:2,borderRadius:1,background:'#7c5cfc',zIndex:1}} />}
                  </div>
                );
              })}
              {!cur&&<span style={{fontSize:11,color:'var(--color-text-tertiary)',fontStyle:'italic'}}>选择图片以编辑标签</span>}
              {cur&&cur.tags.length===0&&!editingDanbooru&&<span onClick={()=>setEditingDanbooru(true)} style={{fontSize:11,color:'var(--color-text-tertiary)',fontStyle:'italic',cursor:'pointer'}}>暂无标签，点击添加标签</span>}
              {cur&&(cur.tags.length>0)&&!editingDanbooru&&<button onClick={()=>setEditingDanbooru(true)} style={{display:'flex',alignItems:'center',justifyContent:'center',width:20,height:20,borderRadius:'50%',background:'rgba(74,222,128,0.10)',border:'1px solid rgba(74,222,128,0.25)',color:'#4ade80',cursor:'pointer',flexShrink:0,opacity:0.5,transition:'opacity 0.15s'}}
                onMouseEnter={e=>e.currentTarget.style.opacity='1'} onMouseLeave={e=>e.currentTarget.style.opacity='0.5'}
              ><Plus style={{width:11,height:11}} /></button>}
              {cur&&editingDanbooru&&<TagAutocomplete
                autoFocus
                placeholder="输入标签, Enter 添加"
                clearOnSelect={true}
                keepOpen={true}
                onSelect={(tag) => {
                  const v = tag.trim().toLowerCase().replace(/_/g, ' ');
                  if (v && cur && !cur.tags.includes(v)) {
                    setImages(p => p.map((img, i) => i === selectedIdx ? { ...img, tags: [...img.tags, v], dirty: true } : img));
                  }
                }}
                onBlur={() => setEditingDanbooru(false)}
                onKeyDown={(e) => { if (e.key === 'Escape') setEditingDanbooru(false); }}
                inputStyle={{ fontSize: 11, height: 26, border: 'none', background: 'transparent', padding: '0 6px', flex: '1 0 80px', minWidth: 80, maxWidth: 200, outline: 'none' }}
              />}
            </div>
          </div>
        </div>

        {/* resize handle 2 */}
        <div onMouseDown={e=>handleResizeStart('col3',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}} title="拖拽调整宽度">
          <div style={{width:2,height:32,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
        </div>

        {/* ─ Col3: Global Tags + Tool sidebar ─ */}
        <div style={{width:col3W,minWidth:160,maxWidth:500,flexShrink:0,display:'flex',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden'}}>
          {/* 标签列表 */}
          <div style={{flex:1,display:'flex',flexDirection:'column',overflow:'hidden'}}>
            <div style={phdr}>
              <div style={{display:'flex',alignItems:'center',gap:8}}>
                <BarChart3 style={{width:14,height:14,color:'#60a5fa'}} />
                <span style={ptitle}>{tagListMode==='common'?'公共标签':'全部标签'}</span>
                <span style={{fontSize:10,padding:'1px 8px',borderRadius:10,background:'rgba(96,165,250,0.1)',color:'#60a5fa',fontWeight:600}}>{filteredStats.length}</span>
              </div>
              <div style={{display:'flex',alignItems:'center',gap:4}}>
                <button className="btn btn-ghost btn-sm" title="翻译标签" style={{width:22,height:22,padding:0,display:'flex',alignItems:'center',justifyContent:'center',color:Object.keys(translations).length>0?'#60a5fa':undefined}} onClick={handleTranslate} disabled={images.length===0||localStorage.getItem('translate_enabled')!=='true'||translating}>
                  <Languages style={{width:12,height:12}} />
                </button>
                <button className="btn btn-ghost btn-sm" style={{fontSize:9,height:22,padding:'0 6px'}} onClick={()=>setTagListMode(m=>m==='all'?'common':'all')} disabled={images.length===0}>
                  {tagListMode==='all'?'公共':'全部'}
                </button>
              </div>
            </div>
            <div style={{padding:'8px 10px',borderBottom:'1px solid var(--color-border)'}}>  
              <div style={{display:'flex',gap:4}}>
                <div style={{position:'relative',flex:1}}>
                  <Search style={{position:'absolute',left:8,top:'50%',transform:'translateY(-50%)',width:12,height:12,color:'var(--color-text-tertiary)'}} />
                  <input className="form-input" placeholder="搜索标签..." value={globalSearch} onChange={e=>setGlobalSearch(e.target.value)} style={{paddingLeft:26,fontSize:11,height:28}} />
                </div>
                <button className="btn btn-ghost btn-sm" onClick={()=>setTagSortBy(b=>b==='freq'?'name':'freq')}
                  title={tagSortBy==='freq'?'按频次排序':'按名称排序'}
                  style={{width:28,height:28,padding:0,display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0,color:tagSortBy==='freq'?'#60a5fa':'#a78bfa'}}>
                  {tagSortBy==='freq'?<BarChart style={{width:13,height:13}} />:<Hash style={{width:13,height:13}} />}
                </button>
                <button className="btn btn-ghost btn-sm" onClick={()=>setTagSortDir(d=>d==='desc'?'asc':'desc')}
                  title={tagSortDir==='desc'?'降序':'升序'}
                  style={{width:28,height:28,padding:0,display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0,color:'var(--color-text-tertiary)'}}>
                  <ArrowUpDown style={{width:13,height:13,transform:tagSortDir==='asc'?'scaleY(-1)':undefined,transition:'transform 0.2s'}} />
                </button>
              </div>
            </div>
            {tagFilterActive&&<div style={{padding:'4px 10px',background:'linear-gradient(90deg,rgba(124,92,252,0.08),rgba(124,92,252,0.02))',borderBottom:'1px solid var(--color-border)',display:'flex',alignItems:'center',gap:6}}>
              <Filter style={{width:10,height:10,color:'#7c5cfc',flexShrink:0}} />
              <span style={{fontSize:10,color:'#a78bfa',flex:1}}>筛选中 <b>{filtered.length}</b>/{images.length}</span>
              <button onClick={()=>setTagFilterActive(false)} style={{display:'flex',alignItems:'center',justifyContent:'center',width:16,height:16,borderRadius:'50%',background:'rgba(248,113,113,0.1)',border:'none',cursor:'pointer',color:'#f87171',padding:0,flexShrink:0}}><X style={{width:8,height:8}} /></button>
            </div>}
            <div style={{flex:1,overflowY:'auto',userSelect:'none'}}>
              {filteredStats.map(([tag,count])=>{
                const c=getChipColor(tag);const pct=images.length>0?(count/images.length)*100:0;
                const inCur=cur?.tags.includes(tag);const tr=getTranslation(tag,translations);
                const isSel=selectedTags.has(tag);
                return(
                  <div key={tag} onMouseDown={e=>e.preventDefault()} onClick={e=>toggleTagSelect(tag,e)}
                    style={{display:'flex',alignItems:'center',gap:8,padding:'7px 12px',cursor:'pointer',borderBottom:'1px solid rgba(255,255,255,0.03)',
                      background:isSel?'rgba(124,92,252,0.12)':inCur?'rgba(124,92,252,0.04)':'transparent',
                      borderLeft:isSel?'2px solid #7c5cfc':'2px solid transparent',transition:'all 0.12s'}}
                    onMouseEnter={e=>{if(!isSel)e.currentTarget.style.background=inCur?'rgba(124,92,252,0.08)':'var(--color-bg-hover)';}}
                    onMouseLeave={e=>{e.currentTarget.style.background=isSel?'rgba(124,92,252,0.12)':inCur?'rgba(124,92,252,0.04)':'transparent';}}
                  >
                    <div style={{flex:1,minWidth:0}}>
                      <div style={{fontSize:11,fontWeight:500,color:c.tx,overflow:'hidden',textOverflow:'ellipsis',whiteSpace:'nowrap'}}>
                        {tag}{tr&&<span style={{color:'var(--color-text-tertiary)',fontWeight:400,fontSize:10,marginLeft:4}}>{tr}</span>}
                      </div>
                      <div style={{height:3,borderRadius:2,background:'var(--color-bg-input)',marginTop:3,overflow:'hidden'}}>
                        <div style={{width:`${pct}%`,height:'100%',borderRadius:2,background:`linear-gradient(90deg,${c.bd},${c.tx})`}} />
                      </div>
                    </div>
                    <span style={{fontSize:10,color:'var(--color-text-tertiary)',minWidth:28,textAlign:'right',flexShrink:0}}>{count}</span>
                    {inCur&&<CheckCircle2 style={{width:12,height:12,color:'#4ade80',flexShrink:0}} />}
                  </div>
                );
              })}
              {images.length===0&&<div style={{display:'flex',flexDirection:'column',alignItems:'center',justifyContent:'center',height:'100%',gap:8,color:'var(--color-text-tertiary)',padding:20}}><Tags style={{width:28,height:28,opacity:0.2}} /><span style={{fontSize:11,opacity:0.6}}>加载数据集后显示标签</span></div>}
              {images.length>0&&filteredStats.length===0&&<div style={{padding:20,textAlign:'center',fontSize:11,color:'var(--color-text-tertiary)'}}>{globalSearch?'无匹配标签':tagListMode==='common'?'无公共标签':'暂无标签数据'}</div>}
            </div>
            <div style={{padding:'6px 12px',borderTop:'1px solid var(--color-border)',fontSize:10,color:'var(--color-text-tertiary)',display:'flex',justifyContent:'space-between'}}>
              {selectedTags.size>0?<span style={{color:'#a78bfa'}}>已选 {selectedTags.size} 个</span>:<span>{tagStats.length} 种标签</span>}
              <span>{taggedN}/{images.length} 已标注</span>
            </div>
            {showTranslateBar && translateProgress && (
              <div style={{padding:'5px 12px',borderTop:'1px solid var(--color-border)',display:'flex',alignItems:'center',gap:8,background:'rgba(96,165,250,0.04)'}}>
                <span style={{fontSize:10,fontWeight:600,color:'#60a5fa',flexShrink:0}}>翻译进度</span>
                <div style={{flex:1,height:3,borderRadius:2,background:'var(--color-border)',overflow:'hidden'}}>
                  <div style={{
                    width:`${translateProgress.total > 0 ? (translateProgress.current / translateProgress.total) * 100 : 0}%`,
                    height:'100%',borderRadius:2,
                    background: translateProgress.current >= translateProgress.total ? '#4ade80' : 'linear-gradient(90deg, #7c5cfc, #00d4ff)',
                    transition:'width 0.3s ease'
                  }} />
                </div>
                <span style={{fontSize:10,color: translateProgress.current >= translateProgress.total ? '#4ade80' : 'var(--color-text-tertiary)',whiteSpace:'nowrap',fontVariantNumeric:'tabular-nums'}}>
                  {translateProgress.current >= translateProgress.total ? '✓ ' : ''}{translateProgress.current}/{translateProgress.total}
                </span>
              </div>
            )}
          </div>

          {/* 工具栏 */}
          <div style={{display:'flex',flexDirection:'column',gap:2,padding:'8px 4px',borderLeft:'1px solid var(--color-border)',alignItems:'center'}}>
            {[
              {icon:<Filter style={{width:14,height:14}} />,tip:'按选中标签筛选图片（支持选中多个标签筛选）',onClick:()=>setTagFilterActive(v=>!v),disabled:images.length===0,color:tagFilterActive?'#7c5cfc':undefined},
              {icon:<Replace style={{width:14,height:14}} />,tip:'替换标签',onClick:()=>{setShowReplaceModal(true);if(selectedTags.size===1)setReplaceFrom([...selectedTags][0]);},disabled:images.length===0},
              {icon:<ListPlus style={{width:14,height:14}} />,tip:'批量添加标签',onClick:()=>setShowAddModal(true),disabled:images.length===0},
              {icon:<Trash2 style={{width:14,height:14}} />,tip:'删除选中标签(全部图片)',onClick:handleBatchDelete,disabled:selectedTags.size===0,color:selectedTags.size>0?'#f87171':undefined},
              {icon:<PlusCircle style={{width:14,height:14}} />,tip:'添加选中标签到当前图片',onClick:addSelectedToCurrent,disabled:!cur||selectedTags.size===0},
              {icon:<MinusCircle style={{width:14,height:14}} />,tip:'从当前图片移除选中标签',onClick:removeSelectedFromCurrent,disabled:!cur||selectedTags.size===0||!curHasAllSelected,color:curHasAllSelected&&selectedTags.size>0?'#f87171':undefined},
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

      {mode === 'natural' && (
        <NaturalLangTab images={nlImages} setImages={setNlImages} onRefresh={folderPath ? handleRefresh : undefined} />
      )}

      <div style={{display:mode==='json'?'flex':'none',flex:1,overflow:'hidden',minHeight:0}}>
        <JsonTagTab ref={jsonTabRef} />
      </div>

      {/* ═ 批量添加弹窗 ═ */}
      {showAddModal&&<div style={{position:'fixed',inset:0,background:'rgba(0,0,0,0.5)',zIndex:9999,display:'flex',alignItems:'center',justifyContent:'center'}} onClick={()=>setShowAddModal(false)}>
        <div style={{background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',padding:20,width:380,maxWidth:'90vw'}} onClick={e=>e.stopPropagation()}>
          <h3 style={{margin:'0 0 14px',fontSize:14,fontWeight:700,color:'var(--color-text-primary)'}}>批量添加标签</h3>
          <label style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:4,display:'block'}}>标签内容（逗号分隔）</label>
          <input className="form-input" placeholder="1girl, solo, smile" value={addTagInput}
            onChange={e=>setAddTagInput(e.target.value)}
            style={{fontSize:12,marginBottom:12}} />
          <div style={{display:'flex',gap:12,marginBottom:12}}>
            <label style={{fontSize:11,color:'var(--color-text-secondary)',display:'flex',alignItems:'center',gap:4,cursor:'pointer'}}>
              <input type="radio" checked={addPosition==='start'} onChange={()=>setAddPosition('start')} /> 最前面
            </label>
            <label style={{fontSize:11,color:'var(--color-text-secondary)',display:'flex',alignItems:'center',gap:4,cursor:'pointer'}}>
              <input type="radio" checked={addPosition==='end'} onChange={()=>setAddPosition('end')} /> 最后面
            </label>
          </div>
          <label style={{fontSize:11,color:'var(--color-text-secondary)',display:'flex',alignItems:'center',gap:4,cursor:'pointer',marginBottom:16}}>
            <input type="checkbox" checked={addOverwrite} onChange={e=>setAddOverwrite(e.target.checked)} />
            如果标签已存在，先移除再添加到指定位置
          </label>
          <div style={{display:'flex',gap:8,justifyContent:'flex-end'}}>
            <button className="btn btn-secondary" style={{height:30,fontSize:11}} onClick={()=>setShowAddModal(false)}>取消</button>
            <button className="btn btn-primary" style={{height:30,fontSize:11}} onClick={handleBatchAdd} disabled={!addTagInput.trim()}>添加到全部图片</button>
          </div>
        </div>
      </div>}

      {/* ═ 替换弹窗 ═ */}
      {showReplaceModal&&<div style={{position:'fixed',inset:0,background:'rgba(0,0,0,0.5)',zIndex:9999,display:'flex',alignItems:'center',justifyContent:'center'}} onClick={()=>{setShowReplaceModal(false);setReplaceDropOpen(false);}}>
        <div style={{background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',padding:20,width:380,maxWidth:'90vw'}} onClick={e=>e.stopPropagation()}>
          <h3 style={{margin:'0 0 14px',fontSize:14,fontWeight:700,color:'var(--color-text-primary)'}}>标签替换</h3>
          <label style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:4,display:'block'}}>原标签</label>
          <div style={{position:'relative',marginBottom:10}}>
            <input className="form-input" placeholder="搜索并选择原标签..." value={replaceDropOpen ? replaceSearch : replaceFrom}
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
                  <div style={{padding:'12px 10px',fontSize:11,color:'var(--color-text-tertiary)',textAlign:'center'}}>无匹配标签</div>
                )}
              </div>
            )}
          </div>
          <label style={{fontSize:11,color:'var(--color-text-secondary)',marginBottom:4,display:'block'}}>替换为</label>
          <input className="form-input" placeholder="新标签" value={replaceTo}
            onChange={e=>setReplaceTo(e.target.value)}
            style={{fontSize:12,marginBottom:16}} />
          <div style={{display:'flex',gap:8,justifyContent:'flex-end'}}>
            <button className="btn btn-secondary" style={{height:30,fontSize:11}} onClick={()=>{setShowReplaceModal(false);setReplaceDropOpen(false);}}>取消</button>
            <button className="btn btn-primary" style={{height:30,fontSize:11}} onClick={handleReplace} disabled={!replaceFrom.trim()||!replaceTo.trim()}>替换全部</button>
          </div>
        </div>
      </div>}
    </div>

      <AlertModal
        open={!!alertMsg}
        onClose={() => setAlertMsg('')}
        title="错误"
        message={alertMsg}
      />
    </>
  );
}
