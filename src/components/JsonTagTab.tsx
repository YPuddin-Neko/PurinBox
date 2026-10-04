import { useState, useEffect, useMemo, useCallback, forwardRef, useImperativeHandle, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as dialogOpen } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import {
  Sparkles, Eye, ListPlus, ListX, BarChart3, Filter, Code, Trash2, CopyX,
  Lock, User, Layers, Shirt, Tags, TreePine,
} from 'lucide-react';
import { JSON_FIELDS, collectAllTags, jsonTagPreview, moveJsonTag, type JsonTagData } from '../utils/jsonTagFields';
import { dedupeTags, getJsonStatColor, splitTagInput, tagKey } from '../utils/tagText';
import { saveFailureAlert, saveFailures, settleSaved } from '../utils/tagSave';
import { ensureAssetScope } from '../utils/assetScope';
import type { SaveAllResult } from '../api/commandOptions';
import { useTagStats } from '../hooks/useTagStats';
import { useTagTranslation } from '../hooks/useTagTranslation';
import { useDragResize } from '../hooks/useDragResize';
import { useTagFieldDrag } from '../hooks/useTagFieldDrag';
import TagChipList from './TagChipList';
import ImageGridColumn from './ImageGridColumn';
import TagStatsPanel from './TagStatsPanel';
import ScopeToggle from './ScopeToggle';
import TagAutocomplete from './TagAutocomplete';
import { ImagePreviewPane, OptionChips, PanelSaveButton, ResizeHandle, TagPanelHeader, TagStatsActions, ToolSidebar } from './TagEditorLayout';
import { Modal, AlertModal } from './Modal';

/** parse_failed：JSON 存在但解析失败，data 只是空默认值。保存时会跳过这类条目，
 *  单图编辑和批量增删也跳过它们，免得被标成未保存却永远存不进文件。 */
interface JsonImageItem { path: string; filename: string; data: JsonTagData; has_json: boolean; parse_failed?: boolean; dirty?: boolean; }
interface JsonDataset { folder: string; images: JsonImageItem[]; detected_format: string; }

