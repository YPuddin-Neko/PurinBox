import { useState, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import {
  Tags, FolderOpen, Save, X, Trash2, BarChart3, Loader2, Replace, Filter, ListPlus, PlusCircle, MinusCircle,
  List, CopyX, Wand2,
} from 'lucide-react';
import { useTagStats } from '../hooks/useTagStats';
import { useTagTranslation } from '../hooks/useTagTranslation';
import { useDragResize, useFlexRatioResize } from '../hooks/useDragResize';
import { useTagFieldDrag } from '../hooks/useTagFieldDrag';
import {
  dedupeTags, moveTagWithin, normalizeLikeTags, renameSelected, sameTags, splitTagInput, toDanbooruEscaped,
} from '../utils/tagText';
import { saveFailureAlert, saveFailures, settleSaved } from '../utils/tagSave';
import { ensureAssetScope } from '../utils/assetScope';
import type { SaveAllResult } from '../api/commandOptions';
import TagChipList from '../components/TagChipList';
import ImageGridColumn from '../components/ImageGridColumn';
import TagStatsPanel from '../components/TagStatsPanel';
import ScopeToggle from '../components/ScopeToggle';
import { ImagePreviewPane, OptionChips, PanelSaveButton, ResizeHandle, TagPanelHeader, TagStatsActions, ToolSidebar } from '../components/TagEditorLayout';
import SegmentedTabs from '../components/ui/SegmentedTabs';
import PageHeader from '../components/ui/PageHeader';
import { Modal, AlertModal } from '../components/Modal';
import NaturalLangTab from '../components/NaturalLangTab';
import JsonTagTab, { type JsonTagTabHandle } from '../components/JsonTagTab';
import RecursiveScanToggle from '../components/RecursiveScanToggle';
import Checkbox from '../components/Checkbox';

type ImageItem = { filename: string; path: string; tags: string[]; dirty: boolean; };
type CaptionItem = { filename: string; path: string; caption: string; dirty: boolean; };
type TagDataset = { folder: string; images: { path: string; filename: string; tags: string[] }[] };
type CaptionDataset = { folder: string; images: { path: string; filename: string; caption: string }[] };

const RED = '#f87171';
const modalLabel = { fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 4, display: 'block' } as const;
const modalButton = { height: 30, fontSize: 11 } as const;
const modalActions = { display: 'flex', gap: 8, justifyContent: 'flex-end' } as const;
const topButton = { gap: 6, height: 34, fontSize: 12 } as const;
const spinner14 = { width: 14, height: 14, animation: 'spin 1s linear infinite' } as const;

export default function TagManagerPage() {
  const { t } = useTranslation();
  const [mode, setMode] = useState<'danbooru' | 'natural' | 'json'>('danbooru');
  const [images, setImages] = useState<ImageItem[]>([]);
  const [nlImages, setNlImages] = useState<CaptionItem[]>([]);

  const [selectedIdx, setSelectedIdx] = useState(-1);
  const [searchText, setSearchText] = useState('');
  const [filterMode, setFilterMode] = useState<'all' | 'tagged' | 'untagged'>('all');

  const [savingSingle, setSavingSingle] = useState(false);
  const [alertMsg, setAlertMsg] = useState('');

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
  const previewRow = useFlexRatioResize({ initial: 3, min: 0.5, max: 6 });

  // ── 右栏状态 ──
  const [showAddModal, setShowAddModal] = useState(false);
  const [addTagInput, setAddTagInput] = useState('');
  const [addPosition, setAddPosition] = useState<'start' | 'end'>('start');
  const [addOverwrite, setAddOverwrite] = useState(false);
  /** 批量添加/删除的应用范围：当前图片 or 全部图片 */
  const [addScope, setAddScope] = useState<'current' | 'all'>('all');
  const [showDeleteModal, setShowDeleteModal] = useState(false);
  const [deleteScope, setDeleteScope] = useState<'current' | 'all'>('all');
  const [showReplaceModal, setShowReplaceModal] = useState(false);
  const [replaceFrom, setReplaceFrom] = useState('');
  const [replaceTo, setReplaceTo] = useState('');
  const [replaceSearch, setReplaceSearch] = useState('');
  const [replaceDropOpen, setReplaceDropOpen] = useState(false);
  const [tagFilterActive, setTagFilterActive] = useState(false);
  const translation = useTagTranslation();
  const { translations, translateProgress } = translation;
  const tagLists = useMemo(() => images.map(image => image.tags), [images]);
  const stats = useTagStats(tagLists, folderPath, translations);
  const { selectedTags, setSelectedTags, tagListMode, filteredStats, tagStats, taggedCount, setGlobalSearch } = stats;

  // ── 加载文件夹 ──
  const handleLoadFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: t('tagEditor.selectFolder') });
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
    } catch (e: unknown) {
      console.error('加载失败:', e);
    } finally {
      setLoading(false);
    }
  };

  // ── 刷新 ──
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
    } catch (e: unknown) {
      console.error('刷新失败:', e);
    } finally {
      setLoading(false);
    }
  };

  const reportSaveFailures = (result: unknown) => {
    const failures = saveFailures(result);
    if (failures.length) setAlertMsg(saveFailureAlert(failures, t));
    return failures;
  };

  // ── 全部保存 ──
  const handleSaveAll = async () => {
    const dirty = images.filter(img => img.dirty);
    if (dirty.length === 0) return;
    const sent = new Map(dirty.map(img => [img.path, img.tags]));
    setSaving(true);
    try {
      const failures = reportSaveFailures(await invoke<SaveAllResult>('save_all_tag_files', { items: dirty.map(img => ({ path: img.path, tags: img.tags })) }));
      setImages(previous => settleSaved(previous, sent, failures, img => img.tags));
    } catch (e: unknown) {
      setAlertMsg(`${t('tagEditor.saveFailed')}: ${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const handleSaveAllCaptions = async () => {
    const dirty = nlImages.filter(img => img.dirty);
    if (dirty.length === 0) return;
    const sent = new Map(dirty.map(img => [img.path, img.caption]));
    setSaving(true);
    try {
      const failures = reportSaveFailures(await invoke<SaveAllResult>('save_all_caption_files', { items: dirty.map(img => ({ path: img.path, content: img.caption })) }));
      setNlImages(previous => settleSaved(previous, sent, failures, img => img.caption));
    } catch (e: unknown) {
      setAlertMsg(`${t('tagEditor.saveFailed')}: ${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const cur = selectedIdx >= 0 && selectedIdx < images.length ? images[selectedIdx] : null;

  // 新标签按数据集现有格式规范化：已有 danbooru 原形标签就转义，否则保持空格形式。
  // 优先看当前图片，当前图无标签时看整个数据集
  const normalizeNewTag = (raw: string, img?: ImageItem | null) =>
    normalizeLikeTags(raw, img && img.tags.length > 0 ? img.tags : images.flatMap(i => i.tags));

  // 一键转义：全部标签转成 danbooru 原形。转义后可能撞出重复
  // （"long hair" 和 "long_hair" 归一为同一个），顺手去重
  const handleEscapeAll = () => {
    setImages(p => p.map(img => {
      const next = dedupeTags(img.tags.map(toDanbooruEscaped)).tags;
      return sameTags(img.tags, next) ? img : { ...img, tags: next, dirty: true };
    }));
  };

  // ── 标签筛选图片 ──
  const filtered = useMemo(() => {
    let list = images.map((img, i) => ({ ...img, _i: i }));
    if (searchText) {
      const q = searchText.toLowerCase();
      list = list.filter(img => img.filename.toLowerCase().includes(q) || img.tags.some(tag => tag.includes(q)));
    }
    if (filterMode === 'tagged') list = list.filter(img => img.tags.length > 0);
    if (filterMode === 'untagged') list = list.filter(img => img.tags.length === 0);
    if (tagFilterActive && selectedTags.size > 0) {
      list = list.filter(img => [...selectedTags].every(tag => img.tags.includes(tag)));
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
      tags.forEach(tag => {
        const idx = newTags.indexOf(tag);
        if (idx >= 0) {
          if (addOverwrite) { newTags.splice(idx, 1); }
          else return;
        }
        toInsert.push(tag);
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
      const newTags = img.tags.filter(tag => !selectedTags.has(tag));
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
      if (!img.tags.some(tag => tag.toLowerCase() === fromKey)) return img;
      const replaced = img.tags.map(tag => tag.toLowerCase() === fromKey ? to : tag);
      // 替换后可能产生重复标签，去重（保序）
      return { ...img, tags: dedupeTags(replaced).tags, dirty: true };
    }));
    setShowReplaceModal(false); setReplaceFrom(''); setReplaceTo('');
    setSelectedTags(prev => { const n = new Set(prev); [...n].forEach(tag => { if (tag.toLowerCase() === fromKey) n.delete(tag); }); return n; });
  };

  // ── 为当前图片添加/删除选中标签 ──
  const addSelectedToCurrent = () => {
    if (!cur) return;
    setImages(p => p.map((img, i) => {
      if (i !== selectedIdx) return img;
      const newTags = [...img.tags];
      selectedTags.forEach(tag => { if (!newTags.includes(tag)) newTags.push(tag); });
      return { ...img, tags: newTags, dirty: true };
    }));
  };
  const removeSelectedFromCurrent = () => {
    if (!cur) return;
    setImages(p => p.map((img, i) => {
      if (i !== selectedIdx) return img;
      return { ...img, tags: img.tags.filter(tag => !selectedTags.has(tag)), dirty: true };
    }));
  };
  const curHasAllSelected = cur ? [...selectedTags].every(tag => cur.tags.includes(tag)) : false;

  // ── 切换图片 ──
  const goPrev = useCallback(() => { setSelectedIdx(i => Math.max(0, i - 1)); }, []);
  const goNext = useCallback(() => { setSelectedIdx(i => Math.min(images.length - 1, i + 1)); }, [images.length]);

  const setCurrentTags = (path: string, tags: string[]) =>
    setImages(previous => previous.map(image => image.path === path && !sameTags(image.tags, tags) ? { ...image, tags, dirty: true } : image));
  const tagDrag = useTagFieldDrag({
    scope: cur?.path ?? '', revision: cur?.tags, disabled: !cur || loading,
    onDrop: (source, target) => {
      if (!cur || source.field !== target.field || cur.tags[source.index] !== source.value) return;
      setCurrentTags(cur.path, moveTagWithin(cur.tags, source.index, target.index));
    },
  });

  // ── 保存当前图片 ──
  const handleSaveSingle = async () => {
    if (!cur) return;
    const sent = new Map([[cur.path, cur.tags]]);
    setSavingSingle(true);
    try {
      await invoke('save_single_tag_file', { imagePath: cur.path, tags: cur.tags });
      setImages(previous => settleSaved(previous, sent, [], img => img.tags));
    } catch (e) {
      setAlertMsg(`${t('tagEditor.saveFailed')}: ${String(e)}`);
    } finally { setSavingSingle(false); }
  };

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (mode !== 'danbooru') return;
    // 焦点在输入框、文本域或 contenteditable 里时，方向键留给编辑控件，不切换图片
    const el = e.target as HTMLElement | null;
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el?.isContentEditable) return;
    if (e.key === 'ArrowLeft') { e.preventDefault(); goPrev(); }
    if (e.key === 'ArrowRight') { e.preventDefault(); goNext(); }
  }, [goPrev, goNext, mode]);

  const dirtyCount = images.filter(i => i.dirty).length;

  const topBar = mode === 'json'
    ? { loading: jsonLoading, saving: jsonSaving, dirty: jsonDirtyCount, onLoad: () => jsonTabRef.current?.loadFolder(), onSave: () => jsonTabRef.current?.saveAll() }
    : mode === 'natural'
      ? { loading, saving, dirty: nlImages.filter(i => i.dirty).length, onLoad: handleLoadFolder, onSave: handleSaveAllCaptions }
      : { loading, saving, dirty: dirtyCount, onLoad: handleLoadFolder, onSave: handleSaveAll };
  const replaceMatches = tagStats.filter(([tag]) => !replaceSearch || tag.includes(replaceSearch.toLowerCase()));

  return (
    <>
    <div className="page" style={{ height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden', gap: 0 }} onKeyDown={handleKeyDown} tabIndex={0}>
      <PageHeader icon={List} color="#7c5cfc" title={t('tagManager.title')} subtitle={t('tagManager.subtitle')} />

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)', flexShrink: 0 }}>
        <SegmentedTabs value={mode} onChange={setMode} tabs={[
          { id: 'danbooru', label: t('tagManager.danbooruTab') },
          { id: 'natural', label: t('tagManager.naturalTab') },
          { id: 'json', label: t('tagManager.jsonTab') },
        ]} />
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <RecursiveScanToggle checked={recursive} onChange={setRecursive} style={{ height: 34 }} />
          <button className="btn btn-secondary" style={topButton} onClick={topBar.onLoad} disabled={topBar.loading}>
            {topBar.loading ? <Loader2 style={spinner14} /> : <FolderOpen style={{ width: 14, height: 14 }} />}
            {' '}{topBar.loading ? t('tagManager.loading') : t('tagManager.loadFolder')}
          </button>
          <button className="btn btn-primary" style={topButton} disabled={topBar.dirty === 0 || topBar.saving} onClick={topBar.onSave}>
            {topBar.saving ? <Loader2 style={spinner14} /> : <Save style={{ width: 14, height: 14 }} />}
            {' '}{t('tagManager.saveAll')}{topBar.dirty > 0 ? ` (${topBar.dirty})` : ''}
          </button>
        </div>
      </div>

      {mode === 'danbooru' && (
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>
        <ImageGridColumn key={folderPath} width={col1.size} items={filtered} total={images.length} tagged={taggedCount}
          search={searchText} onSearch={setSearchText} filter={filterMode} onFilter={setFilterMode}
          selected={selectedIdx} onSelect={setSelectedIdx} onRefresh={folderPath ? handleRefresh : undefined} loading={loading}
          badge={image => image.tags.length || null} />

        <ResizeHandle axis="x" onMouseDown={col1.onMouseDown} />

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>
          <ImagePreviewPane ref={previewRow.beforeRef} path={cur?.path} filename={cur?.filename} index={selectedIdx} total={images.length}
            onPrev={goPrev} onNext={goNext} style={{ flex: previewRow.ratio, minHeight: 80 }} />

          <ResizeHandle axis="y" onMouseDown={previewRow.onMouseDown} />

          <div ref={previewRow.afterRef} className="tag-card" style={{ flex: 1, minHeight: 60 }}>
            <TagPanelHeader icon={Tags} color="#4ade80" title={t('tagManager.imageTags')} extra={
              <span className="tag-panel-count" style={{ background: cur?.dirty ? 'rgba(239,68,68,0.1)' : 'rgba(74,222,128,0.1)', color: cur?.dirty ? '#ef4444' : '#4ade80' }}>
                {cur?.tags.length || 0}
              </span>
            }>
              <PanelSaveButton saving={savingSingle} disabled={!cur || !cur.dirty} onClick={handleSaveSingle} />
            </TagPanelHeader>
            <div {...tagDrag.editorProps} style={{ flex: 1, padding: '10px 14px', overflowY: 'auto' }}>
              {!cur && <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', fontStyle: 'italic' }}>{t('tagEditor.selectToEdit')}</span>}
              {cur && <TagChipList key={cur.path} values={cur.tags} translations={translations} normalize={raw => normalizeNewTag(raw, cur)}
                fieldDrag={tagDrag.bindField('tags')} onChange={tags => setCurrentTags(cur.path, tags)}
                onRename={(from, to) => setSelectedTags(previous => renameSelected(previous, from, to))} />}
            </div>
          </div>
        </div>

        <ResizeHandle axis="x" onMouseDown={col3.onMouseDown} />

        <div className="tag-card" style={{ width: col3.size, minWidth: 160, maxWidth: 500, flexShrink: 0, flexDirection: 'row' }}>
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            <TagPanelHeader icon={BarChart3} color="#60a5fa" title={tagListMode === 'common' ? t('tagEditor.commonTags') : t('tagEditor.allTags')}
              extra={<span className="tag-panel-count" style={{ background: 'rgba(96,165,250,0.1)', color: '#60a5fa' }}>{filteredStats.length}</span>}>
              <div className="tag-header-actions">
                <button className="btn btn-ghost btn-sm tag-icon-btn" title={t('tagManager.escapeAllTip')} onClick={handleEscapeAll} disabled={images.length === 0}>
                  <Wand2 style={{ width: 12, height: 12 }} />
                </button>
                <TagStatsActions stats={stats} translation={translation} disabled={images.length === 0} onError={setAlertMsg} />
              </div>
            </TagPanelHeader>
            <TagStatsPanel stats={stats} translations={translations} currentTags={new Set(cur?.tags ?? [])} total={images.length}
              filteredCount={filtered.length} filterActive={tagFilterActive} onClearFilter={() => setTagFilterActive(false)} progress={translateProgress} />
          </div>

          <ToolSidebar items={[
            { icon: Filter, label: t('tagEditor.filterByTag'), onClick: () => setTagFilterActive(v => !v), disabled: images.length === 0, color: tagFilterActive ? '#7c5cfc' : undefined },
            { icon: Replace, label: t('tagManager.replaceTag'), disabled: images.length === 0, onClick: () => {
              setReplaceTo(''); setReplaceFrom(selectedTags.size === 1 ? [...selectedTags][0] : ''); setShowReplaceModal(true);
            } },
            { icon: ListPlus, label: t('tagEditor.batchAdd'), disabled: images.length === 0, onClick: () => {
              setAddTagInput(''); setAddPosition('start'); setAddOverwrite(false); setAddScope('all'); setShowAddModal(true);
            } },
            { icon: CopyX, label: t('tagEditor.dedupeTags'), onClick: handleDeduplicateTags, disabled: images.length === 0, color: '#f59e0b' },
            { icon: Trash2, label: t('tagEditor.deleteSelected'), onClick: () => { setDeleteScope('all'); setShowDeleteModal(true); },
              disabled: selectedTags.size === 0, color: selectedTags.size > 0 ? RED : undefined },
            { icon: PlusCircle, label: t('tagManager.addToCurrent'), onClick: addSelectedToCurrent, disabled: !cur || selectedTags.size === 0 },
            { icon: MinusCircle, label: t('tagManager.removeFromCurrent'), onClick: removeSelectedFromCurrent,
              disabled: !cur || selectedTags.size === 0 || !curHasAllSelected, color: curHasAllSelected && selectedTags.size > 0 ? RED : undefined },
          ]} />
        </div>
      </div>
      )}

      {mode === 'natural' && (
        <NaturalLangTab images={nlImages} setImages={setNlImages} onRefresh={folderPath ? handleRefresh : undefined} onError={setAlertMsg} />
      )}

      <div style={{ display: mode === 'json' ? 'flex' : 'none', flex: 1, overflow: 'hidden', minHeight: 0 }}>
        <JsonTagTab ref={jsonTabRef} recursive={recursive} onDirtyChange={setJsonDirtyCount} onLoadingChange={setJsonLoading} onSavingChange={setJsonSaving} />
      </div>

      <Modal open={showAddModal} onClose={() => setShowAddModal(false)} title={t('tagEditor.batchAddTitle')} width={380}>
        <label style={modalLabel}>{t('tagEditor.tagContent')}</label>
        <input className="form-input" placeholder="1girl, solo, smile" value={addTagInput} onChange={e => setAddTagInput(e.target.value)}
          style={{ fontSize: 12, marginBottom: 12 }} />
        <OptionChips legend={t('tagEditor.position')} value={addPosition} onChange={setAddPosition} color="#4ade80"
          options={[{ value: 'start', label: t('tagEditor.prepend') }, { value: 'end', label: t('tagEditor.append') }]} />
        <ScopeToggle value={addScope} onChange={setAddScope} hasCurrent={!!cur} />
        <Checkbox checked={addOverwrite} onChange={setAddOverwrite} size={14} label={t('tagManager.overwriteIfExist')}
          style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 16 }} />
        <div style={modalActions}>
          <button className="btn btn-secondary" style={modalButton} onClick={() => setShowAddModal(false)}>{t('common.cancel')}</button>
          <button className="btn btn-primary" style={modalButton} onClick={handleBatchAdd} disabled={!addTagInput.trim() || (addScope === 'current' && !cur)}>
            {addScope === 'all' ? t('tagManager.addToAll') : t('tagManager.addToCurrentOne')}
          </button>
        </div>
      </Modal>
      <Modal open={showDeleteModal} onClose={() => setShowDeleteModal(false)} title={t('tagEditor.batchDeleteTitle')} width={380}>
        <div style={{ fontSize: 11, color: 'var(--color-text-secondary)', marginBottom: 10 }}>
          {t('tagEditor.deleteTagsHint', { n: selectedTags.size })}
        </div>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginBottom: 12, maxHeight: 96, overflowY: 'auto', overscrollBehavior: 'contain' }}>
          {[...selectedTags].map(tag => (
            <span key={tag} style={{ fontSize: 10, padding: '1px 7px', borderRadius: 4, border: '1px solid rgba(248,113,113,0.45)',
              background: 'rgba(248,113,113,0.06)', color: 'var(--color-text-secondary)' }}>{tag}</span>
          ))}
        </div>
        <ScopeToggle value={deleteScope} onChange={setDeleteScope} hasCurrent={!!cur} color={RED} />
        <div style={modalActions}>
          <button className="btn btn-secondary" style={modalButton} onClick={() => setShowDeleteModal(false)}>{t('common.cancel')}</button>
          <button className="btn btn-primary" style={{ ...modalButton, background: '#ef4444', borderColor: '#ef4444' }} onClick={handleBatchDelete}
            disabled={deleteScope === 'current' && !cur}>
            {deleteScope === 'all' ? t('tagEditor.deleteFromAll') : t('tagEditor.deleteFromCurrentOne')}
          </button>
        </div>
      </Modal>
      <Modal open={showReplaceModal} onClose={() => setShowReplaceModal(false)} title={t('tagManager.replaceTitle')} width={380}>
        <label style={modalLabel}>{t('tagManager.originalTag')}</label>
        <div style={{ position: 'relative', marginBottom: 10 }}>
          <input className="form-input" placeholder={t('tagManager.searchAndSelect')} value={replaceDropOpen ? replaceSearch : replaceFrom}
            onFocus={() => { setReplaceDropOpen(true); setReplaceSearch(''); }}
            onChange={e => { setReplaceSearch(e.target.value); setReplaceDropOpen(true); }}
            style={{ fontSize: 12 }} />
          {replaceFrom && !replaceDropOpen && (
            <button onClick={() => { setReplaceFrom(''); setReplaceSearch(''); }}
              style={{ position: 'absolute', right: 8, top: '50%', transform: 'translateY(-50%)', background: 'none', border: 'none', cursor: 'pointer',
                color: 'var(--color-text-tertiary)', padding: 0, display: 'flex' }}>
              <X style={{ width: 12, height: 12 }} />
            </button>
          )}
          {replaceDropOpen && (
            <div style={{ position: 'absolute', top: '100%', left: 0, right: 0, maxHeight: 180, overflow: 'auto', background: 'var(--color-bg-elevated)',
              border: '1px solid var(--color-border)', borderRadius: 6, marginTop: 2, zIndex: 10, boxShadow: 'var(--shadow-md)' }}>
              {replaceMatches.slice(0, 50).map(([tag, count]) => (
                <div key={tag} onClick={() => { setReplaceFrom(tag); setReplaceSearch(''); setReplaceDropOpen(false); }}
                  style={{ padding: '6px 10px', fontSize: 11, cursor: 'pointer', display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                    background: replaceFrom === tag ? 'rgba(124,92,252,0.1)' : 'transparent', color: 'var(--color-text-primary)' }}
                  onMouseEnter={e => e.currentTarget.style.background = 'var(--color-bg-hover)'}
                  onMouseLeave={e => e.currentTarget.style.background = replaceFrom === tag ? 'rgba(124,92,252,0.1)' : 'transparent'}>
                  <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{tag}</span>
                  <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)', flexShrink: 0, marginLeft: 8 }}>{count}</span>
                </div>
              ))}
              {replaceMatches.length === 0 && (
                <div style={{ padding: '12px 10px', fontSize: 11, color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{t('tagEditor.noMatch')}</div>
              )}
            </div>
          )}
        </div>
        <label style={modalLabel}>{t('tagManager.replaceTo')}</label>
        <input className="form-input" placeholder={t('tagManager.newTag')} value={replaceTo} onChange={e => setReplaceTo(e.target.value)}
          style={{ fontSize: 12, marginBottom: 16 }} />
        <div style={modalActions}>
          <button className="btn btn-secondary" style={modalButton} onClick={() => { setShowReplaceModal(false); setReplaceDropOpen(false); }}>{t('common.cancel')}</button>
          <button className="btn btn-primary" style={modalButton} onClick={handleReplace} disabled={!replaceFrom.trim() || !replaceTo.trim()}>{t('tagManager.replaceAll')}</button>
        </div>
      </Modal>
    </div>

      <AlertModal
        open={!!alertMsg}
        onClose={() => setAlertMsg('')}
        title={t('tagManager.errorTitle')}
        message={alertMsg}
      />
    </>
  );
}
