// ═══════════════ 工作流运行前校验 ═══════════════
import type { Node, Edge } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import { getNodeDef, isInPlaceNode, withDefaults } from './nodeDefinitions';

/** 路径里的 .workflow_temp 段（中间产物目录） */
export const TEMP_PATH_RE = /[\\/]\.workflow_temp(?=[\\/]|$)/;

export type WorkflowIssueKind =
  | 'inputInTemp'
  | 'cycle'
  | 'missingInputPath'
  | 'missingOutputPath'
  | 'missingModel'
  | 'noInput'
  | 'multipleInputs'
  | 'needsOutput';

export interface WorkflowIssue {
  kind: WorkflowIssueKind;
  /** 出问题的节点；cycle 为空串 */
  nodeId: string;
}

export interface WorkflowPlan {
  /** 执行顺序（拓扑序）；有问题时为空 */
  order: string[];
  issue: WorkflowIssue | null;
}

/** 分桶节点的两个分支出口 */
const BRANCH_HANDLES = ['output-a', 'output-b'];

/** 一组分桶结果：分桶节点 id → 生效的出口 */
type Branches = ReadonlyMap<string, string>;

/** 节点结果所在的位置：kept 是输入或输出文件夹，temp 是运行结束后会清理的 .workflow_temp */
type Place = 'kept' | 'temp';

function topologicalOrder(nodes: Node<WorkflowNodeData>[], edges: Edge[]): string[] | null {
  const targets = new Map<string, string[]>(nodes.map(node => [node.id, []]));
  const inDegree = new Map<string, number>(nodes.map(node => [node.id, 0]));
  for (const edge of edges) {
    targets.get(edge.source)?.push(edge.target);
    inDegree.set(edge.target, (inDegree.get(edge.target) ?? 0) + 1);
  }
  const queue = [...inDegree].filter(([, degree]) => degree === 0).map(([id]) => id);
  const order: string[] = [];
  while (queue.length > 0) {
    const id = queue.shift()!;
    order.push(id);
    for (const target of targets.get(id) ?? []) {
      const degree = (inDegree.get(target) ?? 1) - 1;
      inDegree.set(target, degree);
      if (degree === 0) queue.push(target);
    }
  }
  return order.length === nodes.length ? order : null;
}

function compatible(a: Branches, b: Branches): boolean {
  for (const [id, handle] of a) {
    const other = b.get(id);
    if (other !== undefined && other !== handle) return false;
  }
  return true;
}

function dedupe(list: Branches[]): Branches[] {
  const unique = new Map<string, Branches>();
  for (const branches of list) {
    unique.set([...branches].map(([id, handle]) => `${id}:${handle}`).sort().join('|'), branches);
  }
  return [...unique.values()];
}

const isBranchHandle = (handle: string | null | undefined): handle is string =>
  !!handle && BRANCH_HANDLES.includes(handle);

/**
 * 运行前检查整张图，返回执行顺序或第一个问题（按执行顺序）。
 *
 * 分桶节点运行时只走一个分支，所以按「分桶结果的组合」判断两路上游能否同时产出：
 * 来自同一分桶两个不同出口的上游互斥，可以汇合到一个节点。
 * 引擎按同样的规则执行，校验通过后每个节点最多一个实际产出的上游，
 * 每条链的结果都落在输入或输出文件夹里，不会只留在运行结束后清理的 .workflow_temp。
 */
export function planWorkflow(nodes: Node<WorkflowNodeData>[], edges: Edge[]): WorkflowPlan {
  const fail = (kind: WorkflowIssueKind, nodeId = ''): WorkflowPlan => ({ order: [], issue: { kind, nodeId } });

  // 运行前要清理残留的 .workflow_temp，输入在里面会被一起删掉
  const inputInTemp = nodes.find(
    node => node.data.type === 'image-folder' && TEMP_PATH_RE.test(String(node.data.params.path ?? '')),
  );
  if (inputInTemp) return fail('inputInTemp', inputInTemp.id);

  const byId = new Map(nodes.map(node => [node.id, node]));
  // 端点已不存在的连线不参与执行，也不能让拓扑排序误判成环
  const links = edges.filter(edge => byId.has(edge.source) && byId.has(edge.target));
  const order = topologicalOrder(nodes, links);
  if (!order) return fail('cycle');

  const typeOf = (id: string) => byId.get(id)?.data.type;
  // 每个节点在哪些分桶结果下会运行；分桶节点的出口边只在对应结果下生效
  const runsWhen = new Map<string, Branches[]>();
  const viaEdge = (edge: Edge): Branches[] => {
    const branches = runsWhen.get(edge.source) ?? [];
    const handle = edge.sourceHandle;
    if (typeOf(edge.source) !== 'bucket-assign' || !isBranchHandle(handle)) return branches;
    return branches.map(set => new Map(set).set(edge.source, handle));
  };
  const places = new Map<string, Set<Place>>();

  for (const id of order) {
    const node = byId.get(id)!;
    const type = node.data.type;
    const def = getNodeDef(type);
    const params = withDefaults(def, node.data.params);
    const parents = links.filter(edge => edge.target === id);
    const children = links.filter(edge => edge.source === id);

    if (type === 'image-folder') {
      if (!String(params.path).trim()) return fail('missingInputPath', id);
      runsWhen.set(id, [new Map()]);
      places.set(id, new Set(['kept']));
      continue;
    }
    if (type === 'output-folder' && !String(params.path).trim()) return fail('missingOutputPath', id);
    if (type === 'llm-tagger' && !String(params.model_name).trim()) return fail('missingModel', id);
    if (parents.length === 0) return fail('noInput', id);

    const bySource = new Map<string, Branches[]>();
    for (const edge of parents) bySource.set(edge.source, [...(bySource.get(edge.source) ?? []), ...viaEdge(edge)]);
    const upstreams = [...bySource.values()];
    for (let i = 0; i < upstreams.length; i++) {
      for (let j = i + 1; j < upstreams.length; j++) {
        if (upstreams[i].some(a => upstreams[j].some(b => compatible(a, b)))) return fail('multipleInputs', id);
      }
    }
    runsWhen.set(id, dedupe(upstreams.flat()));

    let place: Set<Place>;
    if (type === 'output-folder') {
      place = new Set(['kept']);
    } else if (type === 'bucket-assign' || !def?.buildOptions || isInPlaceNode(def, params)) {
      place = new Set([...bySource.keys()].flatMap(source => [...(places.get(source) ?? [])]));
    } else {
      // 下游接了输出文件夹时直接写进去，否则写进 .workflow_temp
      place = new Set([children.some(edge => typeOf(edge.target) === 'output-folder') ? 'kept' : 'temp']);
    }
    places.set(id, place);

    if (place.has('temp')) {
      const ends = type === 'bucket-assign'
        ? BRANCH_HANDLES.some(handle => !children.some(edge => edge.sourceHandle === handle || !isBranchHandle(edge.sourceHandle)))
        : children.length === 0;
      if (ends) return fail('needsOutput', id);
    }
  }

  return { order, issue: null };
}
