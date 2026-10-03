import { JSON_FIELDS, collectAllTags, jsonTagPreview, moveJsonTag, type JsonTagData } from '../utils/jsonTagFields';
import { dedupeTags, splitTagInput } from '../utils/tagText';
import { useTagStats } from '../hooks/useTagStats';
import { useTagTranslation } from '../hooks/useTagTranslation';
import { useDragResize } from '../hooks/useDragResize';
import { useTagFieldDrag } from '../hooks/useTagFieldDrag';
import TagChipList from './TagChipList';
import ImageGridColumn from './ImageGridColumn';
import TagStatsPanel from './TagStatsPanel';
import ScopeToggle from './ScopeToggle';
import { Modal, AlertModal } from './Modal';
import { useState, useEffect, useMemo, useCallback, forwardRef, useImperativeHandle } from 'react';
import TagAutocomplete from './TagAutocomplete';
import ImageLightbox from './ImageLightbox';
import ThumbImage from './ThumbImage';
import { invoke } from '@tauri-apps/api/core';
import { ensureAssetScope } from '../utils/assetScope';
import { convertFileSrc } from '@tauri-apps/api/core';
import { open as dialogOpen } from '@tauri-apps/plugin-dialog';
import { Save, ChevronLeft, ChevronRight, Image as ImageIcon, Loader2, Sparkles, Eye, Languages, ListPlus, ListX, BarChart3, Filter, Code, Trash2, CopyX, Lock, User, Layers, Shirt, Tags, TreePine } from 'lucide-react';
import { useTranslation } from 'react-i18next';

/** parse_failed：JSON 存在但解析失败，data 只是空默认值。保存时会跳过这类条目，
 *  单图编辑和批量增删也跳过它们，免得被标成未保存却永远存不进文件。 */
interface JsonImageItem { path: string; filename: string; data: JsonTagData; has_json: boolean; parse_failed?: boolean; dirty?: boolean; }
interface JsonDataset { folder: string; images: JsonImageItem[]; detected_format: string; }

const phdr: React.CSSProperties = { display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: 6, padding: '10px 14px', borderBottom: '1px solid var(--color-border)', flexShrink: 0 };
const ptitle: React.CSSProperties = { fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)', whiteSpace: 'nowrap' };
const sections = [
  { key: 'fixed', label: 'fixedSection', icon: Lock, color: '#f59e0b' },
  { key: 'character', label: 'characterSection', icon: User, color: '#f472b6' },
  { key: 'from_path', label: 'fromPathSection', icon: Layers, color: '#22d3ee' },
  { key: 'ai_output', label: 'aiOutputSection', icon: Sparkles, color: '#818cf8' },
] as const;
const listIcons = { appearance: Shirt, tags: Tags, environment: TreePine };

export interface JsonTagTabHandle {
  loadFolder: () => Promise<void>;
  saveAll: () => Promise<void>;
}

