// ═══════════════ 工作流文件（.purin）读写 ═══════════════
import type { Node, Edge } from '@xyflow/react';
import type { WorkflowData, WorkflowNodeData } from './workflowTypes';
import { fillBlankPrompts, getNodeDef, withDefaults } from './nodeDefinitions';

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const toHandle = (value: unknown) => (typeof value === 'string' ? value : undefined);

/** 画布 → .purin 文件内容 */
export function serializeWorkflow(name: string, nodes: Node<WorkflowNodeData>[], edges: Edge[]): string {
  const data: WorkflowData = {
    version: 1,
    name,
    nodes: nodes.map(n => ({
      id: n.id, type: n.data.type, position: n.position,
      data: { type: n.data.type, params: withDefaults(getNodeDef(n.data.type), n.data.params), status: 'idle' },
    })),
    edges: edges.map(e => ({
      id: e.id, source: e.source, target: e.target, sourceHandle: toHandle(e.sourceHandle), targetHandle: toHandle(e.targetHandle),
    })),
  };
  return JSON.stringify(data);
}

/**
 * .purin 文件内容 → 画布节点与连线；不是工作流结构时返回 null。
 * 保存过的参数照常生效，缺的项取默认值，VLM 节点留空的提示词换成默认提示词；端点不存在的连线丢弃。
 */
export function parseWorkflow(json: string): { nodes: Node<WorkflowNodeData>[]; edges: Edge[] } | null {
  let data: unknown;
  try {
    data = JSON.parse(json);
  } catch {
    return null;
  }
  if (!isRecord(data) || !Array.isArray(data.nodes) || !Array.isArray(data.edges)) return null;

  const nodes: Node<WorkflowNodeData>[] = [];
  for (const raw of data.nodes) {
    if (!isRecord(raw) || typeof raw.id !== 'string' || !raw.id) return null;
    const { position, data: nodeData } = raw;
    if (!isRecord(position) || !Number.isFinite(position.x) || !Number.isFinite(position.y)) return null;
    if (!isRecord(nodeData) || typeof nodeData.type !== 'string' || !nodeData.type) return null;
    const params = isRecord(nodeData.params) ? nodeData.params : {};
    nodes.push({
      id: raw.id,
      type: 'baseNode',
      position: { x: position.x as number, y: position.y as number },
      data: { type: nodeData.type, params: fillBlankPrompts(nodeData.type, withDefaults(getNodeDef(nodeData.type), params)), status: 'idle' },
    });
  }
  const ids = new Set(nodes.map(n => n.id));
  if (ids.size !== nodes.length) return null;

  const edges: Edge[] = [];
  for (const raw of data.edges) {
    if (!isRecord(raw) || typeof raw.id !== 'string' || typeof raw.source !== 'string' || typeof raw.target !== 'string') return null;
    if (!ids.has(raw.source) || !ids.has(raw.target)) continue;
    edges.push({
      id: raw.id, source: raw.source, target: raw.target,
      sourceHandle: toHandle(raw.sourceHandle), targetHandle: toHandle(raw.targetHandle),
    });
  }
  return { nodes, edges };
}
