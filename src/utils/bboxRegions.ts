export interface BboxRegion {
  id: string;
  label: string;
  box: [number, number, number, number];
  color: string;
  nested: boolean;
}
const regionColor = (index: number) => `hsl(${(275 + index * 137.508) % 360} 72% 65%)`;
const object = (v: unknown): Record<string, any> | null => v !== null && typeof v === 'object' && !Array.isArray(v) ? v : null;
function box(values: unknown[], width = 1, height = 1): BboxRegion['box'] | null {
  if (!(width > 0 && height > 0) || values.length !== 4 || values.some(v => typeof v !== 'number' || !Number.isFinite(v))) return null;
  const [l, t, r, b] = (values as number[]).map((v, i) => Math.max(0, Math.min(1, v / (i % 2 ? height : width))));
  return l < r && t < b ? [l, t, r, b] : null;
}
const text = (v: unknown) => typeof v === 'string' ? v.trim() : '';

export function bboxRegions(document: unknown): BboxRegion[] {
  const root = object(document);
  if (!root) return [];
  const graph = object(root.graph) ?? root;
  if (Array.isArray(graph.vertices)) {
    const vertices = graph.vertices.map(object).filter((v): v is Record<string, any> => v !== null);
    const byId = new Map(vertices.map(v => [v.vertex_id, v]));
    const regions: BboxRegion[] = [];
    for (const vertex of vertices) {
      if (!['entity', 'composition'].includes(vertex.label)) continue;
      const b = object(vertex.bbox);
      const coords = b && box([b.left, b.top, b.right, b.bottom]);
      if (!coords) continue;
      const incoming = vertices.flatMap(v => Array.isArray(v.out_edges) ? v.out_edges.filter((e: any) => object(e) && e.target === vertex.vertex_id) : []);
      const names = [...new Set(incoming.map((e: any) => text(e.text)).filter(Boolean))];
      const descriptions = Array.isArray(vertex.descs) ? vertex.descs : [];
      const label = names.join(' / ') || text(descriptions.find((d: any) => object(d) && d.label === 'short')?.text) || text(vertex.vertex_id) || text(descriptions.find((d: any) => object(d))?.text);
      const parent = incoming.find((e: any) => ['entity', 'composition'].includes(byId.get(e.source)?.label));
      regions.push({ id: String(vertex.vertex_id), label, box: coords, color: regionColor(regions.length), nested: Boolean(parent) });
    }
    return regions;
  }
  if (root.bbox_coord_space !== 'abs' || root.bbox_format !== 'xyxy' || !Array.isArray(root.items)
      || typeof root.width !== 'number' || typeof root.height !== 'number') return [];
  return root.items.flatMap((value: unknown, index: number) => {
    const item = object(value);
    const coords = item && Array.isArray(item.bbox) && box(item.bbox, root.width, root.height);
    return coords ? [{ id: String(index), label: text(item.text) || text(item.caption) || text(item.label), box: coords, color: regionColor(index), nested: false }] : [];
  });
}
