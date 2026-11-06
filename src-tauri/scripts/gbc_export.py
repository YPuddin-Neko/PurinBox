"""Validate GBC interchange graphs and render training sidecars."""
import json
import math


def graph_document(value):
    graph = value.get('graph', value) if isinstance(value, dict) else None
    if not isinstance(graph, dict) or not isinstance(graph.get('vertices'), list):
        raise ValueError('GBC 缺少 vertices')
    vertices = graph['vertices']
    by_id = {}
    for vertex in vertices:
        if not isinstance(vertex, dict):
            raise ValueError('GBC 节点必须为对象')
        key = vertex.get('vertex_id')
        if not isinstance(key, str) or key in by_id:
            raise ValueError('GBC 节点 ID 缺失或重复')
        if vertex.get('label') not in ('image', 'entity', 'composition', 'relation'):
            raise ValueError('GBC 节点类型无效')
        box = vertex.get('bbox', {})
        if not isinstance(box, dict):
            raise ValueError('GBC 区域框无效')
        coords = [box.get(k) for k in ('left', 'top', 'right', 'bottom')]
        if any(isinstance(x, bool) or not isinstance(x, (int, float)) or not math.isfinite(x) or not 0 <= x <= 1 for x in coords):
            raise ValueError('GBC 坐标必须在 0 到 1 之间')
        if coords[0] >= coords[2] or coords[1] >= coords[3]:
            raise ValueError('GBC 区域必须具有正面积')
        descs = vertex.get('descs')
        if not isinstance(descs, list) or not descs or any(not isinstance(d, dict) or not isinstance(d.get('text'), str) or not d['text'].strip() or not isinstance(d.get('label'), str) for d in descs):
            raise ValueError('GBC 节点描述无效')
        by_id[key] = vertex
    roots = [v['vertex_id'] for v in vertices if v['label'] == 'image']
    if len(roots) != 1:
        raise ValueError('GBC 必须有一个全图根节点')
    incoming = {key: [] for key in by_id}
    adjacency = {key: [] for key in by_id}
    for key, vertex in by_id.items():
        edges = vertex.get('out_edges', [])
        if not isinstance(edges, list) or any(not isinstance(edge, dict) for edge in edges):
            raise ValueError('GBC 边列表无效')
        for edge in edges:
            if edge.get('source') != key or edge.get('target') not in by_id or not isinstance(edge.get('text'), str):
                raise ValueError('GBC 边引用无效')
            adjacency[key].append(edge['target'])
            incoming[edge['target']].append(edge)
        for field in ('sub_masks', 'super_masks'):
            if any(ref not in by_id for ref in vertex.get(field, [])):
                raise ValueError('GBC 遮罩引用无效')
    for key, vertex in by_id.items():
        if 'in_edges' in vertex:
            if not isinstance(vertex['in_edges'], list):
                raise ValueError('GBC 入边列表无效')
            canonical = lambda edges: sorted(json.dumps(e, sort_keys=True) for e in edges)
            if canonical(vertex['in_edges']) != canonical(incoming[key]):
                raise ValueError('GBC 入边与出边不一致')
    visiting, visited = set(), set()
    def visit(key):
        if key in visiting:
            raise ValueError('GBC 包含环')
        if key in visited:
            return
        visiting.add(key)
        for target in adjacency[key]:
            visit(target)
        visiting.remove(key)
        visited.add(key)
    visit(roots[0])
    if len(visited) != len(vertices):
        raise ValueError('GBC 包含无法从根到达的节点')
    return graph


def description(vertex):
    descs = vertex['descs']
    for label in ('detail', 'original', 'short'):
        texts = [d['text'].strip() for d in descs if d['label'] == label]
        if texts:
            return ' '.join(dict.fromkeys(texts))
    return descs[0]['text'].strip()


def layout_document(value):
    allowed = {'width', 'height', 'bbox_coord_space', 'bbox_format', 'summary', 'items'}
    if not isinstance(value, dict) or set(value) != allowed:
        raise ValueError('Layout 根字段无效')
    w, h = value['width'], value['height']
    if any(isinstance(n, bool) or not isinstance(n, int) or n <= 0 or n % 16 for n in (w, h)):
        raise ValueError('Layout 画布尺寸须为 16 的正整数倍')
    if value['bbox_coord_space'] != 'abs' or value['bbox_format'] != 'xyxy':
        raise ValueError('Layout 草稿须使用 abs / xyxy 坐标')
    summary = value['summary']
    if not isinstance(summary, dict) or set(summary) - {'caption', 'canvas', 'meta', 'palette'} or not isinstance(summary.get('caption'), str) or not summary['caption'].strip():
        raise ValueError('Layout 全图描述无效')
    if summary.get('canvas', 'single') not in ('single', 'multi_panel', 'collage'):
        raise ValueError('Layout 画布类型无效')
    items = value['items']
    if not isinstance(items, list) or not items:
        raise ValueError('Layout 至少需要一个区域')
    for item in items:
        if not isinstance(item, dict) or set(item) - {'label', 'caption', 'bbox', 'text', 'tags', 'palette'}:
            raise ValueError('Layout 区域字段无效')
        if item.get('label') not in ('object', 'character', 'text') or not isinstance(item.get('caption'), str) or not item['caption'].strip():
            raise ValueError('Layout 区域描述无效')
        if item['label'] == 'text' and (not isinstance(item.get('text'), str) or not item['text'].strip()):
            raise ValueError('文字区域必须包含实际文字')
        box = item.get('bbox')
        if not isinstance(box, list) or len(box) != 4 or any(isinstance(n, bool) or not isinstance(n, (int, float)) or not math.isfinite(n) for n in box):
            raise ValueError('Layout 区域框无效')
        if not (0 <= box[0] < box[2] <= w and 0 <= box[1] < box[3] <= h):
            raise ValueError('Layout 区域框超出画布或面积为零')
    for obj in [summary, *items]:
        for field in ('meta', 'palette', 'tags'):
            if field in obj and (not isinstance(obj[field], list) or any(not isinstance(s, str) for s in obj[field])):
                raise ValueError('Layout 列表字段无效')
    return value


def layout_from_graph(graph, width, height):
    graph = graph_document(graph)
    scale = 16 // math.gcd(math.gcd(width, height), 16)
    width, height = width * scale, height * scale
    root = next(v for v in graph['vertices'] if v['label'] == 'image')
    relations = [description(v) for v in graph['vertices'] if v['label'] == 'relation']
    items = []
    for vertex in graph['vertices']:
        if vertex['label'] not in ('entity', 'composition'):
            continue
        box = vertex['bbox']
        items.append({'label': 'object', 'caption': description(vertex),
                      'bbox': [box['left'] * width, box['top'] * height, box['right'] * width, box['bottom'] * height]})
    return layout_document({'width': width, 'height': height, 'bbox_coord_space': 'abs', 'bbox_format': 'xyxy',
                            'summary': {'canvas': 'single', 'caption': ' '.join([description(root), *relations])}, 'items': items})
