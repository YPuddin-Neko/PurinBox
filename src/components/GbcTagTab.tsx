import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useState } from 'react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Check, ChevronLeft, ChevronRight, Copy, Eye, EyeOff, Focus, Image, Layers, Scan, Tags, ZoomIn,
} from 'lucide-react';
import { useDragResize, useFlexRatioResize } from '../hooks/useDragResize';
import { ensureAssetScope } from '../utils/assetScope';
import { bboxRegions, type BboxRegion } from '../utils/bboxRegions';
import ImageGridColumn from './ImageGridColumn';
import BboxImage from './BboxImage';
import ImageLightbox from './ImageLightbox';
import { TagPanelHeader, PanelSaveButton, ResizeHandle } from './TagEditorLayout';
import { AlertModal } from './Modal';

type Edge = { source: string; target: string; text?: string };
type Vertex = {
  vertex_id: string;
  label: string;
  bbox: Record<string, number>;
  descs: { label: string; text: string }[];
  out_edges?: Edge[];
  in_edges?: Edge[];
};
type Graph = { vertices: Vertex[]; img_size?: [number, number]; [key: string]: unknown };
type Document = { path: string; image_path: string; content: string };
type Item = Document & { value: Graph; original: string; filename: string; dirty: boolean };

interface RegionTreeNode {
  vertex: Vertex;
  index: number;
  depth: number;
  displayName: string;
  color: string;
  nested: boolean;
  isRoot: boolean;
}

const graphOf = (value: Graph) => (value.graph ?? value) as Graph;
const MIN_SPAN = 0.01;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
const round3 = (value: number) => Math.round(value * 1000) / 1000;

function buildRegionTree(
  graph: Graph | undefined,
  regions: readonly BboxRegion[],
  wholeImageLabel: string,
): RegionTreeNode[] {
  if (!graph?.vertices?.length) return [];
  const vertices = graph.vertices;
  const indexById = new Map(vertices.map((v, i) => [v.vertex_id, i]));
  const regionById = new Map(regions.map(r => [r.id, r]));

  const rootIdx = vertices.findIndex(v => v.label === 'image' || v.vertex_id === '');
  const visited = new Set<number>();
  const ordered: { index: number; depth: number }[] = [];

  const visit = (idx: number, depth: number) => {
    if (idx < 0 || idx >= vertices.length || visited.has(idx)) return;
    visited.add(idx);
    ordered.push({ index: idx, depth });
    const out = vertices[idx].out_edges;
    if (Array.isArray(out)) {
      for (const edge of out) {
        const targetIdx = indexById.get(edge.target);
        if (targetIdx !== undefined && !visited.has(targetIdx)) {
          visit(targetIdx, depth + 1);
        }
      }
    }
  };

  if (rootIdx >= 0) visit(rootIdx, 0);
  for (let i = 0; i < vertices.length; i++) {
    if (!visited.has(i)) visit(i, rootIdx >= 0 ? 1 : 0);
  }

  return ordered.map(({ index, depth }) => {
    const vertex = vertices[index];
    const isRoot = vertex.label === 'image' || vertex.vertex_id === '';
    const reg = regionById.get(vertex.vertex_id);
    const shortText = vertex.descs?.find(d => d.label === 'short')?.text?.trim();
    const displayName = isRoot
      ? wholeImageLabel
      : (reg?.label || shortText || vertex.vertex_id);
    return {
      vertex,
      index,
      depth,
      displayName,
      color: reg?.color ?? 'var(--color-text-tertiary)',
      nested: Boolean(reg?.nested || depth >= 2),
      isRoot,
    };
  });
}

export type GbcTagTabHandle = { loadFolder: () => void; saveAll: () => void };