const sections = [
  { key: 'fixed', label: 'fixedSection', icon: Lock, color: '#f59e0b' },
  { key: 'character', label: 'characterSection', icon: User, color: '#f472b6' },
  { key: 'from_path', label: 'fromPathSection', icon: Layers, color: '#22d3ee' },
  { key: 'ai_output', label: 'aiOutputSection', icon: Sparkles, color: '#818cf8' },
] as const;
const listIcons = { appearance: Shirt, tags: Tags, environment: TreePine };
const countBadge = (color: string): CSSProperties => ({ fontSize: 9, padding: '0 5px', borderRadius: 6, background: `${color}14` });
const RED = '#f87171';

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
  const [images, setImages] = useState<JsonImageItem[]>([]);
  const [selectedIdx, setSelectedIdx] = useState(-1);
  const [folderPath, setFolderPath] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savingSingle, setSavingSingle] = useState(false);
  const [searchText, setSearchText] = useState('');
  const [filterMode, setFilterMode] = useState<'all' | 'tagged' | 'untagged'>('all');

  const [simplified, setSimplified] = useState(() => localStorage.getItem('json_tag_simplified') === 'true');
  const translation = useTagTranslation();
  const { translations, translateProgress } = translation;
  const [alertMsg, setAlertMsg] = useState('');
  const col1 = useDragResize({ initial: 220, min: 160, max: 500, storageKey: 'json_col1w' });
  const col3 = useDragResize({ initial: 220, min: 160, max: 500, direction: -1, storageKey: 'json_col3w' });
  const preview = useDragResize({ initial: 220, min: 100, max: 500, axis: 'y', storageKey: 'json_previewh' });
  const tagLists = useMemo(() => images.map(image => collectAllTags(image.data)), [images]);
  const stats = useTagStats(tagLists, folderPath, translations);
  const { selectedTags, setSelectedTags, tagListMode, filteredStats } = stats;

  const [showBatchAddModal, setShowBatchAddModal] = useState(false);
  const [showBatchDeleteModal, setShowBatchDeleteModal] = useState(false);
  const [batchField, setBatchField] = useState('fixed.quality');
  const [batchTags, setBatchTags] = useState('');
  const [batchPosition, setBatchPosition] = useState<'prepend' | 'append'>('prepend');
  /** 批量添加/删除的应用范围：当前图片 or 全部图片 */
  const [batchScope, setBatchScope] = useState<'current' | 'all'>('all');
  const [showSelDeleteModal, setShowSelDeleteModal] = useState(false);
  const [selDeleteScope, setSelDeleteScope] = useState<'current' | 'all'>('all');

  const visibleBatchFieldOptions = JSON_FIELDS.filter(field => !simplified || field.simplified).map(field => ({
    value: field.key, label: `${field.name} - ${t(field.labelKey)}`,
  }));

  const [col3Mode, setCol3Mode] = useState<'json' | 'stats'>('stats');
  const [tagFilterActive, setTagFilterActive] = useState(false);

  const handleLoadFolder = useCallback(async () => {
    const sel = await dialogOpen({ directory: true, multiple: false, title: t('tagEditor.selectFolder') });
    if (!sel) return;
    setLoading(true);
    try {
      await ensureAssetScope(sel as string);
      const r = await invoke<JsonDataset>('load_json_dataset', { folder: sel as string, recursive });
      setImages(r.images.map(img => ({ ...img, dirty: false })));
      setSelectedIdx(r.images.length > 0 ? 0 : -1);
      setFolderPath(sel as string); setSearchText(''); setFilterMode('all');
      if (r.detected_format === 'simplified') { setSimplified(true); localStorage.setItem('json_tag_simplified', 'true'); }
      else if (r.detected_format === 'full') { setSimplified(false); localStorage.setItem('json_tag_simplified', 'false'); }
    } catch (e) { console.error(e); } finally { setLoading(false); }
  }, [recursive, t]);
  const handleRefresh = useCallback(async () => {
    if (!folderPath) return;
    setLoading(true);
    try {
      await ensureAssetScope(folderPath);
      const r = await invoke<JsonDataset>('load_json_dataset', { folder: folderPath, recursive });
      setImages(r.images.map(img => ({ ...img, dirty: false })));
    } catch (e) { console.error(e); } finally { setLoading(false); }
  }, [folderPath, recursive]);

  const cur = selectedIdx >= 0 && selectedIdx < images.length ? images[selectedIdx] : null;
  const dirtyCount = images.filter(i => i.dirty).length;

  const handleSaveSingle = useCallback(async () => {
    if (!cur || !cur.dirty || cur.parse_failed) return;
    const sent = new Map([[cur.path, cur.data]]);
    setSavingSingle(true);
    try {
      await invoke('save_single_json_file', { imagePath: cur.path, data: cur.data, simplified });
      setImages(previous => settleSaved(previous, sent, [], image => image.data));
    } catch (e) {
      setAlertMsg(`${t('tagEditor.saveFailed')}: ${String(e)}`);
    } finally { setSavingSingle(false); }
  }, [cur, simplified, t]);
  const handleSaveAll = useCallback(async () => {
    const dirty = images.filter(i => i.dirty && !i.parse_failed);
    if (!dirty.length) return;
    const sent = new Map(dirty.map(i => [i.path, i.data]));
    setSaving(true);
    try {
      const failures = saveFailures(await invoke<SaveAllResult>('save_all_json_files', { items: dirty.map(i => ({ path: i.path, data: i.data })), simplified }));
      setImages(previous => settleSaved(previous, sent, failures, image => image.data));
      if (failures.length) setAlertMsg(saveFailureAlert(failures, t));
    } catch (e) {
      setAlertMsg(`${t('tagEditor.saveFailed')}: ${String(e)}`);
    } finally { setSaving(false); }
  }, [images, simplified, t]);

  useImperativeHandle(ref, () => ({
    loadFolder: handleLoadFolder,
    saveAll: handleSaveAll,
  }), [handleLoadFolder, handleSaveAll]);

  useEffect(() => { onDirtyChange?.(dirtyCount); }, [dirtyCount, onDirtyChange]);
  useEffect(() => { onLoadingChange?.(loading); }, [loading, onLoadingChange]);
  useEffect(() => { onSavingChange?.(saving); }, [saving, onSavingChange]);
  const goPrev = () => { if (selectedIdx > 0) setSelectedIdx(selectedIdx - 1); };
  const goNext = () => { if (selectedIdx < images.length - 1) setSelectedIdx(selectedIdx + 1); };

  const filtered = useMemo(() => {
    let list = images.map((img, i) => ({ ...img, _i: i }));
    if (searchText) {
      const q = searchText.toLowerCase();
      list = list.filter(img => {
        if (img.filename.toLowerCase().includes(q)) return true;
        // 搜索比筛选多带一个 nl（自然语言描述里也能搜到词）
        const all = [...collectAllTags(img.data), img.data.ai_output?.nl || ''];
        return all.some(tag => tag.toLowerCase().includes(q));
      });
    }
    if (filterMode === 'tagged') list = list.filter(img => img.has_json);
    if (filterMode === 'untagged') list = list.filter(img => !img.has_json);
    if (tagFilterActive && selectedTags.size > 0) {
      list = list.filter(img => {
        const allTags = collectAllTags(img.data);
        return [...selectedTags].every(tag => allTags.includes(tag));
      });
    }
    return list;
  }, [images, searchText, filterMode, tagFilterActive, selectedTags]);

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
  /** keyOf 决定哪些写法算同一个标签：统计列表里选中的按原文（忽略大小写），手输的按 tagKey */
  const removeTags = (tags: Iterable<string>, scope: 'current' | 'all', field: string, keyOf: (tag: string) => string) => {
    if (scope === 'current' && !cur) return;
    const keys = new Set([...tags].map(keyOf));
    setImages(previous => previous.map((image, index) => {
      if (image.parse_failed || scope === 'current' && index !== selectedIdx) return image;
      let data = image.data;
      for (const definition of JSON_FIELDS) {
        if (field !== 'all' && definition.key !== field) continue;
        const values = definition.get(data), next = values.filter(tag => !keys.has(keyOf(tag)));
        if (next.length !== values.length) data = definition.set(data, next);
      }
      return data === image.data ? image : { ...image, data, dirty: true };
    }));
  };
  const handleSidebarBatchDelete = () => {
    removeTags(selectedTags, selDeleteScope, 'all', tag => tag.toLowerCase());
    setSelectedTags(new Set()); setShowSelDeleteModal(false);
  };
  // 手输的标签与文件里的比较两侧都按 tagKey：存的 long_hair 和输入的 Long Hair 是同一个标签
  const handleBatchAdd = () => {
    const tags = splitTagInput(batchTags), field = JSON_FIELDS.find(item => item.key === batchField);
    if (!field || !tags.length || batchScope === 'current' && !cur) return;
    setImages(previous => previous.map((image, index) => {
      if (image.parse_failed || batchScope === 'current' && index !== selectedIdx) return image;
      const values = field.get(image.data), keys = new Set(values.map(tagKey));
      const incoming = tags.filter(tag => !keys.has(tagKey(tag)));
      if (!incoming.length) return image;
      const data = field.set(image.data, batchPosition === 'prepend' ? [...incoming, ...values] : [...values, ...incoming]);
      return { ...image, data, dirty: true };
    }));
    setBatchTags(''); setShowBatchAddModal(false);
  };
  const handleBatchDelete = () => {
    removeTags(splitTagInput(batchTags), batchScope, batchField, tagKey);
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
  const parseFailedBadge = cur?.parse_failed && (
    <span style={{ fontSize: 10, color: RED, fontWeight: 600, padding: '1px 6px', borderRadius: 4,
      background: 'rgba(248,113,113,0.12)', border: '1px solid rgba(248,113,113,0.3)' }}>
      {t('jsonTag.parseFailed')}
    </span>
  );
  const formatBadge = images.length > 0 && (
    <span style={{ fontSize: 9, padding: '2px 8px', borderRadius: 10, fontWeight: 700,
      background: simplified ? 'rgba(34,197,94,0.15)' : 'rgba(99,102,241,0.15)', color: simplified ? '#22c55e' : '#818cf8',
      border: `1px solid ${simplified ? 'rgba(34,197,94,0.25)' : 'rgba(99,102,241,0.25)'}` }}>
      {simplified ? t('jsonTag.simplified') : t('jsonTag.full')}
    </span>
  );
  const editable = !!cur && !cur.parse_failed;

  return (
    <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
      <ImageGridColumn key={folderPath} width={col1.size} items={filtered} total={images.length} tagged={taggedCount}
        search={searchText} onSearch={setSearchText} filter={filterMode} onFilter={setFilterMode}
        selected={selectedIdx} onSelect={setSelectedIdx} onRefresh={folderPath ? handleRefresh : undefined} loading={loading}
        badge={image => image.has_json ? String(collectAllTags(image.data).length) : null} />

      <ResizeHandle axis="x" onMouseDown={col1.onMouseDown} />

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>
        <ImagePreviewPane path={cur?.path} filename={cur?.filename} index={selectedIdx} total={images.length}
          onPrev={goPrev} onNext={goNext} badge={parseFailedBadge} style={{ height: preview.size, flexShrink: 0 }} />

        <ResizeHandle axis="y" onMouseDown={preview.onMouseDown} />

        <div className="tag-card" style={{ flex: 1, minHeight: 0 }}>
          <TagPanelHeader icon={Sparkles} color="#22d3ee" title={t('jsonTag.jsonEditor')} extra={formatBadge}>
            <PanelSaveButton saving={savingSingle} disabled={!cur || !cur.dirty} onClick={handleSaveSingle} />
          </TagPanelHeader>

          <div {...tagDrag.editorProps} style={{ flex: 1, overflowY: 'auto', padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 14 }}>
            {!cur ? <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', fontStyle: 'italic' }}>{t('tagEditor.selectToEdit')}</span> : (<>
              {sections.filter(section => !simplified || section.key !== 'from_path').map(section => (
                <div key={section.key} style={{ marginBottom: 8 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: 10, fontWeight: 700, marginBottom: 6, color: section.color }}>
                    <section.icon size={11} /><span>{t(`jsonTag.${section.label}`)}</span>
                    {section.key === 'from_path' && <span style={countBadge(section.color)}>{cur.data.from_path.appearance.length}</span>}
                  </div>
                  {JSON_FIELDS.filter(field => field.section === section.key && (!simplified || field.simplified)).map(field => {
                    const Icon = field.section === 'ai_output' && field.kind === 'list' ? listIcons[field.name as keyof typeof listIcons] : null;
                    return (
                      <div key={field.key} style={{ marginBottom: 6 }}>
                        {field.section !== 'from_path' && (
                          <div style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: Icon ? 10 : 9, fontWeight: 600, color: field.color,
                            opacity: Icon ? 1 : 0.7, marginBottom: Icon ? 4 : 2 }}>
                            {Icon && <Icon size={11} />}
                            <span>
                              {field.name === 'name' && simplified
                                ? `character - ${t('jsonTag.fieldCharacter')}`
                                : t(`jsonTag.${Icon ? field.name : field.name + 'Label'}`)}
                            </span>
                            {Icon && <span style={countBadge(field.color)}>{field.get(cur.data).length}</span>}
                          </div>
                        )}
                        <fieldset disabled={cur.parse_failed} style={{ padding: '5px 8px', borderRadius: 'var(--radius-md)', margin: 0, minWidth: 0,
                          background: `${field.color}${Icon ? '14' : '0f'}`, border: `1px solid ${field.color}${Icon ? '40' : '26'}` }}>
                          <TagChipList key={cur.path + field.key} values={field.get(cur.data)} translations={translations} color={field.color}
                            fieldDrag={tagDrag.bindField(field.key)} onChange={values => updateData(data => field.set(data, values))} />
                        </fieldset>
                      </div>
                    );
                  })}
                </div>
              ))}
              <div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 5, marginBottom: 6 }}>
                  <Eye style={{ width: 11, height: 11, color: '#94a3b8' }} />
                  <span style={{ fontSize: 10, fontWeight: 700, color: '#94a3b8' }}>{t('jsonTag.nlSection')}</span>
                </div>
                <textarea className="form-input" value={cur.data.ai_output.nl || ''} disabled={cur.parse_failed}
                  onChange={e => { const nl = e.target.value; updateData(d => d.ai_output.nl === nl ? d : ({ ...d, ai_output: { ...d.ai_output, nl } })); }}
                  style={{ fontSize: 11, minHeight: 56, resize: 'vertical', lineHeight: 1.6, borderRadius: 8, padding: '6px 10px' }} />
              </div>
            </>)}
          </div>
        </div>
      </div>

      <ResizeHandle axis="x" onMouseDown={col3.onMouseDown} />

      <div className="tag-card" style={{ width: col3.size, minWidth: 160, maxWidth: 500, flexShrink: 0, flexDirection: 'row' }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          <TagPanelHeader icon={col3Mode === 'stats' ? BarChart3 : Code} color="#60a5fa"
            title={col3Mode === 'stats' ? (tagListMode === 'common' ? t('tagEditor.commonTags') : t('tagEditor.allTags')) : t('jsonTag.tagContent')}
            extra={col3Mode === 'stats' && <span className="tag-panel-count" style={{ background: 'rgba(96,165,250,0.1)', color: '#60a5fa' }}>{filteredStats.length}</span>}>
            <div className="tag-header-actions">
              {col3Mode === 'stats' && (
                <TagStatsActions stats={stats} translation={translation} disabled={images.length === 0} onError={setAlertMsg} spinner />
              )}
              <button className="btn btn-ghost btn-sm tag-icon-btn" onClick={() => setCol3Mode(m => m === 'json' ? 'stats' : 'json')}
                title={col3Mode === 'json' ? t('tagEditor.allTags') : t('jsonTag.tagContent')}
                style={{ color: col3Mode === 'stats' ? '#60a5fa' : undefined }}>
                {col3Mode === 'json' ? <BarChart3 style={{ width: 12, height: 12 }} /> : <Code style={{ width: 12, height: 12 }} />}
              </button>
            </div>
          </TagPanelHeader>

          {col3Mode === 'stats' ? (
            <TagStatsPanel stats={stats} translations={translations} currentTags={currentTags} total={images.length} colorOf={getJsonStatColor}
              filteredCount={filtered.length} filterActive={tagFilterActive} onClearFilter={() => setTagFilterActive(false)} progress={translateProgress} />
          ) : (
            <div style={{ flex: 1, overflowY: 'auto', padding: '10px 12px', fontSize: 11 }}>
              {!cur ? <span style={{ color: 'var(--color-text-tertiary)', fontStyle: 'italic' }}>{t('jsonTag.selectToView')}</span> : (
                <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-all', fontFamily: '"SF Mono","Fira Code","Cascadia Code",Menlo,Consolas,monospace',
                  fontSize: 10, lineHeight: 1.7, color: 'var(--color-text-primary)' }}>
                  {JSON.stringify(jsonTagPreview(cur.data, simplified), null, 2)}
                </pre>
              )}
            </div>
          )}
        </div>

        {col3Mode === 'stats' && <ToolSidebar items={[
          { icon: Filter, label: t('tagEditor.filterByTag'), onClick: () => setTagFilterActive(v => !v), disabled: images.length === 0, color: tagFilterActive ? '#7c5cfc' : undefined },
          { icon: ListPlus, label: t('tagEditor.batchAdd'), disabled: images.length === 0, onClick: () => {
            setBatchField(simplified ? 'ai_output.tags' : 'fixed.quality'); setBatchTags(''); setBatchPosition('prepend'); setBatchScope('all'); setShowBatchAddModal(true);
          } },
          { icon: ListX, label: t('jsonTag.batchDelete'), disabled: images.length === 0, onClick: () => {
            setBatchField('all'); setBatchTags(''); setBatchScope('all'); setShowBatchDeleteModal(true);
          } },
          { icon: CopyX, label: t('tagEditor.dedupeTags'), onClick: handleDeduplicateTags, disabled: images.length === 0, color: '#f59e0b' },
          { icon: Trash2, label: t('tagEditor.deleteSelected'), onClick: () => { setSelDeleteScope('all'); setShowSelDeleteModal(true); },
            disabled: selectedTags.size === 0, color: selectedTags.size > 0 ? RED : undefined },
        ]} />}
      </div>

      <Modal open={showBatchAddModal} onClose={() => setShowBatchAddModal(false)} title={t('tagEditor.batchAddTitle')} width={440} sectioned
        headerIcon={<ListPlus size={16} color="#4ade80" />}>
        <OptionChips legend={t('jsonTag.targetField')} options={visibleBatchFieldOptions} value={batchField} onChange={setBatchField} />
        <OptionChips legend={t('tagEditor.position')} value={batchPosition} onChange={setBatchPosition} color="#4ade80"
          options={[{ value: 'prepend', label: t('tagEditor.prepend') }, { value: 'append', label: t('tagEditor.append') }]} />
        <ScopeToggle value={batchScope} onChange={setBatchScope} hasCurrent={editable} />
        <label className="form-label" style={{ fontSize: 11 }}>{t('tagEditor.tagContent')}</label>
        <TagAutocomplete multi value={batchTags} onChange={setBatchTags} onSelect={handleBatchAdd} autoFocus placeholder="tag1, tag2, tag3" />
        <div className="tag-batch-actions">
          <button className="btn btn-ghost" onClick={() => setShowBatchAddModal(false)}>{t('common.cancel')}</button>
          <button className="btn btn-primary" onClick={handleBatchAdd} disabled={!batchTags.trim()}>
            <ListPlus style={{ width: 12, height: 12 }} />{t('jsonTag.batchApplyAdd')}
          </button>
        </div>
      </Modal>
      <Modal open={showBatchDeleteModal} onClose={() => setShowBatchDeleteModal(false)} title={t('tagEditor.batchDeleteTitle')} variant="warning" width={440} sectioned
        headerIcon={<ListX size={16} color={RED} />}>
        <OptionChips legend={t('jsonTag.targetField')} value={batchField} onChange={setBatchField} color={RED}
          options={[{ value: 'all', label: t('jsonTag.allFields') }, ...visibleBatchFieldOptions]} />
        <ScopeToggle value={batchScope} onChange={setBatchScope} hasCurrent={editable} color={RED} />
        <label className="form-label" style={{ fontSize: 11 }}>{t('tagEditor.tagContent')}</label>
        <TagAutocomplete multi value={batchTags} onChange={setBatchTags} onSelect={handleBatchDelete} autoFocus placeholder="tag1, tag2, tag3" />
        <div className="tag-batch-actions">
          <button className="btn btn-ghost" onClick={() => setShowBatchDeleteModal(false)}>{t('common.cancel')}</button>
          <button className="btn btn-danger" onClick={handleBatchDelete} disabled={!batchTags.trim()}>
            <ListX style={{ width: 12, height: 12 }} />{t('jsonTag.batchApplyDelete')}
          </button>
        </div>
      </Modal>
      <Modal open={showSelDeleteModal} onClose={() => setShowSelDeleteModal(false)} title={t('jsonTag.deleteSelectedTitle')} variant="warning" width={440} sectioned
        headerIcon={<Trash2 size={16} color={RED} />}>
        <div>{t('tagEditor.deleteTagsHint', { n: selectedTags.size })}</div>
        <div style={{ maxHeight: 96, overflowY: 'auto', display: 'flex', flexWrap: 'wrap', gap: 4, padding: 8, margin: '8px 0',
          background: 'rgba(248,113,113,0.04)', border: '1px solid rgba(248,113,113,0.15)', borderRadius: 8 }}>
          {[...selectedTags].map(tag => (
            <span key={tag} style={{ fontSize: 10, color: RED, background: 'rgba(248,113,113,0.1)', borderRadius: 12, padding: '2px 8px', overflowWrap: 'anywhere' }}>{tag}</span>
          ))}
        </div>
        <ScopeToggle value={selDeleteScope} onChange={setSelDeleteScope} hasCurrent={editable} color={RED} />
        <div className="tag-batch-actions">
          <button className="btn btn-ghost" onClick={() => setShowSelDeleteModal(false)}>{t('common.cancel')}</button>
          <button className="btn btn-danger" onClick={handleSidebarBatchDelete} disabled={selDeleteScope === 'current' && !editable}>
            <Trash2 style={{ width: 12, height: 12 }} />{t(selDeleteScope === 'all' ? 'tagEditor.deleteFromAll' : 'tagEditor.deleteFromCurrentOne')}
          </button>
        </div>
      </Modal>
      <AlertModal open={!!alertMsg} onClose={() => setAlertMsg('')} message={alertMsg} />
    </div>
  );
});

export default JsonTagTab;
