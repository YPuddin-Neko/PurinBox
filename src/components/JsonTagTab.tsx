import { useState, useMemo, useCallback, useRef, forwardRef, useImperativeHandle } from 'react';
import TagAutocomplete from './TagAutocomplete';
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { open as dialogOpen } from '@tauri-apps/plugin-dialog';
import {
  FolderOpen, Save, ChevronLeft, ChevronRight, X, Plus, Search,
  Image as ImageIcon, Loader2, RefreshCw, Tags, Sparkles, Eye, Shirt, TreePine, Lock, User, Layers, Languages,
  ListPlus, ListX, BarChart3, ArrowUpDown, Hash, BarChart, CheckCircle2, Filter, Code, Trash2
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

// 完整格式类型定义
interface JsonFixed { quality?: string; series?: string; artist?: string; }
interface JsonCharacter { name: string; variant: string; }
interface JsonFromPath { appearance: string[]; }
interface JsonAiOutput { count?: string; appearance: string[]; tags: string[]; environment: string[]; nl?: string; }
interface JsonTagData { fixed: JsonFixed; character: JsonCharacter; from_path: JsonFromPath; ai_output: JsonAiOutput; }
interface JsonImageItem { path: string; filename: string; data: JsonTagData; has_json: boolean; dirty?: boolean; }
interface JsonDataset { folder: string; images: JsonImageItem[]; detected_format: string; }

type AiCatKey = 'appearance' | 'tags' | 'environment';
const AI_CATS_KEYS = [
  { key: 'appearance' as const, labelKey: 'jsonTag.appearance', icon: Shirt, color: '#c084fc', bg: 'rgba(192,132,252,0.08)', bd: 'rgba(192,132,252,0.25)' },
  { key: 'tags' as const, labelKey: 'jsonTag.tags', icon: Tags, color: '#60a5fa', bg: 'rgba(96,165,250,0.08)', bd: 'rgba(96,165,250,0.25)' },
  { key: 'environment' as const, labelKey: 'jsonTag.environment', icon: TreePine, color: '#34d399', bg: 'rgba(52,211,153,0.08)', bd: 'rgba(52,211,153,0.25)' },
] as const;
const chipC: Record<AiCatKey, {bg:string;bd:string;tx:string}> = {
  appearance:{bg:'rgba(192,132,252,0.10)',bd:'rgba(192,132,252,0.25)',tx:'#c084fc'},
  tags:{bg:'rgba(96,165,250,0.10)',bd:'rgba(96,165,250,0.25)',tx:'#60a5fa'},
  environment:{bg:'rgba(52,211,153,0.10)',bd:'rgba(52,211,153,0.25)',tx:'#34d399'},
};
const phdr:React.CSSProperties={display:'flex',alignItems:'center',justifyContent:'space-between',padding:'10px 14px',borderBottom:'1px solid var(--color-border)',flexShrink:0};
const ptitle:React.CSSProperties={fontSize:12,fontWeight:700,color:'var(--color-text-primary)',textTransform:'uppercase',letterSpacing:'0.5px'};


export interface JsonTagTabHandle {
  loadFolder: () => Promise<void>;
  saveAll: () => Promise<void>;
  dirtyCount: number;
  loading: boolean;
  saving: boolean;
}

const JsonTagTab = forwardRef<JsonTagTabHandle>(function JsonTagTab(_props, ref) {
  const { t } = useTranslation();
  const [images,setImages]=useState<JsonImageItem[]>([]);
  const [selectedIdx,setSelectedIdx]=useState(-1);
  const [folderPath,setFolderPath]=useState('');
  const [loading,setLoading]=useState(false);
  const [saving,setSaving]=useState(false);
  const [savingSingle,setSavingSingle]=useState(false);
  const [searchText,setSearchText]=useState('');
  const [filterMode,setFilterMode]=useState<'all'|'tagged'|'untagged'>('all');
  const [imgPage,setImgPage]=useState(0);
  const IMG_PER_PAGE = 30;

  const [simplified,setSimplified]=useState(()=>localStorage.getItem('json_tag_simplified')==='true');
  const [translations,setTranslations]=useState<Record<string,string>>({});
  const [nlTranslation,setNlTranslation]=useState('');
  const [translating,setTranslating]=useState(false);
  const [editingField,setEditingField]=useState<string|null>(null);
  // drag reorder
  const [dragCat,setDragCat]=useState<string|null>(null);
  const [dragIdx,setDragIdx]=useState<number|null>(null);
  const [dragOverIdx,setDragOverIdx]=useState<number|null>(null);
  const [dropSide,setDropSide]=useState<'before'|'after'>('before');
  const chipRefsMap=useRef<Record<string,(HTMLDivElement|null)[]>>({});
  const [col1W,setCol1W]=useState(()=>parseInt(localStorage.getItem('json_col1w')||'220'));
  const [col3W,setCol3W]=useState(()=>parseInt(localStorage.getItem('json_col3w')||'220'));
  const [previewH,setPreviewH]=useState(()=>parseInt(localStorage.getItem('json_previewh')||'220'));

  // Batch operations state (modal-based)
  const [showBatchAddModal,setShowBatchAddModal]=useState(false);
  const [showBatchDeleteModal,setShowBatchDeleteModal]=useState(false);
  const [batchField,setBatchField]=useState('ai_output.tags');
  const [batchTags,setBatchTags]=useState('');
  const [batchPosition,setBatchPosition]=useState<'prepend'|'append'>('append');

  const BATCH_FIELD_OPTIONS = [
    { value: 'fixed.quality', label: 'quality — ' + t('jsonTag.fieldQuality') },
    { value: 'fixed.series', label: 'series — ' + t('jsonTag.fieldSeries') },
    { value: 'fixed.artist', label: 'artist — ' + t('jsonTag.fieldArtist') },
    { value: 'character.name', label: 'character — ' + t('jsonTag.fieldCharacter') },
    { value: 'character.variant', label: 'variant — ' + t('jsonTag.fieldVariant') },
    { value: 'from_path.appearance', label: 'from_path — ' + t('jsonTag.fieldFromPath') },
    { value: 'ai_output.count', label: 'count — ' + t('jsonTag.fieldCount') },
    { value: 'ai_output.appearance', label: 'appearance — ' + t('jsonTag.fieldAppearance') },
    { value: 'ai_output.tags', label: 'tags — ' + t('jsonTag.fieldTags') },
    { value: 'ai_output.environment', label: 'environment — ' + t('jsonTag.fieldEnvironment') },
  ];

  // Col3 sidebar state
  const [col3Mode,setCol3Mode]=useState<'json'|'stats'>('stats');
  const [globalSearch,setGlobalSearch]=useState('');
  const [tagSortBy,setTagSortBy]=useState<'freq'|'name'>('freq');
  const [tagSortDir,setTagSortDir]=useState<'asc'|'desc'>('desc');
  const [tagListMode,setTagListMode]=useState<'all'|'common'>('all');
  const [selectedTags,setSelectedTags]=useState<Set<string>>(new Set());
  const [tagFilterActive,setTagFilterActive]=useState(false);
  const lastClickedTag=useRef<string>('');

  const handleColResize=useCallback((col:'col1'|'col3',e:React.MouseEvent)=>{
    e.preventDefault();const startX=e.clientX;const startW=col==='col1'?col1W:col3W;
    const setW=col==='col1'?setCol1W:setCol3W;const dir=col==='col1'?1:-1;
    const onMove=(ev:MouseEvent)=>{
      const nw=Math.max(160,Math.min(500,startW+dir*(ev.clientX-startX)));setW(nw);
      localStorage.setItem(col==='col1'?'json_col1w':'json_col3w',String(nw));
    };
    const onUp=()=>{document.removeEventListener('mousemove',onMove);document.removeEventListener('mouseup',onUp);document.body.style.cursor='';};
    document.addEventListener('mousemove',onMove);document.addEventListener('mouseup',onUp);document.body.style.cursor='col-resize';
  },[col1W,col3W]);

  const handleRowResize=useCallback((e:React.MouseEvent)=>{
    e.preventDefault();const startY=e.clientY;const startH=previewH;
    const onMove=(ev:MouseEvent)=>{
      const nh=Math.max(100,Math.min(500,startH+(ev.clientY-startY)));setPreviewH(nh);
      localStorage.setItem('json_previewh',String(nh));
    };
    const onUp=()=>{document.removeEventListener('mousemove',onMove);document.removeEventListener('mouseup',onUp);document.body.style.cursor='';};
    document.addEventListener('mousemove',onMove);document.addEventListener('mouseup',onUp);document.body.style.cursor='row-resize';
  },[previewH]);

  // 安全化数据
  const safeData = (d: any): JsonTagData => ({
    fixed: { quality: d?.fixed?.quality, series: d?.fixed?.series, artist: d?.fixed?.artist },
    character: { name: d?.character?.name || '', variant: d?.character?.variant || '' },
    from_path: { appearance: Array.isArray(d?.from_path?.appearance) ? d.from_path.appearance : [] },
    ai_output: { count: d?.ai_output?.count, appearance: Array.isArray(d?.ai_output?.appearance) ? d.ai_output.appearance : [], tags: Array.isArray(d?.ai_output?.tags) ? d.ai_output.tags : [], environment: Array.isArray(d?.ai_output?.environment) ? d.ai_output.environment : [], nl: d?.ai_output?.nl },
  });

  const handleLoadFolder=useCallback(async()=>{
    const sel=await dialogOpen({directory:true,multiple:false,title:t('jsonTag.selectFolder')});
    if(!sel)return; setLoading(true);
    try{const r=await invoke<JsonDataset>('load_json_dataset',{folder:sel as string});
      setImages(r.images.map(img=>({...img,data:safeData(img.data),dirty:false})));setSelectedIdx(r.images.length>0?0:-1);
      setFolderPath(sel as string);setSearchText('');setFilterMode('all');setImgPage(0);
      if(r.detected_format==='simplified'){setSimplified(true);localStorage.setItem('json_tag_simplified','true');}
      else if(r.detected_format==='full'){setSimplified(false);localStorage.setItem('json_tag_simplified','false');}
    }catch(e){console.error(e);}finally{setLoading(false);}
  },[]);
  const handleRefresh=useCallback(async()=>{if(!folderPath)return;setLoading(true);
    try{const r=await invoke<JsonDataset>('load_json_dataset',{folder:folderPath});
      setImages(r.images.map(img=>({...img,data:safeData(img.data),dirty:false})));
    }catch(e){console.error(e);}finally{setLoading(false);}
  },[folderPath]);

  const cur=selectedIdx>=0&&selectedIdx<images.length?images[selectedIdx]:null;
  const imgSrc=cur?convertFileSrc(cur.path):'';
  const dirtyCount=images.filter(i=>i.dirty).length;

  const handleSaveSingle=useCallback(async()=>{if(!cur||!cur.dirty)return;setSavingSingle(true);
    try{await invoke('save_single_json_file',{imagePath:cur.path,data:cur.data,simplified});
      setImages(p=>p.map((img,i)=>i===selectedIdx?{...img,dirty:false}:img));
    }catch(e){console.error(e);}finally{setSavingSingle(false);}
  },[cur,selectedIdx,simplified]);
  const handleSaveAll=useCallback(async()=>{const dirty=images.filter(i=>i.dirty).map(i=>({path:i.path,data:i.data}));
    if(!dirty.length)return;setSaving(true);
    try{await invoke<number>('save_all_json_files',{items:dirty,simplified});
      setImages(p=>p.map(img=>img.dirty?{...img,dirty:false}:img));
    }catch(e){console.error(e);}finally{setSaving(false);}
  },[images,simplified]);

  useImperativeHandle(ref, () => ({
    loadFolder: handleLoadFolder,
    saveAll: handleSaveAll,
    dirtyCount,
    loading,
    saving,
  }), [handleLoadFolder, handleSaveAll, dirtyCount, loading, saving]);
  const taggedN=images.filter(i=>i.has_json).length;
  const goPrev=()=>{if(selectedIdx>0)setSelectedIdx(selectedIdx-1);};
  const goNext=()=>{if(selectedIdx<images.length-1)setSelectedIdx(selectedIdx+1);};

  const filtered=useMemo(()=>{
    let list=images.map((img,i)=>({...img,_i:i}));
    if(searchText){
      const q=searchText.toLowerCase();
      list=list.filter(img=>{
        if(img.filename.toLowerCase().includes(q)) return true;
        const d=img.data;
        const all=[...(d.fixed.quality?.split(',').map(s=>s.trim())||[]),...(d.fixed.series?.split(',').map(s=>s.trim())||[]),...(d.fixed.artist?.split(',').map(s=>s.trim())||[]),d.character.name,d.character.variant,d.ai_output.count||'',...d.ai_output.appearance,...d.ai_output.tags,...d.ai_output.environment,...d.from_path.appearance,d.ai_output.nl||''];
        return all.some(t=>t.toLowerCase().includes(q));
      });
    }
    if(filterMode==='tagged')list=list.filter(img=>img.has_json);
    if(filterMode==='untagged')list=list.filter(img=>!img.has_json);
    if(tagFilterActive&&selectedTags.size>0){
      list=list.filter(img=>{
        const d=img.data;
        const allTags=[...(d.fixed.quality?.split(',').map(s=>s.trim())||[]),...(d.fixed.series?.split(',').map(s=>s.trim())||[]),...(d.fixed.artist?.split(',').map(s=>s.trim())||[]),d.character.name,d.character.variant,...d.ai_output.appearance,...d.ai_output.tags,...d.ai_output.environment,...d.from_path.appearance].filter(Boolean);
        return [...selectedTags].every(t=>allTags.includes(t));
      });
    }
    return list;
  },[images,searchText,filterMode,tagFilterActive,selectedTags]);

  // Tag statistics for Col3 sidebar
  const tagStats=useMemo(()=>{
    const m:Record<string,number>={};
    images.forEach(img=>{
      const d=img.data;
      const collect=(arr:string[])=>arr.forEach(t=>{if(t)m[t]=(m[t]||0)+1;});
      const collectStr=(s:string|undefined)=>{if(s)s.split(',').map(x=>x.trim()).filter(Boolean).forEach(t=>{m[t]=(m[t]||0)+1;});};
      collectStr(d.fixed.quality);collectStr(d.fixed.series);collectStr(d.fixed.artist);
      collectStr(d.character.name);collectStr(d.character.variant);collectStr(d.ai_output.count);
      collect(d.ai_output.appearance);collect(d.ai_output.tags);collect(d.ai_output.environment);
      collect(d.from_path.appearance);
    });
    return Object.entries(m).sort((a,b)=>b[1]-a[1]);
  },[images]);

  const taggedCount=useMemo(()=>images.filter(i=>i.has_json).length,[images]);
  const filteredStats=useMemo(()=>{
    const base=tagListMode==='common'?tagStats.filter(([,c])=>taggedCount>0&&c===taggedCount):tagStats;
    let sorted=[...base];
    if(tagSortBy==='freq')sorted.sort((a,b)=>tagSortDir==='desc'?b[1]-a[1]:a[1]-b[1]);
    else sorted.sort((a,b)=>tagSortDir==='desc'?b[0].localeCompare(a[0]):a[0].localeCompare(b[0]));
    if(!globalSearch)return sorted;
    const q=globalSearch.toLowerCase();
    return sorted.filter(([t])=>t.includes(q)||(translations[t]||'').includes(q));
  },[tagStats,globalSearch,tagListMode,taggedCount,translations,tagSortBy,tagSortDir]);

  // Tag selection
  const toggleTagSelect=(tag:string,e:React.MouseEvent)=>{
    const isCtrl=e.metaKey||e.ctrlKey;const isShift=e.shiftKey;
    setSelectedTags(prev=>{
      if(isShift&&lastClickedTag.current&&filteredStats.length>0){
        const tags=filteredStats.map(([t])=>t);const lastIdx=tags.indexOf(lastClickedTag.current);const curIdx=tags.indexOf(tag);
        if(lastIdx>=0&&curIdx>=0){const from=Math.min(lastIdx,curIdx);const to=Math.max(lastIdx,curIdx);const next=new Set(isCtrl?prev:[]);for(let i=from;i<=to;i++)next.add(tags[i]);return next;}
      }
      if(isCtrl){const next=new Set(prev);if(next.has(tag))next.delete(tag);else next.add(tag);lastClickedTag.current=tag;return next;}
      lastClickedTag.current=tag;
      if(prev.has(tag)&&prev.size===1)return new Set();
      return new Set([tag]);
    });
    if(!e.shiftKey)lastClickedTag.current=tag;
  };


  // Delete selected tags from all images
  const handleSidebarBatchDelete=useCallback(()=>{
    if(selectedTags.size===0)return;
    setImages(prev=>prev.map(img=>{
      const d=JSON.parse(JSON.stringify(img.data)) as JsonTagData;
      let changed=false;
      const filterArr=(arr:string[])=>{const n=arr.filter(t=>!selectedTags.has(t));if(n.length!==arr.length)changed=true;return n;};
      const filterStr=(val:string|undefined)=>{if(!val)return val;const parts=val.split(',').map(s=>s.trim()).filter(Boolean);const n=parts.filter(t=>!selectedTags.has(t));if(n.length!==parts.length){changed=true;return n.length?n.join(', '):undefined;}return val;};
      d.ai_output.appearance=filterArr(d.ai_output.appearance);d.ai_output.tags=filterArr(d.ai_output.tags);d.ai_output.environment=filterArr(d.ai_output.environment);d.from_path.appearance=filterArr(d.from_path.appearance);
      d.fixed.quality=filterStr(d.fixed.quality);d.fixed.series=filterStr(d.fixed.series);d.fixed.artist=filterStr(d.fixed.artist);d.character.name=filterStr(d.character.name)||'';d.character.variant=filterStr(d.character.variant)||'';d.ai_output.count=filterStr(d.ai_output.count);
      return changed?{...img,data:d,dirty:true}:img;
    }));
    setSelectedTags(new Set());
  },[selectedTags]);

  // Tag chip color helper
  const statChipColors=[{bg:'rgba(124,92,252,0.10)',bd:'rgba(124,92,252,0.25)',tx:'#a78bfa'},{bg:'rgba(96,165,250,0.10)',bd:'rgba(96,165,250,0.25)',tx:'#60a5fa'},{bg:'rgba(74,222,128,0.10)',bd:'rgba(74,222,128,0.25)',tx:'#4ade80'},{bg:'rgba(251,191,36,0.10)',bd:'rgba(251,191,36,0.25)',tx:'#fbbf24'},{bg:'rgba(248,113,113,0.10)',bd:'rgba(248,113,113,0.25)',tx:'#f87171'},{bg:'rgba(192,132,252,0.10)',bd:'rgba(192,132,252,0.25)',tx:'#c084fc'},{bg:'rgba(45,212,191,0.10)',bd:'rgba(45,212,191,0.25)',tx:'#2dd4bf'},{bg:'rgba(251,146,60,0.10)',bd:'rgba(251,146,60,0.25)',tx:'#fb923c'},{bg:'rgba(236,72,153,0.10)',bd:'rgba(236,72,153,0.25)',tx:'#ec4899'},{bg:'rgba(132,204,22,0.10)',bd:'rgba(132,204,22,0.25)',tx:'#84cc16'}];
  const getStatColor=(tag:string)=>{let h=0;for(let i=0;i<tag.length;i++)h=((h<<5)-h+tag.charCodeAt(i))|0;return statChipColors[Math.abs(h)%statChipColors.length];};

  // Batch add tags to all images
  const handleBatchAdd=useCallback(()=>{
    const tags=batchTags.split(',').map(s=>s.trim()).filter(Boolean);
    if(!tags.length)return;
    const isArray=['ai_output.appearance','ai_output.tags','ai_output.environment','from_path.appearance'].includes(batchField);
    setImages(prev=>prev.map(img=>{
      const d=JSON.parse(JSON.stringify(img.data)) as JsonTagData;
      if(isArray){
        const [section,field]=batchField.split('.') as [keyof JsonTagData, string];
        const arr=(d as any)[section][field] as string[];
        const newTags=tags.filter(t=>!arr.includes(t));
        if(newTags.length===0)return img;
        if(batchPosition==='prepend')(d as any)[section][field]=[...newTags,...arr];
        else (d as any)[section][field]=[...arr,...newTags];
      }else{
        const [section,field]=batchField.split('.') as [keyof JsonTagData, string];
        const cur=((d as any)[section][field]||'') as string;
        const parts=cur?cur.split(',').map(s=>s.trim()).filter(Boolean):[];
        const newTags=tags.filter(t=>!parts.includes(t));
        if(newTags.length===0)return img;
        if(batchPosition==='prepend')(d as any)[section][field]=[...newTags,...parts].join(', ');
        else (d as any)[section][field]=[...parts,...newTags].join(', ');
      }
      return {...img,data:d,dirty:true};
    }));
    setBatchTags('');setShowBatchAddModal(false);
  },[batchField,batchTags,batchPosition]);

  // Batch delete tags from all images
  const handleBatchDelete=useCallback(()=>{
    const tags=batchTags.split(',').map(s=>s.trim()).filter(Boolean);
    if(!tags.length)return;
    const tagsSet=new Set(tags);
    const isAll=batchField==='all';
    setImages(prev=>prev.map(img=>{
      const d=JSON.parse(JSON.stringify(img.data)) as JsonTagData;
      let changed=false;
      const filterArr=(arr:string[])=>{const n=arr.filter(t=>!tagsSet.has(t));if(n.length!==arr.length){changed=true;}return n;};
      const filterStr=(val:string|undefined)=>{if(!val)return val;const parts=val.split(',').map(s=>s.trim()).filter(Boolean);const n=parts.filter(t=>!tagsSet.has(t));if(n.length!==parts.length){changed=true;return n.length?n.join(', '):undefined;}return val;};
      if(isAll||batchField==='ai_output.appearance')d.ai_output.appearance=filterArr(d.ai_output.appearance);
      if(isAll||batchField==='ai_output.tags')d.ai_output.tags=filterArr(d.ai_output.tags);
      if(isAll||batchField==='ai_output.environment')d.ai_output.environment=filterArr(d.ai_output.environment);
      if(isAll||batchField==='from_path.appearance')d.from_path.appearance=filterArr(d.from_path.appearance);
      if(isAll||batchField==='fixed.quality')d.fixed.quality=filterStr(d.fixed.quality);
      if(isAll||batchField==='fixed.series')d.fixed.series=filterStr(d.fixed.series);
      if(isAll||batchField==='fixed.artist')d.fixed.artist=filterStr(d.fixed.artist);
      if(isAll||batchField==='character.name')d.character.name=filterStr(d.character.name)||'';
      if(isAll||batchField==='character.variant')d.character.variant=filterStr(d.character.variant)||'';
      if(isAll||batchField==='ai_output.count')d.ai_output.count=filterStr(d.ai_output.count);
      return changed?{...img,data:d,dirty:true}:img;
    }));
    setBatchTags('');setShowBatchDeleteModal(false);
  },[batchField,batchTags]);

  const totalPages = Math.max(1, Math.ceil(filtered.length / IMG_PER_PAGE));
  const pagedFiltered = filtered.slice(imgPage * IMG_PER_PAGE, (imgPage + 1) * IMG_PER_PAGE);
  const prevFilteredLen = useRef(filtered.length);
  if (filtered.length !== prevFilteredLen.current) { prevFilteredLen.current = filtered.length; if (imgPage >= Math.ceil(filtered.length / IMG_PER_PAGE)) { setImgPage(0); } }

  const updateData=useCallback((fn:(d:JsonTagData)=>JsonTagData)=>{
    setImages(p=>p.map((img,i)=>i===selectedIdx?{...img,data:fn(JSON.parse(JSON.stringify(img.data))),dirty:true}:img));
  },[selectedIdx]);

  const removeAiTag=useCallback((cat:AiCatKey,tag:string)=>{
    updateData(d=>{d.ai_output[cat]=d.ai_output[cat].filter(t=>t!==tag);return d;});
  },[updateData]);
  const removeFromPathTag=useCallback((tag:string)=>{
    updateData(d=>{d.from_path.appearance=d.from_path.appearance.filter(t=>t!==tag);return d;});
  },[updateData]);



  // 翻译当前图片所有标签+NL
  const handleTranslate=useCallback(async()=>{
    if(!cur)return;
    setTranslating(true);setNlTranslation('');
    const allTags=[...cur.data.from_path.appearance,...cur.data.ai_output.appearance,...cur.data.ai_output.tags,...cur.data.ai_output.environment];
    const pushSplit=(v:string|undefined)=>{if(v)v.split(',').map(s=>s.trim()).filter(Boolean).forEach(t=>allTags.push(t));};
    pushSplit(cur.data.fixed.quality);
    pushSplit(cur.data.fixed.series);
    pushSplit(cur.data.fixed.artist);
    pushSplit(cur.data.character.name);
    pushSplit(cur.data.character.variant);
    pushSplit(cur.data.ai_output.count);
    const provider=localStorage.getItem('translate_provider')||'google';
    try{
      if(allTags.length>0){
        const result=await invoke<{translations:{source:string;translated:string}[];cached_count:number;translated_count:number}>('translate_tags',{
          tags:allTags,targetLang:localStorage.getItem('translate_target_lang')||'zh-CN',provider,
          baiduAppid:localStorage.getItem('baidu_appid')||'',baiduKey:localStorage.getItem('baidu_key')||'',
          youdaoAppKey:localStorage.getItem('youdao_app_key')||'',youdaoAppSecret:localStorage.getItem('youdao_app_secret')||'',
          bingKey:localStorage.getItem('bing_key')||'',bingRegion:localStorage.getItem('bing_region')||'',
        });
        setTranslations(prev=>{const next={...prev};result.translations.forEach(item=>{if(item.translated)next[item.source]=item.translated;});return next;});
      }
      // NL翻译（不缓存）
      if(cur.data.ai_output.nl){
        const nlResult=await invoke<{translations:{source:string;translated:string}[]}>('translate_tags',{
          tags:[cur.data.ai_output.nl],targetLang:localStorage.getItem('translate_target_lang')||'zh-CN',provider,
          baiduAppid:localStorage.getItem('baidu_appid')||'',baiduKey:localStorage.getItem('baidu_key')||'',
          youdaoAppKey:localStorage.getItem('youdao_app_key')||'',youdaoAppSecret:localStorage.getItem('youdao_app_secret')||'',
          bingKey:localStorage.getItem('bing_key')||'',bingRegion:localStorage.getItem('bing_region')||'',
        });
        if(nlResult.translations[0]?.translated)setNlTranslation(nlResult.translations[0].translated);
      }
    }catch(e){console.error('translate failed:',e);}finally{setTranslating(false);}
  },[cur]);


  // drag reorder handlers
  const dragState=useRef<{active:boolean;fromIdx:number;cat:string;startX:number;startY:number}>({active:false,fromIdx:-1,cat:'',startX:0,startY:0});

  const moveTagInArr=(cat:string,fromIdx:number,toIdx:number)=>{
    if(fromIdx===toIdx)return;
    updateData(d=>{
      // 逗号分隔字符串字段
      const commaFields: Record<string, {get:()=>string|undefined, set:(v:string|undefined)=>void}> = {
        'f.quality': {get:()=>d.fixed.quality, set:v=>{d.fixed.quality=v;}},
        'f.series': {get:()=>d.fixed.series, set:v=>{d.fixed.series=v;}},
        'f.artist': {get:()=>d.fixed.artist, set:v=>{d.fixed.artist=v;}},
        'c.name': {get:()=>d.character.name||undefined, set:v=>{d.character.name=v||'';}},
        'c.variant': {get:()=>d.character.variant||undefined, set:v=>{d.character.variant=v||'';}},
        'ai.count': {get:()=>d.ai_output.count, set:v=>{d.ai_output.count=v;}},
      };
      if(commaFields[cat]){
        const f=commaFields[cat];
        const val=f.get();
        if(!val)return d;
        const parts=val.split(',').map(s=>s.trim()).filter(Boolean);
        if(fromIdx>=parts.length||toIdx>=parts.length)return d;
        const [moved]=parts.splice(fromIdx,1);
        parts.splice(toIdx,0,moved);
        f.set(parts.join(', '));
        return d;
      }
      // 数组字段
      let arr:string[];
      if(cat==='fp')arr=d.from_path.appearance;
      else arr=(d.ai_output as any)[cat];
      if(!arr)return d;
      const [moved]=arr.splice(fromIdx,1);
      arr.splice(toIdx,0,moved);
      return d;
    });
  };

  const handleChipPointerDown=(e:React.PointerEvent,idx:number,cat:string)=>{
    if((e.target as HTMLElement).closest('button'))return;
    e.preventDefault();
    dragState.current={active:false,fromIdx:idx,cat,startX:e.clientX,startY:e.clientY};
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };

  const handleChipPointerMove=(e:React.PointerEvent,cat:string)=>{
    const ds=dragState.current;
    if(ds.fromIdx<0||ds.cat!==cat)return;
    const dx=e.clientX-ds.startX,dy=e.clientY-ds.startY;
    if(!ds.active&&Math.abs(dx)+Math.abs(dy)>5){ds.active=true;setDragIdx(ds.fromIdx);setDragCat(cat);}
    if(!ds.active)return;
    const els=chipRefsMap.current[cat]||[];
    for(let i=0;i<els.length;i++){
      const el=els[i];
      if(!el||i===ds.fromIdx)continue;
      const r=el.getBoundingClientRect();
      if(e.clientX>=r.left&&e.clientX<=r.right&&e.clientY>=r.top&&e.clientY<=r.bottom){
        const mid=r.left+r.width/2;
        const side=e.clientX<mid?'before':'after';
        if(dragOverIdx!==i||dropSide!==side){setDragOverIdx(i);setDropSide(side);}
        return;
      }
    }
  };

  const handleChipPointerUp=()=>{
    const ds=dragState.current;
    if(ds.active&&dragOverIdx!==null&&dragOverIdx!==ds.fromIdx){
      let toIdx=dragOverIdx;
      if(dropSide==='after')toIdx+=1;
      if(ds.fromIdx<toIdx)toIdx-=1;
      moveTagInArr(ds.cat,ds.fromIdx,toIdx);
    }
    dragState.current={active:false,fromIdx:-1,cat:'',startX:0,startY:0};
    setDragIdx(null);setDragOverIdx(null);setDragCat(null);
  };

  const tagChips=(arr:string[],cc:{bg:string;bd:string;tx:string},onRemove:(t:string)=>void,cat:string,editKey?:string,onAdd?:(v:string)=>void)=>{
    if(!chipRefsMap.current[cat])chipRefsMap.current[cat]=[];
    return(
    <div style={{display:'flex',flexWrap:'wrap',gap:4,minHeight:24,alignItems:'center',touchAction:'none'}}
      onPointerMove={e=>handleChipPointerMove(e,cat)} onPointerUp={handleChipPointerUp}>
      {arr.map((tag,ti)=>{
        const tr=translations[tag];
        const isDragging=dragCat===cat&&dragIdx===ti;
        const isOverBefore=dragCat===cat&&dragOverIdx===ti&&dropSide==='before';
        const isOverAfter=dragCat===cat&&dragOverIdx===ti&&dropSide==='after';
        return(
        <div key={ti} ref={el=>{chipRefsMap.current[cat][ti]=el;}} style={{position:'relative',display:'inline-flex'}}
          onPointerDown={e=>handleChipPointerDown(e,ti,cat)}>
          {isOverBefore&&<div style={{position:'absolute',left:-3,top:2,bottom:2,width:2,borderRadius:1,background:'#7c5cfc',zIndex:1}} />}
          <div style={{display:'inline-flex',alignItems:'center',gap:3,padding:'2px 7px',borderRadius:12,background:cc.bg,border:`1px solid ${cc.bd}`,fontSize:11,color:cc.tx,lineHeight:1.3,cursor:'grab',transition:'opacity 0.12s',opacity:isDragging?0.35:1,userSelect:'none'}}>
            <span>{tag}{tr&&<span style={{color:'var(--color-text-tertiary)',fontSize:10,marginLeft:3}}>({tr})</span>}</span>
            <button onClick={()=>onRemove(tag)} style={{display:'flex',alignItems:'center',justifyContent:'center',width:13,height:13,borderRadius:'50%',background:'transparent',color:cc.tx,opacity:0.4,transition:'all 0.12s',flexShrink:0}}
              onMouseEnter={e=>{e.currentTarget.style.opacity='1';e.currentTarget.style.background='rgba(248,113,113,0.15)';e.currentTarget.style.color='#f87171';}}
              onMouseLeave={e=>{e.currentTarget.style.opacity='0.4';e.currentTarget.style.background='transparent';e.currentTarget.style.color=cc.tx;}}
            ><X style={{width:8,height:8}} /></button>
          </div>
          {isOverAfter&&<div style={{position:'absolute',right:-3,top:2,bottom:2,width:2,borderRadius:1,background:'#7c5cfc',zIndex:1}} />}
        </div>
        );})}
      {arr.length===0&&!editKey&&<span style={{fontSize:10,color:'var(--color-text-tertiary)',fontStyle:'italic',lineHeight:'24px'}}>{t('jsonTag.noTagData')}</span>}
      {editKey&&onAdd&&(<>
        {arr.length===0&&editingField!==editKey&&<span onClick={()=>setEditingField(editKey)} style={{fontSize:10,color:'var(--color-text-tertiary)',fontStyle:'italic',lineHeight:'24px',cursor:'pointer'}}>{t('jsonTag.noTagData')}</span>}
        {arr.length>0&&editingField!==editKey&&<button onClick={()=>setEditingField(editKey)} style={{display:'flex',alignItems:'center',justifyContent:'center',width:18,height:18,borderRadius:'50%',background:cc.bg,border:`1px solid ${cc.bd}`,color:cc.tx,cursor:'pointer',flexShrink:0,opacity:0.5,transition:'opacity 0.15s'}}
          onMouseEnter={e=>e.currentTarget.style.opacity='1'} onMouseLeave={e=>e.currentTarget.style.opacity='0.5'}
        ><Plus style={{width:10,height:10}} /></button>}
        {editingField===editKey&&<TagAutocomplete
          autoFocus
          placeholder={t('jsonTag.inputTag')}
          clearOnSelect={true}
          keepOpen={true}
          onSelect={(v) => { if(v.trim())onAdd(v.trim()); }}
          onBlur={() => setEditingField(null)}
          onKeyDown={(e) => { if (e.key === 'Escape') setEditingField(null); }}
          inputStyle={{fontSize:11,height:24,border:'none',background:'transparent',padding:'0 4px',flex:'1 0 60px',minWidth:60,outline:'none',maxWidth:200}}
        />}
      </>)}
    </div>
    );
  };

  return (
    <div style={{flex:1,display:'flex',overflow:'hidden',minHeight:0}}>
      {/* Col1: Images */}
      <div style={{width:col1W,minWidth:160,maxWidth:500,flexShrink:0,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden'}}>
        <div style={{padding:8,borderBottom:'1px solid var(--color-border)'}}>
          <div style={{display:'flex',gap:4,marginBottom:6}}>
            <div style={{position:'relative',flex:1}}>
              <Search style={{position:'absolute',left:8,top:'50%',transform:'translateY(-50%)',width:13,height:13,color:'var(--color-text-tertiary)'}} />
              <input className="form-input" placeholder={t('jsonTag.search')} value={searchText} onChange={e=>setSearchText(e.target.value)} style={{paddingLeft:28,fontSize:11,height:30}} />
            </div>
            <button className="btn btn-ghost btn-sm" onClick={handleRefresh} disabled={!folderPath||loading} title={t('jsonTag.refresh')} style={{width:30,height:30,padding:0,display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}><RefreshCw style={{width:13,height:13,animation:loading?'spin 1s linear infinite':undefined}} /></button>
          </div>
          <div style={{display:'flex',gap:4}}>
            {[{k:'all' as const,l:t('jsonTag.filterAll'),n:images.length},{k:'untagged' as const,l:t('jsonTag.filterUntagged'),n:images.length-taggedN},{k:'tagged' as const,l:t('jsonTag.filterTagged'),n:taggedN}].map(f=>(
              <button key={f.k} onClick={()=>setFilterMode(f.k)} style={{flex:1,padding:'3px 0',borderRadius:6,fontSize:10,fontWeight:500,background:filterMode===f.k?'rgba(124,92,252,0.15)':'transparent',color:filterMode===f.k?'#a78bfa':'var(--color-text-tertiary)',border:filterMode===f.k?'1px solid rgba(124,92,252,0.25)':'1px solid transparent'}}>{f.l} {f.n}</button>
            ))}
          </div>
        </div>
        <div style={{flex:1,overflowY:'auto',padding:6}}>
          {images.length===0?(<div style={{display:'flex',flexDirection:'column',alignItems:'center',justifyContent:'center',height:'100%',gap:8,color:'var(--color-text-tertiary)'}}><FolderOpen style={{width:32,height:32,opacity:0.2}} /><span style={{fontSize:11,opacity:0.6}}>{t('jsonTag.loadHint')}</span></div>):(
            <div style={{display:'grid',gridTemplateColumns:'repeat(3,1fr)',gap:4}}>
              {pagedFiltered.map(img=>{const sel=img._i===selectedIdx;const src=convertFileSrc(img.path);const total=img.data.ai_output.appearance.length+img.data.ai_output.tags.length+img.data.ai_output.environment.length+img.data.from_path.appearance.length;return(
                <div key={img._i} onClick={()=>setSelectedIdx(img._i)} style={{position:'relative',aspectRatio:'1',borderRadius:8,overflow:'hidden',cursor:'pointer',border:`2px solid ${sel?'#7c5cfc':'transparent'}`,boxShadow:sel?'0 0 0 1px rgba(124,92,252,0.3)':'none',transition:'all 0.15s',background:'var(--color-bg-input)'}}>
                  <img src={src} alt={img.filename} style={{width:'100%',height:'100%',objectFit:'cover'}} loading="lazy" />
                  {img.has_json&&<div style={{position:'absolute',bottom:2,right:2,minWidth:14,height:14,borderRadius:7,padding:'0 3px',background:img.dirty?'rgba(239,68,68,0.9)':'rgba(124,92,252,0.85)',fontSize:8,color:'#fff',fontWeight:700,display:'flex',alignItems:'center',justifyContent:'center'}}>{total}</div>}
                </div>
              );})}
            </div>
          )}
        </div>
          {images.length>0&&<div style={{padding:'4px 10px',borderTop:'1px solid var(--color-border)',fontSize:10,color:'var(--color-text-tertiary)',display:'flex',alignItems:'center',justifyContent:'space-between'}}>
            <span>{filtered.length===images.length?t('jsonTag.nImages',{n:images.length}):t('jsonTag.nOfTotal',{n:filtered.length,total:images.length})}</span>
            {totalPages>1&&<div style={{display:'flex',alignItems:'center',gap:4}}>
              <button onClick={()=>setImgPage(p=>Math.max(0,p-1))} disabled={imgPage<=0} style={{width:20,height:20,borderRadius:4,border:'1px solid var(--color-border)',background:imgPage<=0?'transparent':'rgba(124,92,252,0.08)',color:imgPage<=0?'var(--color-text-tertiary)':'#a78bfa',cursor:imgPage<=0?'default':'pointer',display:'flex',alignItems:'center',justifyContent:'center',padding:0}}><ChevronLeft style={{width:11,height:11}} /></button>
              <span style={{fontSize:10,minWidth:40,textAlign:'center'}}>{imgPage+1}/{totalPages}</span>
              <button onClick={()=>setImgPage(p=>Math.min(totalPages-1,p+1))} disabled={imgPage>=totalPages-1} style={{width:20,height:20,borderRadius:4,border:'1px solid var(--color-border)',background:imgPage>=totalPages-1?'transparent':'rgba(124,92,252,0.08)',color:imgPage>=totalPages-1?'var(--color-text-tertiary)':'#a78bfa',cursor:imgPage>=totalPages-1?'default':'pointer',display:'flex',alignItems:'center',justifyContent:'center',padding:0}}><ChevronRight style={{width:11,height:11}} /></button>
            </div>}
          </div>}
      </div>

      {/* resize handle 1 */}
      <div onMouseDown={e=>handleColResize('col1',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}} title={t('jsonTag.dragWidth')}>
        <div style={{width:2,height:32,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
      </div>

      {/* Col2: Preview + Editor */}
      <div style={{flex:1,display:'flex',flexDirection:'column',minWidth:0,overflow:'hidden'}}>
        {/* Preview */}
        <div style={{height:previewH,flexShrink:0,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden'}}>
          <div style={phdr}>
            <div style={{display:'flex',alignItems:'center',gap:8}}>
              <ImageIcon style={{width:14,height:14,color:'#7c5cfc'}} />
              <span style={ptitle}>{t('jsonTag.preview')}</span>
              {cur&&<span style={{fontSize:11,color:'var(--color-text-tertiary)',fontWeight:400}}>{cur.filename}</span>}
            </div>
            <div style={{display:'flex',alignItems:'center',gap:6}}>
              <button className="btn btn-ghost btn-sm" onClick={goPrev} disabled={selectedIdx<=0} style={{width:26,height:26,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6}}><ChevronLeft style={{width:14,height:14}} /></button>
              <span style={{fontSize:11,color:'var(--color-text-tertiary)',minWidth:50,textAlign:'center'}}>{selectedIdx+1}/{images.length}</span>
              <button className="btn btn-ghost btn-sm" onClick={goNext} disabled={selectedIdx>=images.length-1} style={{width:26,height:26,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6}}><ChevronRight style={{width:14,height:14}} /></button>
            </div>
          </div>
          <div style={{flex:1,display:'flex',alignItems:'center',justifyContent:'center',background:'rgba(0,0,0,0.15)',minHeight:0,overflow:'hidden'}}>
            {cur?<img src={imgSrc} alt={cur.filename} draggable={false} style={{maxWidth:'100%',maxHeight:'100%',objectFit:'contain',pointerEvents:'none'}} />
              :<div style={{display:'flex',flexDirection:'column',alignItems:'center',gap:8,color:'var(--color-text-tertiary)'}}><ImageIcon style={{width:48,height:48,opacity:0.2}} /><span style={{fontSize:12,opacity:0.6}}>{images.length===0?t('jsonTag.loadToShow'):t('jsonTag.selectToPreview')}</span></div>}
          </div>
        </div>

        {/* row resize handle */}
        <div onMouseDown={handleRowResize} style={{height:6,cursor:'row-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}} title={t('jsonTag.dragHeight')}>
          <div style={{width:32,height:2,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
        </div>

        {/* Editor */}
        <div style={{flex:1,display:'flex',flexDirection:'column',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden',minHeight:0}}>
          <div style={phdr}>
            <div style={{display:'flex',alignItems:'center',gap:8}}>
              <Sparkles style={{width:14,height:14,color:'#22d3ee'}} />
              <span style={ptitle}>{t('jsonTag.jsonEditor')}</span>
              {images.length>0&&<span style={{fontSize:9,padding:'2px 8px',borderRadius:10,background:simplified?'rgba(34,197,94,0.15)':'rgba(99,102,241,0.15)',color:simplified?'#22c55e':'#818cf8',fontWeight:700,border:`1px solid ${simplified?'rgba(34,197,94,0.25)':'rgba(99,102,241,0.25)'}`}}>{simplified?t('jsonTag.simplified'):t('jsonTag.full')}</span>}
            </div>
            <div style={{display:'flex',alignItems:'center',gap:6}}>
              <button className="btn btn-ghost btn-sm" onClick={handleTranslate} disabled={!cur||translating} title={t('jsonTag.translateTags')} style={{gap:4,fontSize:10,height:24,padding:'0 8px',color:Object.keys(translations).length>0?'#60a5fa':undefined}}>
                {translating?<Loader2 style={{width:10,height:10,animation:'spin 1s linear infinite'}} />:<Languages style={{width:10,height:10}} />}
              </button>
              <button className="btn btn-primary" style={{fontSize:10,gap:4,height:24,padding:'0 10px'}} disabled={!cur||!cur.dirty||savingSingle} onClick={handleSaveSingle}>
                {savingSingle?<Loader2 style={{width:10,height:10,animation:'spin 1s linear infinite'}} />:<Save style={{width:10,height:10}} />} {t('jsonTag.save')}
              </button>
            </div>
          </div>

          <div style={{flex:1,overflowY:'auto',padding:'12px 14px',display:'flex',flexDirection:'column',gap:14}}>
            {!cur?<span style={{fontSize:11,color:'var(--color-text-tertiary)',fontStyle:'italic'}}>{t('jsonTag.selectToEdit')}</span>:(<>
              {/* 单值/多值chip辅助 — 逗号分隔自动拆分为多个chip */}
              {(()=>{
                const fieldChips=(fieldKey:string,val:string|undefined,onSet:(v:string)=>void,onClear:()=>void,color:string,ph:string)=>{
                  const rgbaMatch=color.match(/#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i);
                  const r=rgbaMatch?parseInt(rgbaMatch[1],16):0,g=rgbaMatch?parseInt(rgbaMatch[2],16):0,b=rgbaMatch?parseInt(rgbaMatch[3],16):0;
                  const bgLight=`rgba(${r},${g},${b},0.06)`,bdLight=`rgba(${r},${g},${b},0.15)`,bgChip=`rgba(${r},${g},${b},0.10)`,bdChip=`rgba(${r},${g},${b},0.25)`;
                  // 拆分逗号分隔的多个值
                  const parts = val ? val.split(',').map(s=>s.trim()).filter(Boolean) : [];
                  const removeOne = (idx:number) => {
                    const newParts = parts.filter((_,i)=>i!==idx);
                    if(newParts.length===0) onClear();
                    else onSet(newParts.join(', '));
                  };
                  const addOne = (v:string) => {
                    if(parts.length===0) onSet(v);
                    else onSet([...parts, v].join(', '));
                  };
                  if(!chipRefsMap.current[fieldKey])chipRefsMap.current[fieldKey]=[];
                  return (
                    <div onClick={()=>{if(parts.length===0&&editingField!==fieldKey)setEditingField(fieldKey);}}
                      onPointerMove={e=>handleChipPointerMove(e,fieldKey)} onPointerUp={handleChipPointerUp}
                      style={{padding:'5px 8px',borderRadius:'var(--radius-md)',background:bgLight,border:`1px solid ${bdLight}`,minHeight:24,display:'flex',flexWrap:'wrap',gap:4,alignItems:'center',cursor:parts.length>0?'default':'pointer',touchAction:'none'}}>
                      {parts.map((p,pi)=>{const tr=translations[p];
                        const isDragging=dragCat===fieldKey&&dragIdx===pi;
                        const isOverBefore=dragCat===fieldKey&&dragOverIdx===pi&&dropSide==='before';
                        const isOverAfter=dragCat===fieldKey&&dragOverIdx===pi&&dropSide==='after';
                        return(
                        <div key={pi} ref={el=>{chipRefsMap.current[fieldKey][pi]=el;}} style={{position:'relative',display:'inline-flex'}}
                          onPointerDown={e=>handleChipPointerDown(e,pi,fieldKey)}>
                          {isOverBefore&&<div style={{position:'absolute',left:-3,top:2,bottom:2,width:2,borderRadius:1,background:'#7c5cfc',zIndex:1}} />}
                          <div style={{display:'inline-flex',alignItems:'center',gap:3,padding:'2px 7px',borderRadius:12,background:bgChip,border:`1px solid ${bdChip}`,fontSize:11,color,lineHeight:1.3,cursor:'grab',transition:'opacity 0.12s',opacity:isDragging?0.35:1,userSelect:'none'}}>
                            <span>{p}{tr&&<span style={{color:'var(--color-text-tertiary)',fontSize:10,marginLeft:3}}>({tr})</span>}</span>
                            <button onClick={e=>{e.stopPropagation();removeOne(pi);}} style={{display:'flex',alignItems:'center',justifyContent:'center',width:13,height:13,borderRadius:'50%',background:'transparent',color,opacity:0.4,flexShrink:0}}
                              onMouseEnter={e=>{e.currentTarget.style.opacity='1';e.currentTarget.style.background='rgba(248,113,113,0.15)';e.currentTarget.style.color='#f87171';}}
                              onMouseLeave={e=>{e.currentTarget.style.opacity='0.4';e.currentTarget.style.background='transparent';e.currentTarget.style.color=color;}}
                            ><X style={{width:8,height:8}} /></button>
                          </div>
                          {isOverAfter&&<div style={{position:'absolute',right:-3,top:2,bottom:2,width:2,borderRadius:1,background:'#7c5cfc',zIndex:1}} />}
                        </div>);
                      })}
                      {parts.length>0&&editingField!==fieldKey&&<button onClick={e=>{e.stopPropagation();setEditingField(fieldKey);}} style={{display:'flex',alignItems:'center',justifyContent:'center',width:18,height:18,borderRadius:'50%',background:bgChip,border:`1px solid ${bdChip}`,color,cursor:'pointer',flexShrink:0,opacity:0.5,transition:'opacity 0.15s'}}
                        onMouseEnter={e=>e.currentTarget.style.opacity='1'} onMouseLeave={e=>e.currentTarget.style.opacity='0.5'}
                      ><Plus style={{width:10,height:10}} /></button>}
                      {editingField===fieldKey?<TagAutocomplete
                        autoFocus
                        placeholder={ph}
                        clearOnSelect={true}
                        keepOpen={true}
                        onSelect={(v) => { if(v.trim())addOne(v.trim()); }}
                        onBlur={() => setEditingField(null)}
                        onKeyDown={(e) => { if (e.key === 'Escape') setEditingField(null); }}
                        inputStyle={{fontSize:11,height:24,border:'none',background:'transparent',padding:'0 4px',flex:1,minWidth:60,outline:'none',maxWidth:200}}
                      />
                      :parts.length===0&&<span style={{fontSize:10,color:'var(--color-text-tertiary)',fontStyle:'italic',lineHeight:'24px'}}>{t('jsonTag.noTagData')}</span>}
                    </div>
                  );
                };
                return (<>
              {/* fixed — 固定字段 */}
              <div style={{marginBottom:8}}>
                <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:6}}>
                  <Lock style={{width:11,height:11,color:'#f59e0b'}} />
                  <span style={{fontSize:10,fontWeight:700,color:'#f59e0b'}}>{t('jsonTag.fixedSection')}</span>
                </div>
                <div style={{marginBottom:6}}>
                  <span style={{fontSize:9,fontWeight:600,color:'#f59e0b',opacity:0.7,marginBottom:2,display:'block'}}>{t('jsonTag.qualityLabel')}</span>
                  {fieldChips('f.quality',cur.data.fixed.quality,v=>updateData(d=>{d.fixed.quality=v;return d;}),()=>updateData(d=>{d.fixed.quality=undefined;return d;}),'#f59e0b',t('jsonTag.inputTag'))}
                </div>
                <div style={{marginBottom:6}}>
                  <span style={{fontSize:9,fontWeight:600,color:'#f59e0b',opacity:0.7,marginBottom:2,display:'block'}}>{t('jsonTag.seriesLabel')}</span>
                  {fieldChips('f.series',cur.data.fixed.series,v=>updateData(d=>{d.fixed.series=v;return d;}),()=>updateData(d=>{d.fixed.series=undefined;return d;}),'#f59e0b',t('jsonTag.inputTag'))}
                </div>
                <div>
                  <span style={{fontSize:9,fontWeight:600,color:'#f59e0b',opacity:0.7,marginBottom:2,display:'block'}}>{t('jsonTag.artistLabel')}</span>
                  {fieldChips('f.artist',cur.data.fixed.artist,v=>updateData(d=>{d.fixed.artist=v;return d;}),()=>updateData(d=>{d.fixed.artist=undefined;return d;}),'#f59e0b',t('jsonTag.inputTag'))}
                </div>
              </div>

              {/* character — 角色信息 */}
              <div style={{marginBottom:8}}>
                <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:6}}>
                  <User style={{width:11,height:11,color:'#f472b6'}} />
                  <span style={{fontSize:10,fontWeight:700,color:'#f472b6'}}>{t('jsonTag.characterSection')}</span>
                </div>
                <div style={{marginBottom:6}}>
                  <span style={{fontSize:9,fontWeight:600,color:'#f472b6',opacity:0.7,marginBottom:2,display:'block'}}>{t('jsonTag.nameLabel')}</span>
                  {fieldChips('c.name',cur.data.character.name||undefined,v=>updateData(d=>{d.character.name=v;return d;}),()=>updateData(d=>{d.character.name='';return d;}),'#f472b6',t('jsonTag.inputTag'))}
                </div>
                <div>
                  <span style={{fontSize:9,fontWeight:600,color:'#f472b6',opacity:0.7,marginBottom:2,display:'block'}}>{t('jsonTag.variantLabel')}</span>
                  {fieldChips('c.variant',cur.data.character.variant||undefined,v=>updateData(d=>{d.character.variant=v;return d;}),()=>updateData(d=>{d.character.variant='';return d;}),'#f472b6',t('jsonTag.inputTag'))}
                </div>
              </div>

              {/* from_path — 路径提取外观 */}
              <div style={{marginBottom:8}}>
                <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:6}}>
                  <Layers style={{width:11,height:11,color:'#22d3ee'}} />
                  <span style={{fontSize:10,fontWeight:700,color:'#22d3ee'}}>{t('jsonTag.fromPathSection')}</span>
                  <span style={{fontSize:9,padding:'0 5px',borderRadius:6,background:'rgba(34,211,238,0.08)',color:'#22d3ee',fontWeight:600}}>{cur.data.from_path.appearance.length}</span>
                </div>
                <div style={{padding:'5px 8px',borderRadius:'var(--radius-md)',background:'rgba(34,211,238,0.06)',border:'1px solid rgba(34,211,238,0.15)'}}>
                  {tagChips(cur.data.from_path.appearance,{bg:'rgba(34,211,238,0.10)',bd:'rgba(34,211,238,0.25)',tx:'#22d3ee'},t=>removeFromPathTag(t),'fp','fp',v=>updateData(d=>{d.from_path.appearance=[...d.from_path.appearance,v];return d;}))}
                </div>
              </div>

              {/* ai_output — VLM 打标输出 */}
              <div style={{marginBottom:8}}>
                <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:6}}>
                  <Sparkles style={{width:11,height:11,color:'#818cf8'}} />
                  <span style={{fontSize:10,fontWeight:700,color:'#818cf8'}}>{t('jsonTag.aiOutputSection')}</span>
                </div>
                <div style={{marginBottom:6}}>
                  <span style={{fontSize:9,fontWeight:600,color:'#818cf8',opacity:0.7,marginBottom:2,display:'block'}}>{t('jsonTag.countLabel')}</span>
                  {fieldChips('ai.count',cur.data.ai_output.count,v=>updateData(d=>{d.ai_output.count=v;return d;}),()=>updateData(d=>{d.ai_output.count=undefined;return d;}),'#818cf8',t('jsonTag.inputTag'))}
                </div>
                {AI_CATS_KEYS.map(cat=>{const Icon=cat.icon;const arr=cur.data.ai_output[cat.key]||[];const cc=chipC[cat.key];const editKey=`ai.${cat.key}`;return(
                  <div key={cat.key} style={{marginBottom:6}}>
                    <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:4}}>
                      <Icon style={{width:11,height:11,color:cat.color}} />
                      <span style={{fontSize:10,fontWeight:600,color:cat.color}}>{t(cat.labelKey)}</span>
                      <span style={{fontSize:9,padding:'0 5px',borderRadius:6,background:cat.bg,color:cat.color,fontWeight:600}}>{arr.length}</span>
                    </div>
                    <div style={{padding:'5px 8px',borderRadius:'var(--radius-md)',background:cat.bg,border:`1px solid ${cat.bd}`}}>
                      {tagChips(arr,cc,t=>removeAiTag(cat.key,t),cat.key,editKey,v=>updateData(d=>{const s=new Set(d.ai_output[cat.key]);if(!s.has(v)){d.ai_output[cat.key].push(v);}return d;}))}
                    </div>
                  </div>
                );})}              </div>
                </>);
              })()}

              {/* nl — 自然语言描述 */}
              <div>
                <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:6}}>
                  <Eye style={{width:11,height:11,color:'#94a3b8'}} />
                  <span style={{fontSize:10,fontWeight:700,color:'#94a3b8'}}>{t('jsonTag.nlSection')}</span>
                </div>
                <textarea className="form-input" value={cur.data.ai_output.nl||''} onChange={e=>updateData(d=>{d.ai_output.nl=e.target.value||undefined;return d;})} placeholder="A girl stands under the sky..." style={{fontSize:11,minHeight:56,resize:'vertical',lineHeight:1.6,borderRadius:8,padding:'6px 10px'}} />
              </div>
            </>)}
          </div>

        </div>
      </div>

      {/* resize handle 2 */}
      <div onMouseDown={e=>handleColResize('col3',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}} title={t('jsonTag.dragWidth')}>
        <div style={{width:2,height:32,borderRadius:1,background:'var(--color-border)',transition:'background 0.15s'}} />
      </div>

      {/* Col3: Tag Viewer (switchable JSON/Stats) */}
      <div style={{width:col3W,minWidth:160,maxWidth:500,flexShrink:0,display:'flex',background:'var(--color-bg-secondary)',borderRadius:12,border:'1px solid var(--color-border)',overflow:'hidden'}}>
        <div style={{flex:1,display:'flex',flexDirection:'column',overflow:'hidden'}}>
          <div style={phdr}>
            <div style={{display:'flex',alignItems:'center',gap:8}}>
              {col3Mode==='stats'?<BarChart3 style={{width:14,height:14,color:'#60a5fa'}} />:<Code style={{width:14,height:14,color:'#60a5fa'}} />}
              <span style={ptitle}>{col3Mode==='stats'?(tagListMode==='common'?t('jsonTag.commonTags'):t('jsonTag.allTags')):t('jsonTag.tagContent')}</span>
              {col3Mode==='stats'&&<span style={{fontSize:10,padding:'1px 8px',borderRadius:10,background:'rgba(96,165,250,0.1)',color:'#60a5fa',fontWeight:600}}>{filteredStats.length}</span>}
            </div>
            <div style={{display:'flex',alignItems:'center',gap:4}}>
              {col3Mode==='stats'&&<button className="btn btn-ghost btn-sm" style={{fontSize:9,height:22,padding:'0 6px'}} onClick={()=>setTagListMode(m=>m==='all'?'common':'all')} disabled={images.length===0}>
                {tagListMode==='all'?t('jsonTag.commonLabel'):t('jsonTag.allLabel')}
              </button>}
              <button className="btn btn-ghost btn-sm" onClick={()=>setCol3Mode(m=>m==='json'?'stats':'json')} title={col3Mode==='json'?t('jsonTag.allTags'):t('jsonTag.tagContent')} style={{width:22,height:22,padding:0,display:'flex',alignItems:'center',justifyContent:'center',color:col3Mode==='stats'?'#60a5fa':undefined}}>
                {col3Mode==='json'?<BarChart3 style={{width:12,height:12}} />:<Code style={{width:12,height:12}} />}
              </button>
            </div>
          </div>

          {col3Mode==='stats'?(<>
            {/* Search + Sort */}
            <div style={{padding:'8px 10px',borderBottom:'1px solid var(--color-border)'}}>
              <div style={{display:'flex',gap:4}}>
                <div style={{position:'relative',flex:1}}>
                  <Search style={{position:'absolute',left:8,top:'50%',transform:'translateY(-50%)',width:12,height:12,color:'var(--color-text-tertiary)'}} />
                  <input className="form-input" placeholder={t('jsonTag.searchTags')} value={globalSearch} onChange={e=>setGlobalSearch(e.target.value)} style={{paddingLeft:26,fontSize:11,height:28}} />
                </div>
                <button className="btn btn-ghost btn-sm" onClick={()=>setTagSortBy(b=>b==='freq'?'name':'freq')}
                  title={tagSortBy==='freq'?t('jsonTag.sortByFreq'):t('jsonTag.sortByName')}
                  style={{width:28,height:28,padding:0,display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0,color:tagSortBy==='freq'?'#60a5fa':'#a78bfa'}}>
                  {tagSortBy==='freq'?<BarChart style={{width:13,height:13}} />:<Hash style={{width:13,height:13}} />}
                </button>
                <button className="btn btn-ghost btn-sm" onClick={()=>setTagSortDir(d=>d==='desc'?'asc':'desc')}
                  title={tagSortDir==='desc'?t('jsonTag.descOrder'):t('jsonTag.ascOrder')}
                  style={{width:28,height:28,padding:0,display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0,color:'var(--color-text-tertiary)'}}>
                  <ArrowUpDown style={{width:13,height:13,transform:tagSortDir==='asc'?'scaleY(-1)':undefined,transition:'transform 0.2s'}} />
                </button>
              </div>
            </div>
            {/* Filter indicator */}
            {tagFilterActive&&<div style={{padding:'4px 10px',background:'linear-gradient(90deg,rgba(124,92,252,0.08),rgba(124,92,252,0.02))',borderBottom:'1px solid var(--color-border)',display:'flex',alignItems:'center',gap:6}}>
              <Filter style={{width:10,height:10,color:'#7c5cfc',flexShrink:0}} />
              <span style={{fontSize:10,color:'#a78bfa',flex:1}}>{t('jsonTag.filtering')} <b>{filtered.length}</b>/{images.length}</span>
              <button onClick={()=>setTagFilterActive(false)} style={{display:'flex',alignItems:'center',justifyContent:'center',width:16,height:16,borderRadius:'50%',background:'rgba(248,113,113,0.1)',border:'none',cursor:'pointer',color:'#f87171',padding:0,flexShrink:0}}><X style={{width:8,height:8}} /></button>
            </div>}
            {/* Tag list */}
            <div style={{flex:1,overflowY:'auto',userSelect:'none'}}>
              {filteredStats.map(([tag,count])=>{
                const c=getStatColor(tag);const pct=images.length>0?(count/images.length)*100:0;
                const inCur=cur?[...cur.data.ai_output.appearance,...cur.data.ai_output.tags,...cur.data.ai_output.environment,...cur.data.from_path.appearance,...(cur.data.fixed.quality?.split(',').map(s=>s.trim())||[])].includes(tag):false;
                const tr=translations[tag];
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
              {images.length===0&&<div style={{display:'flex',flexDirection:'column',alignItems:'center',justifyContent:'center',height:'100%',gap:8,color:'var(--color-text-tertiary)',padding:20}}><Tags style={{width:28,height:28,opacity:0.2}} /><span style={{fontSize:11,opacity:0.6}}>{t('jsonTag.loadTagsHint')}</span></div>}
              {images.length>0&&filteredStats.length===0&&<div style={{padding:20,textAlign:'center',fontSize:11,color:'var(--color-text-tertiary)'}}>{globalSearch?t('jsonTag.noMatch'):t('jsonTag.noTagData')}</div>}
            </div>
            {/* Footer */}
            <div style={{padding:'6px 12px',borderTop:'1px solid var(--color-border)',fontSize:10,color:'var(--color-text-tertiary)',display:'flex',justifyContent:'space-between'}}>
              {selectedTags.size>0?<span style={{color:'#a78bfa'}}>{t('jsonTag.selected',{n:selectedTags.size})}</span>:<span>{t('jsonTag.nTagTypes',{n:tagStats.length})}</span>}
              <span>{taggedCount}/{images.length} {t('jsonTag.tagged')}</span>
            </div>
          </>):(
            /* JSON Preview */
            <div style={{flex:1,overflowY:'auto',padding:'10px 12px',fontSize:11}}>
              {!cur?<span style={{color:'var(--color-text-tertiary)',fontStyle:'italic'}}>{t('jsonTag.selectToView')}</span>:(()=>{
                const d=cur.data;
                const clean=(obj:Record<string,any>)=>{const r:Record<string,any>={};for(const[k,v]of Object.entries(obj)){if(v!==undefined&&v!==null&&v!=='')r[k]=v;}return Object.keys(r).length?r:undefined;};
                let json:any;
                if(simplified){
                  const charParts:string[]=[];
                  if(d.character.name)d.character.name.split(',').map(s=>s.trim()).filter(Boolean).forEach(t=>charParts.push(t));
                  if(d.character.variant)d.character.variant.split(',').map(s=>s.trim()).filter(Boolean).forEach(t=>charParts.push(t));
                  const charVal=charParts.length>1?charParts.join(', '):charParts.length===1?charParts[0]:undefined;
                  const allAppearance=[...d.from_path.appearance,...d.ai_output.appearance];
                  json=clean({quality:d.fixed.quality||undefined,series:d.fixed.series||undefined,artist:d.fixed.artist||undefined,character:charVal,count:d.ai_output.count||undefined,appearance:allAppearance.length?allAppearance:undefined,tags:d.ai_output.tags.length?d.ai_output.tags:undefined,environment:d.ai_output.environment.length?d.ai_output.environment:undefined,nl:d.ai_output.nl||undefined});
                }else{
                  const fixed=clean({quality:d.fixed.quality||undefined,series:d.fixed.series||undefined,artist:d.fixed.artist||undefined});
                  const character=clean({name:d.character.name||undefined,variant:d.character.variant||undefined});
                  const from_path=d.from_path.appearance.length?{appearance:d.from_path.appearance}:undefined;
                  const ai_output=clean({count:d.ai_output.count||undefined,appearance:d.ai_output.appearance.length?d.ai_output.appearance:undefined,tags:d.ai_output.tags.length?d.ai_output.tags:undefined,environment:d.ai_output.environment.length?d.ai_output.environment:undefined,nl:d.ai_output.nl||undefined});
                  const result:Record<string,any>={};
                  if(fixed)result.fixed=fixed;if(character)result.character=character;if(from_path)result.from_path=from_path;if(ai_output)result.ai_output=ai_output;
                  json=Object.keys(result).length?result:undefined;
                }
                return json?(
                  <pre style={{margin:0,whiteSpace:'pre-wrap',wordBreak:'break-all',fontFamily:'"SF Mono","Fira Code","Cascadia Code",Menlo,Consolas,monospace',fontSize:10,lineHeight:1.7,color:'var(--color-text-primary)'}}>{JSON.stringify(json,null,2)}</pre>
                ):(<span style={{color:'var(--color-text-tertiary)',fontStyle:'italic'}}>{t('jsonTag.noTagDataView')}</span>);
              })()}
              {nlTranslation&&<div style={{marginTop:8,padding:'6px 8px',borderRadius:6,background:'rgba(148,163,184,0.06)',border:'1px solid rgba(148,163,184,0.1)',fontSize:10,color:'var(--color-text-tertiary)',lineHeight:1.6}}>
                <span style={{fontWeight:600}}>{t('jsonTag.nlTranslation')}:</span> {nlTranslation}
              </div>}
            </div>
          )}
        </div>

        {/* Sidebar action buttons */}
        {col3Mode==='stats'&&<div style={{display:'flex',flexDirection:'column',gap:2,padding:'8px 4px',borderLeft:'1px solid var(--color-border)',alignItems:'center'}}>
          {[
            {icon:<Filter style={{width:14,height:14}} />,tip:t('jsonTag.filterByTag'),onClick:()=>setTagFilterActive(v=>!v),disabled:images.length===0,color:tagFilterActive?'#7c5cfc':undefined},
            {icon:<ListPlus style={{width:14,height:14}} />,tip:t('jsonTag.batchAdd'),onClick:()=>{setBatchField('ai_output.tags');setBatchTags('');setBatchPosition('append');setShowBatchAddModal(true);},disabled:images.length===0},
            {icon:<ListX style={{width:14,height:14}} />,tip:t('jsonTag.batchDelete'),onClick:()=>{setBatchField('all');setBatchTags('');setShowBatchDeleteModal(true);},disabled:images.length===0},
            {icon:<Trash2 style={{width:14,height:14}} />,tip:t('jsonTag.deleteSelected'),onClick:handleSidebarBatchDelete,disabled:selectedTags.size===0,color:selectedTags.size>0?'#f87171':undefined},
          ].map((item,i)=>(
            <button key={i} className="btn btn-ghost" title={item.tip} disabled={item.disabled}
              onClick={item.onClick}
              style={{width:30,height:30,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6,color:item.color}}>
              {item.icon}
            </button>
          ))}
        </div>}
      </div>

      {/* Batch Add Modal */}
      {showBatchAddModal&&<div style={{position:'fixed',inset:0,zIndex:999,display:'flex',alignItems:'center',justifyContent:'center',background:'rgba(0,0,0,0.5)',backdropFilter:'blur(4px)'}} onClick={()=>setShowBatchAddModal(false)}>
        <div onClick={e=>e.stopPropagation()} style={{width:440,background:'var(--color-bg-card)',borderRadius:16,border:'1px solid var(--color-border)',boxShadow:'0 20px 60px rgba(0,0,0,0.3)',overflow:'hidden'}}>
          <div style={{padding:'16px 20px',borderBottom:'1px solid var(--color-border)',display:'flex',alignItems:'center',gap:8}}>
            <ListPlus style={{width:16,height:16,color:'#4ade80'}} />
            <span style={{fontSize:13,fontWeight:700,color:'var(--color-text-primary)'}}>{t('jsonTag.batchAddTitle')}</span>
          </div>
          <div style={{padding:'16px 20px',display:'flex',flexDirection:'column',gap:14}}>
            <div>
              <span style={{fontSize:11,fontWeight:600,color:'var(--color-text-secondary)',display:'block',marginBottom:6}}>{t('jsonTag.targetField')}</span>
              <div style={{display:'flex',flexWrap:'wrap',gap:4}}>
                {BATCH_FIELD_OPTIONS.map(o=>(
                  <button key={o.value} onClick={()=>setBatchField(o.value)}
                    style={{padding:'4px 10px',borderRadius:8,fontSize:10,fontWeight:600,border:'1px solid',cursor:'pointer',transition:'all 0.15s',fontFamily:'inherit',
                      background:batchField===o.value?'rgba(124,92,252,0.15)':'var(--color-bg-input)',
                      borderColor:batchField===o.value?'rgba(124,92,252,0.4)':'var(--color-border)',
                      color:batchField===o.value?'#a78bfa':'var(--color-text-tertiary)'}}>
                    {o.label}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <span style={{fontSize:11,fontWeight:600,color:'var(--color-text-secondary)',display:'block',marginBottom:6}}>{t('jsonTag.position')}</span>
              <div style={{display:'flex',gap:4}}>
                {[{v:'prepend' as const,l:t('jsonTag.prepend')},{v:'append' as const,l:t('jsonTag.append')}].map(p=>(
                  <button key={p.v} onClick={()=>setBatchPosition(p.v)}
                    style={{padding:'4px 14px',borderRadius:8,fontSize:10,fontWeight:600,border:'1px solid',cursor:'pointer',transition:'all 0.15s',fontFamily:'inherit',
                      background:batchPosition===p.v?'rgba(74,222,128,0.15)':'var(--color-bg-input)',
                      borderColor:batchPosition===p.v?'rgba(74,222,128,0.4)':'var(--color-border)',
                      color:batchPosition===p.v?'#4ade80':'var(--color-text-tertiary)'}}>
                    {p.l}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <span style={{fontSize:11,fontWeight:600,color:'var(--color-text-secondary)',display:'block',marginBottom:4}}>{t('jsonTag.batchTagsPlaceholder')}</span>
              <input autoFocus className="form-input" value={batchTags} onChange={e=>setBatchTags(e.target.value)}
                onKeyDown={e=>{if(e.key==='Enter'&&batchTags.trim())handleBatchAdd();}}
                placeholder="tag1, tag2, tag3" style={{fontSize:12,height:34,width:'100%'}} />
            </div>
          </div>
          <div style={{padding:'12px 20px',borderTop:'1px solid var(--color-border)',display:'flex',justifyContent:'flex-end',gap:8}}>
            <button className="btn btn-ghost" onClick={()=>setShowBatchAddModal(false)} style={{fontSize:11,height:30,padding:'0 16px'}}>{t('jsonTag.cancel')}</button>
            <button className="btn btn-primary" onClick={handleBatchAdd} disabled={!batchTags.trim()} style={{fontSize:11,height:30,padding:'0 16px',gap:4}}>
              <ListPlus style={{width:12,height:12}} /> {t('jsonTag.batchApplyAdd')}
            </button>
          </div>
        </div>
      </div>}

      {/* Batch Delete Modal */}
      {showBatchDeleteModal&&<div style={{position:'fixed',inset:0,zIndex:999,display:'flex',alignItems:'center',justifyContent:'center',background:'rgba(0,0,0,0.5)',backdropFilter:'blur(4px)'}} onClick={()=>setShowBatchDeleteModal(false)}>
        <div onClick={e=>e.stopPropagation()} style={{width:440,background:'var(--color-bg-card)',borderRadius:16,border:'1px solid var(--color-border)',boxShadow:'0 20px 60px rgba(0,0,0,0.3)',overflow:'hidden'}}>
          <div style={{padding:'16px 20px',borderBottom:'1px solid var(--color-border)',display:'flex',alignItems:'center',gap:8}}>
            <ListX style={{width:16,height:16,color:'#f87171'}} />
            <span style={{fontSize:13,fontWeight:700,color:'var(--color-text-primary)'}}>{t('jsonTag.batchDeleteTitle')}</span>
          </div>
          <div style={{padding:'16px 20px',display:'flex',flexDirection:'column',gap:14}}>
            <div>
              <span style={{fontSize:11,fontWeight:600,color:'var(--color-text-secondary)',display:'block',marginBottom:6}}>{t('jsonTag.targetField')}</span>
              <div style={{display:'flex',flexWrap:'wrap',gap:4}}>
                <button onClick={()=>setBatchField('all')}
                  style={{padding:'4px 10px',borderRadius:8,fontSize:10,fontWeight:600,border:'1px solid',cursor:'pointer',transition:'all 0.15s',fontFamily:'inherit',
                    background:batchField==='all'?'rgba(248,113,113,0.15)':'var(--color-bg-input)',
                    borderColor:batchField==='all'?'rgba(248,113,113,0.4)':'var(--color-border)',
                    color:batchField==='all'?'#f87171':'var(--color-text-tertiary)'}}>
                  {t('jsonTag.allFields')}
                </button>
                {BATCH_FIELD_OPTIONS.map(o=>(
                  <button key={o.value} onClick={()=>setBatchField(o.value)}
                    style={{padding:'4px 10px',borderRadius:8,fontSize:10,fontWeight:600,border:'1px solid',cursor:'pointer',transition:'all 0.15s',fontFamily:'inherit',
                      background:batchField===o.value?'rgba(248,113,113,0.15)':'var(--color-bg-input)',
                      borderColor:batchField===o.value?'rgba(248,113,113,0.4)':'var(--color-border)',
                      color:batchField===o.value?'#f87171':'var(--color-text-tertiary)'}}>
                    {o.label}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <span style={{fontSize:11,fontWeight:600,color:'var(--color-text-secondary)',display:'block',marginBottom:4}}>{t('jsonTag.batchTagsPlaceholder')}</span>
              <input autoFocus className="form-input" value={batchTags} onChange={e=>setBatchTags(e.target.value)}
                onKeyDown={e=>{if(e.key==='Enter'&&batchTags.trim())handleBatchDelete();}}
                placeholder="tag1, tag2, tag3" style={{fontSize:12,height:34,width:'100%'}} />
            </div>
          </div>
          <div style={{padding:'12px 20px',borderTop:'1px solid var(--color-border)',display:'flex',justifyContent:'flex-end',gap:8}}>
            <button className="btn btn-ghost" onClick={()=>setShowBatchDeleteModal(false)} style={{fontSize:11,height:30,padding:'0 16px'}}>{t('jsonTag.cancel')}</button>
            <button className="btn btn-primary" onClick={handleBatchDelete} disabled={!batchTags.trim()} style={{fontSize:11,height:30,padding:'0 16px',gap:4,background:'rgba(248,113,113,0.9)'}}>
              <ListX style={{width:12,height:12}} /> {t('jsonTag.batchApplyDelete')}
            </button>
          </div>
        </div>
      </div>}
    </div>
  );
});

export default JsonTagTab;
