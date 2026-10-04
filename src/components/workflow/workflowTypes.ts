import type { CommandIO, OptionsCommandCall } from '../../api/commandOptions';

export type NodeCategory = 'input' | 'process' | 'ai' | 'tag' | 'file' | 'condition' | 'output';

export type NodeStatus = 'idle' | 'waiting' | 'running' | 'done' | 'error';

/** 节点参数 schema */
export interface ParamDef {
  key: string;
  labelKey: string;           // i18n key
  type: 'string' | 'number' | 'boolean' | 'select' | 'path' | 'dynamic-select';
  default: string | number | boolean;
  /** string 类型用多行文本框（提示词） */
  multiline?: boolean;
  options?: readonly { value: string; labelKey: string }[];  // for select type
  min?: number;
  max?: number;
  step?: number;
  /** 动态加载选项的 Tauri 命令名 (type = 'dynamic-select') */
  tauriListCommand?: string;
  /** 从返回的对象中取 value/label 的字段名 */
  optionValueKey?: string;
  optionLabelKey?: string;
  /** 仅显示满足此条件的选项 (字段名 → 值) */
  optionFilter?: { key: string; value: any };
}

/** 节点类型定义 */
export interface NodeTypeDef {
  type: string;                // unique node type id
  nameKey: string;             // i18n key for display name
  category: NodeCategory;
  icon: string;                // lucide icon name
  color: string;               // category color
  params: ParamDef[];          // configurable parameters
  hasInput: boolean;           // has input port
  hasOutput: boolean;          // has output port (A)
  hasOutputB?: boolean;        // conditional branch: second output port (B)
  inputLabelKey?: string;      // i18n key for input slot label
  outputLabelKey?: string;     // i18n key for output slot label
  outputBLabelKey?: string;    // i18n key for second output slot label
  buildOptions?: (params: Record<string, any>, io: CommandIO) => OptionsCommandCall | Promise<OptionsCommandCall>;
  inPlace?: boolean | ((params: Record<string, any>) => boolean);
  carrySidecars?: boolean;
  nestedOutput?: boolean;
  flatInputOnly?: boolean;
  allowEmptyResult?: boolean;
  cancelCommand?: string;      // 取消命令（停止工作流时用于终止该节点的子进程）
  progressEvent?: string;      // 后端进度事件名（运行该节点时桥接到画布进度显示）
}

/** 节点运行时数据 */
export interface WorkflowNodeData {
  [key: string]: unknown;      // React Flow requires Record<string, unknown>
  type: string;                // reference to NodeTypeDef.type
  params: Record<string, any>; // parameter values
  status: NodeStatus;
  statusMessage?: string;
  progressCurrent?: number;    // current progress count
  progressTotal?: number;      // total items to process
}

/** 工作流序列化格式 */
export interface WorkflowData {
  version: 1;
  name: string;
  nodes: SerializedNode[];
  edges: SerializedEdge[];
}

export interface SerializedNode {
  id: string;
  type: string;
  position: { x: number; y: number };
  data: WorkflowNodeData;
}

export interface SerializedEdge {
  id: string;
  source: string;
  target: string;
  sourceHandle?: string;
  targetHandle?: string;
}