const JsonTagTab = forwardRef<JsonTagTabHandle, {
  recursive?: boolean;
  onDirtyChange?: (count: number) => void;
  onLoadingChange?: (loading: boolean) => void;
  onSavingChange?: (saving: boolean) => void;
}>(function JsonTagTab({ recursive = false, onDirtyChange, onLoadingChange, onSavingChange }, ref) {
  const { t } = useTranslation();
  const [images,setImages]=useState<JsonImageItem[]>([]);
  const [selectedIdx,setSelectedIdx]=useState(-1);
  const [folderPath,setFolderPath]=useState('');
  const [loading,setLoading]=useState(false);
  const [saving,setSaving]=useState(false);
  const [savingSingle,setSavingSingle]=useState(false);
  const [searchText,setSearchText]=useState('');
  const [filterMode,setFilterMode]=useState<'all'|'tagged'|'untagged'>('all');

  const [simplified,setSimplified]=useState(()=>localStorage.getItem('json_tag_simplified')==='true');
  const { translations, translating, translateProgress, translate } = useTagTranslation();
  const [alertMsg, setAlertMsg] = useState('');
  const [showLargePreview,setShowLargePreview]=useState(false);
  const col1 = useDragResize({ initial: 220, min: 160, max: 500, storageKey: 'json_col1w' });
  const col3 = useDragResize({ initial: 220, min: 160, max: 500, direction: -1, storageKey: 'json_col3w' });
  const preview = useDragResize({ initial: 220, min: 100, max: 500, axis: 'y', storageKey: 'json_previewh' });
  const col1W = col1.size, col3W = col3.size, previewH = preview.size;
  const handleColResize = (column: 'col1' | 'col3', e: React.MouseEvent) => (column === 'col1' ? col1 : col3).onMouseDown(e);
  const handleRowResize = preview.onMouseDown;
  const tagLists = useMemo(() => images.map(image => collectAllTags(image.data)), [images]);
  const stats = useTagStats(tagLists, folderPath, translations);
  const { selectedTags, setSelectedTags, tagListMode, setTagListMode, filteredStats, tagStats } = stats;

  const [showBatchAddModal,setShowBatchAddModal]=useState(false);
  const [showBatchDeleteModal,setShowBatchDeleteModal]=useState(false);
  const [batchField,setBatchField]=useState('fixed.quality');
  const [batchTags,setBatchTags]=useState('');
  const [batchPosition,setBatchPosition]=useState<'prepend'|'append'>('prepend');
  /** 批量添加/删除的应用范围：当前图片 or 全部图片 */
  const [batchScope,setBatchScope]=useState<'current'|'all'>('all');
  const [showSelDeleteModal,setShowSelDeleteModal]=useState(false);
  const [selDeleteScope,setSelDeleteScope]=useState<'current'|'all'>('all');

  const visibleBatchFieldOptions = JSON_FIELDS.filter(field => !simplified || field.simplified).map(field => ({
    value: field.key, label: field.name + ' - ' + t(field.labelKey),
  }));

  const [col3Mode,setCol3Mode]=useState<'json'|'stats'>('stats');
  const [tagFilterActive,setTagFilterActive]=useState(false);

  const handleLoadFolder=useCallback(async()=>{
    const sel=await dialogOpen({directory:true,multiple:false,title:t('jsonTag.selectFolder')});
    if(!sel)return; setLoading(true);
    try{await ensureAssetScope(sel as string);
      const r=await invoke<JsonDataset>('load_json_dataset',{folder:sel as string, recursive});
      setImages(r.images.map(img=>({...img,dirty:false})));setSelectedIdx(r.images.length>0?0:-1);
      setFolderPath(sel as string);setSearchText('');setFilterMode('all');
      if(r.detected_format==='simplified'){setSimplified(true);localStorage.setItem('json_tag_simplified','true');}
      else if(r.detected_format==='full'){setSimplified(false);localStorage.setItem('json_tag_simplified','false');}
    }catch(e){console.error(e);}finally{setLoading(false);}
  },[recursive,t]);
  const handleRefresh=useCallback(async()=>{if(!folderPath)return;setLoading(true);
    try{await ensureAssetScope(folderPath);
      const r=await invoke<JsonDataset>('load_json_dataset',{folder:folderPath, recursive});
      setImages(r.images.map(img=>({...img,dirty:false})));
    }catch(e){console.error(e);}finally{setLoading(false);}
  },[folderPath,recursive]);

  const cur=selectedIdx>=0&&selectedIdx<images.length?images[selectedIdx]:null;
  const imgSrc=cur?convertFileSrc(cur.path):'';
  const dirtyCount=images.filter(i=>i.dirty).length;

  const handleSaveSingle=useCallback(async()=>{if(!cur||!cur.dirty||cur.parse_failed)return;setSavingSingle(true);
    try{await invoke('save_single_json_file',{imagePath:cur.path,data:cur.data,simplified});
      setImages(p=>p.map((img,i)=>i===selectedIdx?{...img,dirty:false}:img));
    }catch(e){console.error(e);}finally{setSavingSingle(false);}
  },[cur,selectedIdx,simplified]);
  const handleSaveAll=useCallback(async()=>{const dirty=images.filter(i=>i.dirty&&!i.parse_failed).map(i=>({path:i.path,data:i.data}));
    if(!dirty.length)return;setSaving(true);
    try{await invoke<number>('save_all_json_files',{items:dirty,simplified});
      setImages(p=>p.map(img=>img.dirty?{...img,dirty:false}:img));
    }catch(e){console.error(e);}finally{setSaving(false);}
  },[images,simplified]);

  useImperativeHandle(ref, () => ({
    loadFolder: handleLoadFolder,
    saveAll: handleSaveAll,
  }), [handleLoadFolder, handleSaveAll]);

  useEffect(() => { onDirtyChange?.(dirtyCount); }, [dirtyCount, onDirtyChange]);
  useEffect(() => { onLoadingChange?.(loading); }, [loading, onLoadingChange]);
  useEffect(() => { onSavingChange?.(saving); }, [saving, onSavingChange]);
  const goPrev=()=>{if(selectedIdx>0)setSelectedIdx(selectedIdx-1);};
  const goNext=()=>{if(selectedIdx<images.length-1)setSelectedIdx(selectedIdx+1);};

  const filtered=useMemo(()=>{
    let list=images.map((img,i)=>({...img,_i:i}));
    if(searchText){
      const q=searchText.toLowerCase();
      list=list.filter(img=>{
        if(img.filename.toLowerCase().includes(q)) return true;
        // 搜索比筛选多带一个 nl（自然语言描述里也能搜到词）
        const all=[...collectAllTags(img.data),img.data.ai_output?.nl||''];
        return all.some(t=>t.toLowerCase().includes(q));
      });
    }
    if(filterMode==='tagged')list=list.filter(img=>img.has_json);
    if(filterMode==='untagged')list=list.filter(img=>!img.has_json);
    if(tagFilterActive&&selectedTags.size>0){
      list=list.filter(img=>{
        const allTags=collectAllTags(img.data);
        return [...selectedTags].every(t=>allTags.includes(t));
      });
    }
    return list;
  },[images,searchText,filterMode,tagFilterActive,selectedTags]);

  const taggedCount = images.filter(image => image.has_json).length;
  const currentTags = useMemo(() => new Set(cur ? collectAllTags(cur.data) : []), [cur]);
  const updateData = useCallback((fn: (data: JsonTagData) => JsonTagData) => {
    setImages(previous => previous.map((image, index) => {
      if (index !== selectedIdx || image.parse_failed) return image;
      const data = fn(image.data);
      return data === image.data ? image : { ...image, data, dirty: true };
    }));
  }, [selectedIdx]);
  const tagDrag = useTagFieldDrag({
    scope: `${cur?.path ?? ''}:${simplified}`, revision: cur?.data, disabled: !cur || !!cur.parse_failed || loading,
    onDrop: (source, target) => updateData(data => moveJsonTag(data, source, target, simplified)),
  });
  const removeTags = (tags: Set<string>, scope: 'current' | 'all', field = 'all') => {
    if (scope === 'current' && !cur) return;
    const keys = new Set([...tags].map(tag => tag.toLowerCase()));
    setImages(previous => previous.map((image, index) => {
      if (image.parse_failed || scope === 'current' && index !== selectedIdx) return image;
      let data = image.data;
      for (const definition of JSON_FIELDS) {
        if (field !== 'all' && definition.key !== field) continue;
        const values = definition.get(data), next = values.filter(tag => !keys.has(tag.toLowerCase()));
        if (next.length !== values.length) data = definition.set(data, next);
      }
      return data === image.data ? image : { ...image, data, dirty: true };
    }));
  };
  const handleSidebarBatchDelete = () => {
    removeTags(selectedTags, selDeleteScope); setSelectedTags(new Set()); setShowSelDeleteModal(false);
  };
  const handleBatchAdd = () => {
    const tags = splitTagInput(batchTags), field = JSON_FIELDS.find(field => field.key === batchField);
    if (!field || !tags.length || batchScope === 'current' && !cur) return;
    setImages(previous => previous.map((image, index) => {
      if (image.parse_failed || batchScope === 'current' && index !== selectedIdx) return image;
      const values = field.get(image.data), keys = new Set(values.map(tag => tag.toLowerCase()));
      const incoming = tags.filter(tag => !keys.has(tag.toLowerCase()));
      if (!incoming.length) return image;
      const data = field.set(image.data, batchPosition === 'prepend' ? [...incoming, ...values] : [...values, ...incoming]);
      return { ...image, data, dirty: true };
    }));
    setBatchTags(''); setShowBatchAddModal(false);
  };
  const handleBatchDelete = () => {
    removeTags(new Set(splitTagInput(batchTags)), batchScope, batchField);
    setBatchTags(''); setShowBatchDeleteModal(false);
  };
  const handleDeduplicateTags = () => {
    setImages(previous => previous.map(image => {
      if (image.parse_failed) return image;
      let data = image.data;
      const seen = new Set<string>();
      for (const field of JSON_FIELDS) data = field.set(data, dedupeTags(field.get(data), seen).tags);
      return data === image.data ? image : { ...image, data, dirty: true };
    }));
  };
  const handleTranslate = async () => {
    if (localStorage.getItem('translate_enabled') !== 'true') return;
    try { await translate(tagStats.map(([tag]) => tag)); }
    catch (error) { setAlertMsg(t('tagManager.translateFail') + ': ' + String(error)); }
  };

  return (
    <div style={{flex:1,display:'flex',overflow:'hidden',minHeight:0}}>
      <ImageGridColumn key={folderPath} width={col1W} items={filtered} total={images.length} tagged={taggedCount}
        search={searchText} onSearch={setSearchText} filter={filterMode} onFilter={setFilterMode}
        selected={selectedIdx} onSelect={setSelectedIdx} onRefresh={folderPath ? handleRefresh : undefined} loading={loading}
        badge={image => image.has_json ? String(collectAllTags(image.data).length) : null} />

      {/* resize handle 1 */}
      <div onMouseDown={e=>handleColResize('col1',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}>
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
              {cur?.parse_failed&&<span style={{fontSize:10,color:'#f87171',fontWeight:600,padding:'1px 6px',borderRadius:4,background:'rgba(248,113,113,0.12)',border:'1px solid rgba(248,113,113,0.3)'}}>{t('jsonTag.parseFailed')}</span>}
            </div>
            <div style={{display:'flex',alignItems:'center',gap:6}}>
              <button className="btn btn-ghost btn-sm" onClick={goPrev} disabled={selectedIdx<=0} style={{width:26,height:26,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6}}><ChevronLeft style={{width:14,height:14}} /></button>
              <span style={{fontSize:11,color:'var(--color-text-tertiary)',minWidth:50,textAlign:'center'}}>{selectedIdx+1}/{images.length}</span>
              <button className="btn btn-ghost btn-sm" onClick={goNext} disabled={selectedIdx>=images.length-1} style={{width:26,height:26,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6}}><ChevronRight style={{width:14,height:14}} /></button>
            </div>
          </div>
          <div style={{flex:1,display:'flex',alignItems:'center',justifyContent:'center',background:'rgba(0,0,0,0.15)',minHeight:0,overflow:'hidden'}}>
            {cur?<ThumbImage path={cur.path} maxEdge={1024} alt={cur.filename} draggable={false} onClick={()=>setShowLargePreview(true)} style={{maxWidth:'100%',maxHeight:'100%',objectFit:'contain',cursor:'zoom-in'}} />
              :<div style={{display:'flex',flexDirection:'column',alignItems:'center',gap:8,color:'var(--color-text-tertiary)'}}><ImageIcon style={{width:48,height:48,opacity:0.2}} /><span style={{fontSize:12,opacity:0.6}}>{images.length===0?'':t('jsonTag.selectToPreview')}</span></div>}
          </div>
        </div>

        {/* row resize handle */}
        <div onMouseDown={handleRowResize} style={{height:6,cursor:'row-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}>
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
              <button className="btn btn-primary" style={{fontSize:10,gap:4,height:24,padding:'0 10px'}} disabled={!cur||!cur.dirty||savingSingle} onClick={handleSaveSingle}>
                {savingSingle?<Loader2 style={{width:10,height:10,animation:'spin 1s linear infinite'}} />:<Save style={{width:10,height:10}} />} {t('jsonTag.save')}
              </button>
            </div>
          </div>

          <div {...tagDrag.editorProps} style={{flex:1,overflowY:'auto',padding:'12px 14px',display:'flex',flexDirection:'column',gap:14}}>
            {!cur?<span style={{fontSize:11,color:'var(--color-text-tertiary)',fontStyle:'italic'}}>{t('jsonTag.selectToEdit')}</span>:(<>
              {sections.filter(section => !simplified || section.key !== 'from_path').map(section => (
                <div key={section.key} style={{ marginBottom: 8 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: 10, fontWeight: 700, marginBottom: 6, color: section.color }}>
                    <section.icon size={11} /><span>{t(`jsonTag.${section.label}`)}</span>
                    {section.key === 'from_path' && <span style={{ fontSize: 9, padding: '0 5px', borderRadius: 6, background: `${section.color}14` }}>{cur.data.from_path.appearance.length}</span>}
                  </div>
                  {JSON_FIELDS.filter(field => field.section === section.key && (!simplified || field.simplified)).map(field => {
                    const Icon = field.section === 'ai_output' && field.kind === 'list' ? listIcons[field.name as keyof typeof listIcons] : null;
                    return <div key={field.key} style={{ marginBottom: 6 }}>
                      {field.section !== 'from_path' && <div style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: Icon ? 10 : 9, fontWeight: 600, color: field.color, opacity: Icon ? 1 : 0.7, marginBottom: Icon ? 4 : 2 }}>
                        {Icon && <Icon size={11} />}
                        <span>{field.name === 'name' && simplified ? `character - ${t('jsonTag.fieldCharacter')}` : t(`jsonTag.${Icon ? field.name : field.name + 'Label'}`)}</span>
                        {Icon && <span style={{ fontSize: 9, padding: '0 5px', borderRadius: 6, background: `${field.color}14` }}>{field.get(cur.data).length}</span>}
                      </div>}
                      <fieldset disabled={cur.parse_failed} style={{ padding: '5px 8px', borderRadius: 'var(--radius-md)', background: `${field.color}${Icon ? '14' : '0f'}`, border: `1px solid ${field.color}${Icon ? '40' : '26'}`, margin: 0, minWidth: 0 }}>
                        <TagChipList key={cur.path + field.key} values={field.get(cur.data)} translations={translations} color={field.color} fieldDrag={tagDrag.bindField(field.key)}
                          onChange={values => updateData(data => field.set(data, values))} />
                      </fieldset>
                    </div>;
                  })}
                </div>
              ))}
              {/* nl — 自然语言描述 */}
              <div>
                <div style={{display:'flex',alignItems:'center',gap:5,marginBottom:6}}>
                  <Eye style={{width:11,height:11,color:'#94a3b8'}} />
                  <span style={{fontSize:10,fontWeight:700,color:'#94a3b8'}}>{t('jsonTag.nlSection')}</span>
                </div>
                <textarea className="form-input" value={cur.data.ai_output.nl||''} disabled={cur.parse_failed} onChange={e=>{ const nl = e.target.value; updateData(d=>d.ai_output.nl === nl ? d : ({...d,ai_output:{...d.ai_output,nl}})); }} style={{fontSize:11,minHeight:56,resize:'vertical',lineHeight:1.6,borderRadius:8,padding:'6px 10px'}} />
              </div>
            </>)}
          </div>

        </div>
      </div>

      {/* resize handle 2 */}
      <div onMouseDown={e=>handleColResize('col3',e)} style={{width:6,cursor:'col-resize',display:'flex',alignItems:'center',justifyContent:'center',flexShrink:0}}>
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
              {col3Mode==='stats'&&<>
                <button className="btn btn-ghost btn-sm" onClick={handleTranslate} disabled={images.length===0||localStorage.getItem('translate_enabled')!=='true'||translating} title={t('jsonTag.translateTags')} style={{width:22,height:22,padding:0,display:'flex',alignItems:'center',justifyContent:'center',color:Object.keys(translations).length>0?'#60a5fa':undefined}}>
                  {translating?<Loader2 style={{width:12,height:12,animation:'spin 1s linear infinite'}} />:<Languages style={{width:12,height:12}} />}
                </button>
                <button className="btn btn-ghost btn-sm" style={{fontSize:9,height:22,padding:'0 6px'}} onClick={()=>setTagListMode(m=>m==='all'?'common':'all')} disabled={images.length===0}>
                  {tagListMode==='all'?t('jsonTag.commonLabel'):t('jsonTag.allLabel')}
                </button>
              </>}
              <button className="btn btn-ghost btn-sm" onClick={()=>setCol3Mode(m=>m==='json'?'stats':'json')} title={col3Mode==='json'?t('jsonTag.allTags'):t('jsonTag.tagContent')} style={{width:22,height:22,padding:0,display:'flex',alignItems:'center',justifyContent:'center',color:col3Mode==='stats'?'#60a5fa':undefined}}>
                {col3Mode==='json'?<BarChart3 style={{width:12,height:12}} />:<Code style={{width:12,height:12}} />}
              </button>
            </div>
          </div>

          {col3Mode==='stats'?(<>
            <TagStatsPanel stats={stats} translations={translations} currentTags={currentTags} total={images.length}
              filteredCount={filtered.length} filterActive={tagFilterActive} onClearFilter={() => setTagFilterActive(false)} progress={translateProgress} />
          </>):(
            /* JSON Preview */
            <div style={{flex:1,overflowY:'auto',padding:'10px 12px',fontSize:11}}>
              {!cur?<span style={{color:'var(--color-text-tertiary)',fontStyle:'italic'}}>{t('jsonTag.selectToView')}</span>:(()=>{
                const d=cur.data;
                const json = jsonTagPreview(d, simplified);
                return json?(
                  <pre style={{margin:0,whiteSpace:'pre-wrap',wordBreak:'break-all',fontFamily:'"SF Mono","Fira Code","Cascadia Code",Menlo,Consolas,monospace',fontSize:10,lineHeight:1.7,color:'var(--color-text-primary)'}}>{JSON.stringify(json,null,2)}</pre>
                ):(<span style={{color:'var(--color-text-tertiary)',fontStyle:'italic'}}>{t('jsonTag.noTagDataView')}</span>);
              })()}
            </div>
          )}
        </div>

        {/* Sidebar action buttons */}
        {col3Mode==='stats'&&<div style={{display:'flex',flexDirection:'column',gap:2,padding:'8px 4px',borderLeft:'1px solid var(--color-border)',alignItems:'center'}}>
          {[
            {icon:<Filter style={{width:14,height:14}} />,tip:t('jsonTag.filterByTag'),onClick:()=>setTagFilterActive(v=>!v),disabled:images.length===0,color:tagFilterActive?'#7c5cfc':undefined},
            {icon:<ListPlus style={{width:14,height:14}} />,tip:t('jsonTag.batchAdd'),onClick:()=>{setBatchField(simplified ? 'ai_output.tags' : 'fixed.quality');setBatchTags('');setBatchPosition('prepend');setBatchScope('all');setShowBatchAddModal(true);},disabled:images.length===0},
            {icon:<ListX style={{width:14,height:14}} />,tip:t('jsonTag.batchDelete'),onClick:()=>{setBatchField('all');setBatchTags('');setBatchScope('all');setShowBatchDeleteModal(true);},disabled:images.length===0},
            {icon:<CopyX style={{width:14,height:14}} />,tip:t('jsonTag.dedupeTags'),onClick:handleDeduplicateTags,disabled:images.length===0,color:'#f59e0b'},
            {icon:<Trash2 style={{width:14,height:14}} />,tip:t('jsonTag.deleteSelected'),onClick:()=>{setSelDeleteScope('all');setShowSelDeleteModal(true);},disabled:selectedTags.size===0,color:selectedTags.size>0?'#f87171':undefined},
          ].map((item,i)=>(
            <button key={i} className="btn btn-ghost" title={item.tip} disabled={item.disabled}
              onClick={item.onClick}
              style={{width:30,height:30,padding:0,display:'flex',alignItems:'center',justifyContent:'center',borderRadius:6,color:item.color}}>
              {item.icon}
            </button>
          ))}
        </div>}
      </div>

      {showLargePreview&&cur&&<ImageLightbox src={imgSrc} filename={cur.filename} onClose={()=>setShowLargePreview(false)} />}

      <Modal open={showBatchAddModal} onClose={() => setShowBatchAddModal(false)} title={t('jsonTag.batchAddTitle')} maxWidth={440} className="tag-batch-modal" headerIcon={<ListPlus size={16} color="#4ade80" />} bodyStyle={{ padding: '16px 20px' }}>
        <fieldset className="tag-batch-fieldset"><legend>{t('jsonTag.targetField')}</legend>
          <div className="tag-batch-options">{visibleBatchFieldOptions.map(option => <label key={option.value} className="tag-option"><input type="radio" name="json-batch-add-field" checked={batchField === option.value} onChange={() => setBatchField(option.value)} />{option.label}</label>)}</div>
        </fieldset>
        <fieldset className="tag-batch-fieldset"><legend>{t('jsonTag.position')}</legend>
          <div className="tag-batch-options" style={{ '--option-color': '#4ade80' } as React.CSSProperties}>{(['prepend', 'append'] as const).map(value => <label key={value} className="tag-option"><input type="radio" name="json-batch-add-position" checked={batchPosition === value} onChange={() => setBatchPosition(value)} />{t('tagManager.' + value)}</label>)}</div>
        </fieldset>
        <ScopeToggle value={batchScope} onChange={setBatchScope} hasCurrent={!!cur && !cur.parse_failed} appearance="chips" />
        <label className="form-label" style={{ fontSize: 11 }}>{t('jsonTag.batchTagsPlaceholder')}</label>
        <TagAutocomplete multi value={batchTags} onChange={setBatchTags} onSelect={handleBatchAdd} autoFocus placeholder="tag1, tag2, tag3" />
        <div className="tag-batch-actions">
          <button className="btn btn-ghost" onClick={() => setShowBatchAddModal(false)}>{t('jsonTag.cancel')}</button>
          <button className="btn btn-primary" onClick={handleBatchAdd} disabled={!batchTags.trim()}><ListPlus size={12} />{t('jsonTag.batchApplyAdd')}</button>
        </div>
      </Modal>
      <Modal open={showBatchDeleteModal} onClose={() => setShowBatchDeleteModal(false)} title={t('jsonTag.batchDeleteTitle')} variant="warning" maxWidth={440} className="tag-batch-modal" headerIcon={<ListX size={16} color="#f87171" />} bodyStyle={{ padding: '16px 20px' }}>
        <fieldset className="tag-batch-fieldset"><legend>{t('jsonTag.targetField')}</legend>
          <div className="tag-batch-options" style={{ '--option-color': '#f87171' } as React.CSSProperties}>{[{ value: 'all', label: t('jsonTag.allFields') }, ...visibleBatchFieldOptions].map(option => <label key={option.value} className="tag-option"><input type="radio" name="json-batch-delete-field" checked={batchField === option.value} onChange={() => setBatchField(option.value)} />{option.label}</label>)}</div>
        </fieldset>
        <ScopeToggle value={batchScope} onChange={setBatchScope} hasCurrent={!!cur && !cur.parse_failed} appearance="chips" />
        <label className="form-label" style={{ fontSize: 11 }}>{t('jsonTag.batchTagsPlaceholder')}</label>
        <TagAutocomplete multi value={batchTags} onChange={setBatchTags} onSelect={handleBatchDelete} autoFocus placeholder="tag1, tag2, tag3" />
        <div className="tag-batch-actions">
          <button className="btn btn-ghost" onClick={() => setShowBatchDeleteModal(false)}>{t('jsonTag.cancel')}</button>
          <button className="btn btn-danger" onClick={handleBatchDelete} disabled={!batchTags.trim()}><ListX size={12} />{t('jsonTag.batchApplyDelete')}</button>
        </div>
      </Modal>
      <Modal open={showSelDeleteModal} onClose={() => setShowSelDeleteModal(false)} title={t('jsonTag.deleteSelectedTitle')} variant="warning" maxWidth={440} className="tag-batch-modal" headerIcon={<Trash2 size={16} color="#f87171" />} bodyStyle={{ padding: '16px 20px' }}>
        <div>{t('jsonTag.deleteTagsHint', { n: selectedTags.size })}</div>
        <div style={{ maxHeight: 96, overflowY: 'auto', display: 'flex', flexWrap: 'wrap', gap: 4, padding: 8, background: 'rgba(248,113,113,0.04)', border: '1px solid rgba(248,113,113,0.15)', borderRadius: 8, margin: '8px 0' }}>{[...selectedTags].map(tag => <span key={tag} style={{ fontSize: 10, color: '#f87171', background: 'rgba(248,113,113,0.1)', borderRadius: 12, padding: '2px 8px', overflowWrap: 'anywhere' }}>{tag}</span>)}</div>
        <ScopeToggle value={selDeleteScope} onChange={setSelDeleteScope} hasCurrent={!!cur && !cur.parse_failed} appearance="chips" />
        <div className="tag-batch-actions">
          <button className="btn btn-ghost" onClick={() => setShowSelDeleteModal(false)}>{t('jsonTag.cancel')}</button>
          <button className="btn btn-danger" onClick={handleSidebarBatchDelete} disabled={selDeleteScope === 'current' && (!cur || cur.parse_failed)}><Trash2 size={12} />{t(selDeleteScope === 'all' ? 'jsonTag.deleteFromAll' : 'jsonTag.deleteFromCurrentOne')}</button>
        </div>
      </Modal>
      <AlertModal open={!!alertMsg} onClose={() => setAlertMsg('')} message={alertMsg} />
    </div>
  );
});

export default JsonTagTab;
