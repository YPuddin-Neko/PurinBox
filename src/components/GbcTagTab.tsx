import { forwardRef, useEffect, useImperativeHandle, useState } from 'react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Image, Tags, Scan } from 'lucide-react';
import { useDragResize, useFlexRatioResize } from '../hooks/useDragResize';
import { ensureAssetScope } from '../utils/assetScope';
import { bboxRegions } from '../utils/bboxRegions';
import ImageGridColumn from './ImageGridColumn';
import BboxImage from './BboxImage';
import { TagPanelHeader, PanelSaveButton, ResizeHandle } from './TagEditorLayout';
import { AlertModal } from './Modal';

type Vertex = { vertex_id: string; label: string; bbox: Record<string, number>; descs: { label: string; text: string }[] };
type Graph = { vertices: Vertex[]; [key: string]: unknown };
type Document = { path: string; image_path: string; content: string };
type Item = Document & { value: Graph; original: string; filename: string; dirty: boolean };
const graphOf = (value: Graph) => (value.graph ?? value) as Graph;
export type GbcTagTabHandle = { loadFolder: () => void; saveAll: () => void };

export default forwardRef<GbcTagTabHandle, { recursive: boolean; onDirtyChange: (n: number) => void; onLoadingChange: (v: boolean) => void; onSavingChange: (v: boolean) => void }>(function GbcTagTab({ recursive, onDirtyChange, onLoadingChange, onSavingChange }, ref) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [items, setItems] = useState<Item[]>([]);
  const [selected, setSelected] = useState(0);
  const [region, setRegion] = useState(0);
  const [folder, setFolder] = useState('');
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<'all' | 'tagged' | 'untagged'>('all');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const col1 = useDragResize({ initial: 220, min: 160, max: 500 });
  const col3 = useDragResize({ initial: 250, min: 160, max: 500, direction: -1 });
  const previewRow = useFlexRatioResize({ initial: 3, min: 0.5, max: 6 });
  const current = items[selected];
  const graph = current && graphOf(current.value);
  const vertex = graph?.vertices[region];
  const regions = current ? bboxRegions(current.value) : [];
  useEffect(() => onDirtyChange(items.filter(item => item.dirty).length), [items, onDirtyChange]);
  useEffect(() => onLoadingChange(loading), [loading, onLoadingChange]);
  useEffect(() => onSavingChange(saving), [saving, onSavingChange]);
  const load = async (path?: string) => {
    try {
      const chosen = path || await open({ directory: true, multiple: false, title: t('tagEditor.selectFolder') });
      if (typeof chosen !== 'string') return;
      setLoading(true);
      await ensureAssetScope(chosen);
      const docs = await invoke<Document[]>('load_gbc_dataset', { folder: chosen, recursive });
      setItems(docs.map(doc => ({ ...doc, original: doc.content, value: JSON.parse(doc.content), filename: doc.image_path.split(/[\\/]/).pop()!, dirty: false })));
      setFolder(chosen); setSelected(0); setRegion(0); setSearch(''); setFilter('all');
    } catch (e) { setError(String(e)); } finally { setLoading(false); }
  };
  const update = (mutate: (v: Vertex) => void) => setItems(previous => previous.map((item, i) => {
    if (i !== selected) return item;
    const value = structuredClone(item.value);
    mutate(graphOf(value).vertices[region]);
    return { ...item, value, dirty: true };
  }));
  const save = async (targets: Item[]) => {
    setSaving(true);
    let success = true;
    try {
      for (const item of targets.filter(i => i.dirty)) {
        const content = JSON.stringify(item.value, null, 2);
        await invoke('save_gbc_document', { path: item.path, content, original: item.original });
        setItems(previous => previous.map(now => now.path === item.path ? { ...now, original: content, dirty: now.value !== item.value } : now));
      }
    } catch (e) { setError(String(e)); success = false; } finally { setSaving(false); }
    return success;
  };
  useImperativeHandle(ref, () => ({ loadFolder: () => { void load(); }, saveAll: () => { void save(items); } }));
  const choose = (index: number) => { setSelected(index); setRegion(0); };
  const filtered = items.map((item, _i) => ({ ...item, path: item.image_path, _i })).filter(item => filter !== 'untagged' && item.filename.toLowerCase().includes(search.toLowerCase()));
  return <>
    <div style={{ flex: 1, display: 'flex', minHeight: 0, overflow: 'hidden' }}>
      <ImageGridColumn width={col1.size} items={filtered} total={items.length} tagged={items.length} search={search} onSearch={setSearch} filter={filter} onFilter={setFilter} selected={selected} onSelect={choose} onRefresh={folder ? () => void load(folder) : undefined} loading={loading} badge={item => graphOf(item.value).vertices.length} />
      <ResizeHandle axis="x" onMouseDown={col1.onMouseDown} />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>
        <div ref={previewRow.beforeRef} className="tag-card" style={{ flex: previewRow.ratio, minHeight: 80 }}>
          <TagPanelHeader icon={Image} color="#7c5cfc" title={t('tagEditor.preview')} extra={<span className="tag-panel-note">{current?.filename}</span>} />
          <div className="tag-preview-body">{current ? <BboxImage src={convertFileSrc(current.image_path)} alt={current.filename} regions={regions} selectedRegion={vertex?.vertex_id} onRegionSelect={id => setRegion(graph!.vertices.findIndex(v => v.vertex_id === id))} /> : <div className="tag-preview-empty">{t('tagEditor.loadToShow')}</div>}</div>
        </div>
        <ResizeHandle axis="y" onMouseDown={previewRow.onMouseDown} />
        <div ref={previewRow.afterRef} className="tag-card" style={{ flex: 1, minHeight: 130 }}>
          <TagPanelHeader icon={Tags} color="#4ade80" title={t('gbc.description')} extra={<span className="tag-panel-note">{vertex && (vertex.vertex_id || t('gbc.wholeImage'))}</span>}>
            <PanelSaveButton saving={saving} disabled={!current?.dirty} onClick={() => void save([current])} />
          </TagPanelHeader>
          <div style={{ padding: '10px 14px', overflowY: 'auto', flex: 1 }}>
            {vertex ? vertex.descs.map((desc, i) => <textarea key={`${current.path}-${region}-${i}`} aria-label={`${t('gbc.description')} ${i + 1}`} className="form-input" rows={3} value={desc.text} disabled={saving} onChange={e => update(v => { v.descs[i].text = e.target.value; })} style={{ marginBottom: 8, resize: 'vertical' }} />) : <span className="tag-panel-note">{t('tagEditor.selectToEdit')}</span>}
          </div>
        </div>
      </div>
      <ResizeHandle axis="x" onMouseDown={col3.onMouseDown} />
      <div className="tag-card" style={{ width: col3.size, flexShrink: 0 }}>
        <TagPanelHeader icon={Scan} color="#60a5fa" title={t('gbc.region')} extra={<span className="tag-panel-count">{graph?.vertices.length ?? 0}</span>} />
        <div style={{ flex: 1, overflowY: 'auto', padding: 8 }}>
          {graph?.vertices.map((v, i) => <button type="button" key={v.vertex_id} className={`gbc-region-item${region === i ? ' selected' : ''}`} onClick={() => setRegion(i)}><span style={{ background: regions.find(r => r.id === v.vertex_id)?.color ?? 'var(--color-text-tertiary)' }} />{v.vertex_id || t('gbc.wholeImage')}</button>)}
        </div>
        {vertex && <div style={{ padding: 12, borderTop: '1px solid var(--color-border)' }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>{(['left', 'top', 'right', 'bottom'] as const).map(key => <label className="form-label" key={key}>{t(`gbc.${key}`)}<input className="form-input" type="number" min={0} max={1} step={0.01} disabled={saving} value={vertex.bbox[key]} onChange={e => update(v => { v.bbox[key] = Number(e.target.value); })} /></label>)}</div>
          <button type="button" className="btn btn-secondary" disabled={saving} style={{ width: '100%', marginTop: 12 }} onClick={async () => { if (await save([current])) navigate('/tagger', { state: { gbcImage: current.image_path } }); }}>{t('gbc.continueRefine')}</button>
        </div>}
      </div>
    </div>
    <AlertModal open={!!error} message={error} onClose={() => setError('')} />
  </>;
});