export default forwardRef<GbcTagTabHandle, {
  recursive: boolean;
  onDirtyChange: (n: number) => void;
  onLoadingChange: (v: boolean) => void;
  onSavingChange: (v: boolean) => void;
}>(function GbcTagTab({ recursive, onDirtyChange, onLoadingChange, onSavingChange }, ref) {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const [items, setItems] = useState<Item[]>([]);
  const [selected, setSelected] = useState(0);
  const [region, setRegion] = useState(0);
  const [hoveredRegion, setHoveredRegion] = useState<string | null>(null);
  const [focusMode, setFocusMode] = useState(true);
  const [showBoxes, setShowBoxes] = useState(true);
  const [peekingRaw, setPeekingRaw] = useState(false);
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number } | null>(null);
  const [copiedDescIdx, setCopiedDescIdx] = useState<number | null>(null);

  const [folder, setFolder] = useState('');
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<'all' | 'tagged' | 'untagged'>('all');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [zoomed, setZoomed] = useState(false);

  const col1 = useDragResize({ initial: 220, min: 160, max: 500 });
  const col3 = useDragResize({ initial: 272, min: 180, max: 520, direction: -1 });
  const previewRow = useFlexRatioResize({ initial: 3, min: 0.5, max: 6 });

  const current = items[selected];
  const graph = current && graphOf(current.value);
  const vertex = graph?.vertices[region];
  const regions = useMemo(() => (current ? bboxRegions(current.value) : []), [current]);
  const wholeImageLabel = t('gbc.wholeImage');
  const treeNodes = useMemo(
    () => buildRegionTree(graph, regions, wholeImageLabel),
    [graph, regions, wholeImageLabel],
  );

  const activeNode = useMemo(
    () => treeNodes.find(node => node.index === region),
    [treeNodes, region],
  );

  const filtered = useMemo(
    () => items
      .map((item, _i) => ({ ...item, path: item.image_path, _i }))
      .filter(item => filter !== 'untagged' && item.filename.toLowerCase().includes(search.toLowerCase())),
    [items, filter, search],
  );

  const filteredPos = useMemo(
    () => filtered.findIndex(item => item._i === selected),
    [filtered, selected],
  );

  const choose = useCallback((index: number) => {
    setSelected(index);
    setRegion(0);
    setHoveredRegion(null);
  }, []);

  const goPrevImage = useCallback(() => {
    if (filteredPos > 0) choose(filtered[filteredPos - 1]._i);
  }, [filtered, filteredPos, choose]);

  const goNextImage = useCallback(() => {
    if (filteredPos >= 0 && filteredPos < filtered.length - 1) choose(filtered[filteredPos + 1]._i);
  }, [filtered, filteredPos, choose]);

  useEffect(() => onDirtyChange(items.filter(item => item.dirty).length), [items, onDirtyChange]);
  useEffect(() => onLoadingChange(loading), [loading, onLoadingChange]);
  useEffect(() => onSavingChange(saving), [saving, onSavingChange]);

  // Keyboard shortcuts: Left/Right images, Up/Down GBC regions, hold Space to hide boxes
  useEffect(() => {
    const isTypingTarget = (target: EventTarget | null) => {
      const el = target as HTMLElement | null;
      return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || Boolean(el?.isContentEditable);
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (zoomed || isTypingTarget(event.target)) return;
      if (event.key === 'ArrowLeft') {
        event.preventDefault();
        goPrevImage();
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        goNextImage();
      } else if (event.key === 'ArrowUp' && treeNodes.length > 0) {
        event.preventDefault();
        const curPos = treeNodes.findIndex(n => n.index === region);
        const nextPos = curPos > 0 ? curPos - 1 : treeNodes.length - 1;
        setRegion(treeNodes[nextPos].index);
      } else if (event.key === 'ArrowDown' && treeNodes.length > 0) {
        event.preventDefault();
        const curPos = treeNodes.findIndex(n => n.index === region);
        const nextPos = curPos >= 0 && curPos < treeNodes.length - 1 ? curPos + 1 : 0;
        setRegion(treeNodes[nextPos].index);
      } else if (event.code === 'Space' && !event.repeat && current) {
        event.preventDefault();
        setPeekingRaw(true);
      }
    };

    const onKeyUp = (event: KeyboardEvent) => {
      if (event.code === 'Space') setPeekingRaw(false);
    };

    const onBlur = () => setPeekingRaw(false);

    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);
    window.addEventListener('blur', onBlur);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
      window.removeEventListener('blur', onBlur);
    };
  }, [zoomed, goPrevImage, goNextImage, treeNodes, region, current]);

  const load = async (path?: string) => {
    try {
      const chosen = path || await open({ directory: true, multiple: false, title: t('tagEditor.selectFolder') });
      if (typeof chosen !== 'string') return;
      setLoading(true);
      await ensureAssetScope(chosen);
      const docs = await invoke<Document[]>('load_gbc_dataset', { folder: chosen, recursive });
      setItems(docs.map(doc => ({
        ...doc,
        original: doc.content,
        value: JSON.parse(doc.content),
        filename: doc.image_path.split(/[\\/]/).pop()!,
        dirty: false,
      })));
      setFolder(chosen);
      setSelected(0);
      setRegion(0);
      setHoveredRegion(null);
      setSearch('');
      setFilter('all');
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const update = (mutate: (v: Vertex) => void) => setItems(previous => previous.map((item, i) => {
    if (i !== selected) return item;
    const value = structuredClone(item.value);
    mutate(graphOf(value).vertices[region]);
    return { ...item, value, dirty: true };
  }));

  const updateRegionBoxById = useCallback((id: string, [left, top, right, bottom]: [number, number, number, number]) => {
    setItems(previous => previous.map((item, i) => {
      if (i !== selected) return item;
      const value = structuredClone(item.value);
      const target = graphOf(value).vertices.find(v => v.vertex_id === id);
      if (!target) return item;
      target.bbox = { ...target.bbox, left, top, right, bottom };
      return { ...item, value, dirty: true };
    }));
  }, [selected]);

  const updateCoordField = (key: 'left' | 'top' | 'right' | 'bottom', rawValue: number) => {
    if (!Number.isFinite(rawValue)) return;
    update(v => {
      const b = v.bbox;
      let next = clamp(round3(rawValue), 0, 1);
      if (key === 'left') next = Math.min(next, round3(b.right - MIN_SPAN));
      if (key === 'right') next = Math.max(next, round3(b.left + MIN_SPAN));
      if (key === 'top') next = Math.min(next, round3(b.bottom - MIN_SPAN));
      if (key === 'bottom') next = Math.max(next, round3(b.top + MIN_SPAN));
      v.bbox[key] = next;
    });
  };

  const save = async (targets: Item[]) => {
    setSaving(true);
    let success = true;
    try {
      for (const item of targets.filter(i => i.dirty)) {
        const content = JSON.stringify(item.value, null, 2);
        await invoke('save_gbc_document', { path: item.path, content, original: item.original });
        setItems(previous => previous.map(now => now.path === item.path ? { ...now, original: content, dirty: now.value !== item.value } : now));
      }
    } catch (e) {
      setError(String(e));
      success = false;
    } finally {
      setSaving(false);
    }
    return success;
  };

  useImperativeHandle(ref, () => ({ loadFolder: () => { void load(); }, saveAll: () => { void save(items); } }));

  const copyDescription = (text: string, idx: number) => {
    void navigator.clipboard?.writeText(text);
    setCopiedDescIdx(idx);
    window.setTimeout(() => setCopiedDescIdx(cur => (cur === idx ? null : cur)), 1200);
  };

  const descBadgeMeta = (label: string) => {
    const norm = label.toLowerCase();
    if (norm === 'short') return { text: t('gbc.descShort'), cls: 'is-short' };
    if (norm === 'detail') return { text: t('gbc.descDetail'), cls: 'is-detail' };
    if (norm === 'original') return { text: t('gbc.descOriginal'), cls: 'is-other' };
    return { text: label, cls: 'is-other' };
  };

  const canvasDims = naturalSize
    ?? (Array.isArray(graph?.img_size) && graph.img_size.length === 2
      ? { width: graph.img_size[0], height: graph.img_size[1] }
      : null);

  const activePxDims = vertex && !activeNode?.isRoot && canvasDims
    ? {
        w: Math.max(1, Math.round((vertex.bbox.right - vertex.bbox.left) * canvasDims.width)),
        h: Math.max(1, Math.round((vertex.bbox.bottom - vertex.bbox.top) * canvasDims.height)),
      }
    : null;

  const boxesVisible = showBoxes && !peekingRaw;

  return (
    <>
      <div style={{ flex: 1, display: 'flex', minHeight: 0, overflow: 'hidden' }}>
        <ImageGridColumn
          width={col1.size}
          items={filtered}
          total={items.length}
          tagged={items.length}
          search={search}
          onSearch={setSearch}
          filter={filter}
          onFilter={setFilter}
          selected={selected}
          onSelect={choose}
          onRefresh={folder ? () => void load(folder) : undefined}
          loading={loading}
          badge={item => graphOf(item.value).vertices.length}
        />

        <ResizeHandle axis="x" onMouseDown={col1.onMouseDown} />

        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, overflow: 'hidden' }}>
          <div ref={previewRow.beforeRef} className="tag-card" style={{ flex: previewRow.ratio, minHeight: 80 }}>
            <TagPanelHeader
              icon={Image}
              color="var(--color-accent-primary)"
              title={t('tagEditor.preview')}
              extra={
                <>
                  {current?.filename && <span className="tag-panel-note">{current.filename}</span>}
                  {activeNode && !activeNode.isRoot && (
                    <span className="gbc-active-pill" title={activeNode.displayName}>
                      <span className="gbc-active-pill-dot" style={{ background: activeNode.color }} />
                      <span>{activeNode.displayName}</span>
                    </span>
                  )}
                </>
              }
            >
              {current && (
                <div className="gbc-toolbar-group">
                  <button
                    type="button"
                    className={`gbc-toolbar-btn ${focusMode ? 'is-active' : ''}`}
                    onClick={() => setFocusMode(v => !v)}
                    aria-pressed={focusMode}
                    title={focusMode ? t('gbc.focusMode') : t('gbc.showAll')}
                  >
                    {focusMode ? <Focus /> : <Layers />}
                    <span>{focusMode ? t('gbc.focusMode') : t('gbc.showAll')}</span>
                  </button>
                  <button
                    type="button"
                    className={`gbc-toolbar-btn ${boxesVisible ? 'is-active' : ''}`}
                    onClick={() => setShowBoxes(v => !v)}
                    aria-pressed={boxesVisible}
                    title={t('gbc.toggleBoxes')}
                    aria-label={t('gbc.toggleBoxes')}
                    style={{ padding: '0 6px' }}
                  >
                    {boxesVisible ? <Eye /> : <EyeOff />}
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm tag-icon-btn"
                    title={t('pages.imagePreview')}
                    aria-label={t('pages.imagePreview')}
                    onClick={() => setZoomed(true)}
                  >
                    <ZoomIn style={{ width: 13, height: 13 }} />
                  </button>
                  {filtered.length > 0 && (
                    <div className="tag-preview-nav">
                      <button type="button" className="btn btn-ghost btn-sm" onClick={goPrevImage} disabled={filteredPos <= 0}>
                        <ChevronLeft style={{ width: 14, height: 14 }} />
                      </button>
                      <span>{filteredPos >= 0 ? filteredPos + 1 : 0}/{filtered.length}</span>
                      <button type="button" className="btn btn-ghost btn-sm" onClick={goNextImage} disabled={filteredPos < 0 || filteredPos >= filtered.length - 1}>
                        <ChevronRight style={{ width: 14, height: 14 }} />
                      </button>
                    </div>
                  )}
                </div>
              )}
            </TagPanelHeader>

            <div className="tag-preview-body">
              {current ? (
                <BboxImage
                  src={convertFileSrc(current.image_path)}
                  alt={current.filename}
                  regions={regions}
                  selectedRegion={activeNode?.isRoot ? undefined : vertex?.vertex_id}
                  hoveredRegion={hoveredRegion}
                  focusSelected={focusMode}
                  hideRegions={!boxesVisible}
                  onClick={() => setZoomed(true)}
                  onRegionSelect={id => {
                    const idx = graph!.vertices.findIndex(v => v.vertex_id === id);
                    if (idx >= 0) setRegion(idx);
                  }}
                  onRegionHover={setHoveredRegion}
                  onRegionBoxChange={saving ? undefined : updateRegionBoxById}
                  onNaturalSize={setNaturalSize}
                />
              ) : (
                <div className="tag-preview-empty">
                  <Image style={{ width: 56, height: 56, opacity: 0.2 }} />
                  <span>{items.length === 0 ? t('tagEditor.loadToShow') : t('tagEditor.selectToPreview')}</span>
                </div>
              )}
            </div>
          </div>

          <ResizeHandle axis="y" onMouseDown={previewRow.onMouseDown} />

          <div ref={previewRow.afterRef} className="tag-card" style={{ flex: 1.15, minHeight: 140 }}>
            <TagPanelHeader
              icon={Tags}
              color="#4ade80"
              title={t('gbc.description')}
              extra={activeNode && (
                <span className="gbc-active-pill" title={activeNode.displayName}>
                  <span className="gbc-active-pill-dot" style={{ background: activeNode.color }} />
                  <span>{activeNode.displayName}</span>
                </span>
              )}
            >
              <PanelSaveButton saving={saving} disabled={!current?.dirty} onClick={() => void save([current])} />
            </TagPanelHeader>

            <div className="gbc-desc-stack">
              {vertex ? vertex.descs.map((desc, i) => {
                const badge = descBadgeMeta(desc.label);
                return (
                  <div key={`${current.path}-${region}-${i}`} className="gbc-desc-item">
                    <div className="gbc-desc-head">
                      <span className={`gbc-desc-badge ${badge.cls}`}>{badge.text}</span>
                      <div className="gbc-desc-tools">
                        <span>{t('gbc.charsCount', { n: desc.text.length })}</span>
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm tag-icon-btn"
                          title={t('common.copy')}
                          aria-label={t('common.copy')}
                          onClick={() => copyDescription(desc.text, i)}
                        >
                          {copiedDescIdx === i
                            ? <Check style={{ width: 11, height: 11, color: 'var(--color-success)' }} />
                            : <Copy style={{ width: 11, height: 11 }} />}
                        </button>
                      </div>
                    </div>
                    <textarea
                      aria-label={`${t('gbc.description')} ${i + 1}`}
                      className="form-input"
                      rows={desc.label.toLowerCase() === 'short' ? 2 : 3}
                      value={desc.text}
                      disabled={saving}
                      onChange={e => update(v => { v.descs[i].text = e.target.value; })}
                      style={{ fontSize: 12, lineHeight: 1.55, resize: 'vertical' }}
                    />
                  </div>
                );
              }) : (
                <span className="tag-panel-note">{t('tagEditor.selectToEdit')}</span>
              )}
            </div>
          </div>
        </div>

        <ResizeHandle axis="x" onMouseDown={col3.onMouseDown} />

        <div className="tag-card" style={{ width: col3.size, flexShrink: 0 }}>
          <TagPanelHeader
            icon={Scan}
            color="#60a5fa"
            title={t('gbc.region')}
            extra={<span className="tag-panel-count">{treeNodes.length}</span>}
          />

          <div className="gbc-region-list">
            {treeNodes.map(node => {
              const isSelected = region === node.index;
              const isHovered = !node.isRoot && hoveredRegion === node.vertex.vertex_id;
              const indent = Math.min(node.depth, 4) * 14;
              return (
                <button
                  type="button"
                  key={`${node.vertex.vertex_id}-${node.index}`}
                  className={`gbc-region-item${isSelected ? ' selected' : ''}${isHovered ? ' is-hovered' : ''}`}
                  style={{ paddingLeft: 8 + indent }}
                  title={node.isRoot ? node.displayName : `${node.displayName} (${node.vertex.vertex_id})`}
                  onClick={() => setRegion(node.index)}
                  onMouseEnter={() => setHoveredRegion(node.isRoot ? null : node.vertex.vertex_id)}
                  onMouseLeave={() => setHoveredRegion(null)}
                >
                  {node.depth >= 2 && (
                    <span
                      className="gbc-region-guide"
                      style={{ left: 8 + (Math.min(node.depth, 4) - 1) * 14 + 3 }}
                      aria-hidden="true"
                    />
                  )}
                  <span
                    className={`gbc-region-dot${node.nested ? ' is-nested' : ''}`}
                    style={{ background: node.color }}
                  />
                  <span className="gbc-region-name">{node.displayName}</span>
                  {!node.isRoot && <span className="gbc-region-meta">L{node.depth}</span>}
                </button>
              );
            })}
          </div>

          {current && vertex && (
            <div className="gbc-inspector-footer">
              {!activeNode?.isRoot && (
                <>
                  <div className="gbc-coords-head">
                    <span>{t('gbc.coords')}</span>
                    {activePxDims && (
                      <span className="gbc-coords-px">{activePxDims.w} × {activePxDims.h} px</span>
                    )}
                  </div>
                  <div className="gbc-coords-grid">
                    {(['left', 'top', 'right', 'bottom'] as const).map(key => (
                      <label className="gbc-coord-field" key={key}>
                        <span>{t(`gbc.${key}`)}</span>
                        <input
                          type="number"
                          min={0}
                          max={1}
                          step={0.01}
                          disabled={saving}
                          value={Number(vertex.bbox[key].toFixed(3))}
                          onChange={e => updateCoordField(key, Number(e.target.value))}
                        />
                      </label>
                    ))}
                  </div>
                </>
              )}
              <button
                type="button"
                className="btn btn-secondary"
                disabled={saving}
                style={{ width: '100%', height: 32, fontSize: 12 }}
                onClick={async () => {
                  if (await save([current])) navigate('/tagger', { state: { gbcImage: current.image_path } });
                }}
              >
                {t('gbc.continueRefine')}
              </button>
            </div>
          )}
        </div>
      </div>

      {zoomed && current && (
        <ImageLightbox
          src={convertFileSrc(current.image_path)}
          filename={current.filename}
          regions={boxesVisible ? regions : []}
          onClose={() => setZoomed(false)}
        />
      )}

      <AlertModal open={!!error} message={error} onClose={() => setError('')} />
    </>
  );
});
