// ═══════════════ 工作流执行引擎 ═══════════════
import { invoke } from '@tauri-apps/api/core';
// 统一走 tauriRuntime 封装（浏览器模式下为 noop），不再直接依赖官方 event API
import { listen } from '../../utils/tauriRuntime';
type UnlistenFn = () => void;
import type { Node, Edge } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import { getNodeDef } from './nodeDefinitions';

export type ExecutionStatus = 'idle' | 'running' | 'done' | 'error' | 'cancelled';

export interface ExecutionCallbacks {
  onNodeStatusChange: (nodeId: string, status: WorkflowNodeData['status'], message?: string) => void;
  onStepStart: (nodeId: string, stepIndex: number, totalSteps: number) => void;
  onStepDone: (nodeId: string, stepIndex: number, totalSteps: number) => void;
  onComplete: (elapsed: number) => void;
  onError: (nodeId: string, error: string) => void;
  onProgress?: (nodeId: string, current: number, total: number) => void;
}

/**
 * 拓扑排序：从无入度节点开始 BFS
 * 返回按执行顺序排列的节点 ID 列表
 */
function topologicalSort(nodes: Node<WorkflowNodeData>[], edges: Edge[]): string[] {
  const adj = new Map<string, string[]>();
  const inDegree = new Map<string, number>();

  for (const node of nodes) {
    adj.set(node.id, []);
    inDegree.set(node.id, 0);
  }
  for (const edge of edges) {
    adj.get(edge.source)?.push(edge.target);
    inDegree.set(edge.target, (inDegree.get(edge.target) || 0) + 1);
  }

  // BFS 从入度为 0 的节点开始
  const queue: string[] = [];
  for (const [id, deg] of inDegree) {
    if (deg === 0) queue.push(id);
  }

  const sorted: string[] = [];
  while (queue.length > 0) {
    const current = queue.shift()!;
    sorted.push(current);
    for (const next of adj.get(current) || []) {
      const newDeg = (inDegree.get(next) || 1) - 1;
      inDegree.set(next, newDeg);
      if (newDeg === 0) queue.push(next);
    }
  }

  if (sorted.length !== nodes.length) {
    throw new Error('工作流中存在循环依赖');
  }

  return sorted;
}

/**
 * 获取节点的上游节点（父节点）列表
 */
function getParentNodes(nodeId: string, edges: Edge[]): string[] {
  return edges.filter(e => e.target === nodeId).map(e => e.source);
}

/**
 * 原地操作节点：直接修改输入目录内的文件（重命名、写标签文件），
 * 不会产出新的输出目录。这类节点的输出路径必须透传输入路径。
 */
const IN_PLACE_NODE_TYPES = new Set(['tagger', 'llm-tagger', 'rename']);

/// 输出目录只搬图片、需要顺带携带同名标签文件的图像处理节点
const SIDECAR_CARRY_NODE_TYPES = new Set([
  'scale', 'crop', 'flip', 'format-convert', 'alpha-convert',
  'blur-noise', 'perspective', 'upscale', 'filter',
]);

// 运行代号：取消后旧 execute 仍在收尾（当前节点要跑完才返回），
// 其迟到的回调与 finally 清理不得影响紧接着启动的新一轮运行
let globalRunSeq = 0;

const TEMP_DIR_NAME = '.workflow_temp';

/**
 * 推导临时目录的根（.workflow_temp 所在的目录），分隔符无关。
 * 创建（buildTempOutputPath）与清理（cleanup_workflow_temp）必须共用此逻辑，
 * 否则清理会指向一个不存在的路径而静默什么都不删。
 */
function resolveTempRoot(inputPath: string): string {
  // 统一按两种分隔符处理，末尾分隔符先剥掉
  let rootDir = inputPath.replace(/[/\\]+$/, '');

  // 若输入已位于临时目录内，回退到临时目录的父级，避免层层嵌套
  const tempIdx = rootDir.search(new RegExp(`[/\\\\]${TEMP_DIR_NAME.replace('.', '\\.')}`));
  if (tempIdx !== -1) {
    rootDir = rootDir.substring(0, tempIdx);
  } else {
    // 首次：取输入路径的父目录
    const lastSep = Math.max(rootDir.lastIndexOf('/'), rootDir.lastIndexOf('\\'));
    if (lastSep > 0) {
      rootDir = rootDir.substring(0, lastSep);
    }
    // lastSep <= 0 时（如 "/data" 的父级为根、或 "D:" 这类盘符）保持原样不再截取，
    // 避免 substring(0,-1) 得到空串而让临时目录落到文件系统根
  }

  // 去掉可能残留的尾部分隔符（如输入本身就是盘根 "C:\"），避免拼出 "C:\\"
  return rootDir.replace(/[/\\]+$/, '');
}

/**
 * 生成节点的中间产物临时目录，分隔符无关（兼容 Windows 的 \ 与 POSIX 的 /）
 */
function buildTempOutputPath(inputPath: string, stepIndex: number, nodeType: string): string {
  const base = resolveTempRoot(inputPath);
  // 沿用原始输入路径的分隔符风格（base 可能已被截断到不含分隔符，如 "D:"）
  const sep = inputPath.includes('\\') && !inputPath.includes('/') ? '\\' : '/';
  return `${base}${sep}${TEMP_DIR_NAME}${sep}step_${stepIndex}_${nodeType}`;
}

/**
 * 清理工作流临时目录。temp 根按每个输入节点的路径推导（与创建逻辑一致），
 * 多个输入节点可能散落在不同父目录，逐一去重清理。
 */
async function cleanupWorkflowTemp(nodes: Node<WorkflowNodeData>[]): Promise<void> {
  const roots = new Set<string>();
  for (const node of nodes) {
    if (node.data.type !== 'image-folder') continue;
    const path = node.data.params.path as string | undefined;
    if (path) roots.add(resolveTempRoot(path));
  }
  for (const root of roots) {
    try {
      // 裸盘符 "J:" 传给 Rust 的 Path::join 会变成盘符相对路径 J:.workflow_temp，补回根分隔符
      await invoke('cleanup_workflow_temp', { dir: /^[A-Za-z]:$/.test(root) ? root + '\\' : root });
    } catch (e) {
      // 清理失败不影响主流程
      console.warn('清理工作流临时目录失败:', root, e);
    }
  }
}

/**
 * 构建节点的 Tauri 命令调用参数
 * 根据节点类型，拼装 options 对象
 */
function buildCommandOptions(
  node: Node<WorkflowNodeData>,
  inputPath: string,
  outputPath: string,
  // 上游是美学评分时文件散落在 <label>/ 子目录里，必须递归收集，否则下游静默产出空结果
  recursiveInput = false,
): Record<string, any> | null {
  const data = node.data;
  const def = getNodeDef(data.type);
  if (!def?.tauriCommand) return null;

  const params = data.params;

  switch (data.type) {
    case 'scale':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode: params.mode || 'upscale',
          target_width: params.width || 1024,
          target_height: params.height || 1024,
          down_target_width: 0,
          down_target_height: 0,
          recursive: recursiveInput,
        },
      };

    case 'crop':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          mode: params.mode || 'center',
          crop_anchor: 'center',
          target_width: params.width || 1024,
          target_height: params.height || 1024,
          aspect_ratio: 1.0,
          crop_top: 0, crop_bottom: 0, crop_left: 0, crop_right: 0,
          recursive: recursiveInput,
        },
      };

    case 'flip':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          direction: params.direction || 'horizontal',
          recursive: recursiveInput,
        },
      };

    case 'format-convert':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          target_format: params.target_format || 'png',
          recursive: recursiveInput,
        },
      };

    case 'alpha-convert':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          background: params.background || 'white',
          recursive: recursiveInput,
        },
      };

    case 'blur-noise':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          blur_radius: params.blur_radius || 0,
          noise_strength: params.noise_strength || 0,
          recursive: recursiveInput,
        },
      };

    case 'perspective':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          intensity: params.intensity || 0.1,
          recursive: recursiveInput,
        },
      };

    case 'upscale': {
      const engineId = params.engine_id || 'realcugan';
      // 默认模型必须与引擎匹配：realesrgan 的 Python 脚本对未知模型 id 直接报错退出
      const defaultModel = engineId === 'realesrgan' ? 'realesrgan-x4plus'
        : engineId === 'waifu2x' ? 'models-cunet'
        : 'models-se';
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          engine_id: engineId,
          model_id: params.model_id || defaultModel,
          scale: Number(params.scale) || 2,
          denoise_level: params.denoise_level ?? -1,
          tta: params.tta || false,
          gpu_id: 0,
          tile_size: 0,
          recursive: recursiveInput,
        },
      };
    }

    case 'person-crop':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          use_gpu: true,
          person_enabled: params.person_enabled ?? true,
          person_conf: params.person_conf ?? 0.3,
          upper_enabled: params.upper_enabled ?? false,
          upper_conf: params.upper_conf ?? 0.3,
          upper_tag: params.upper_tag || '',
          head_enabled: params.head_enabled ?? false,
          head_conf: params.head_conf ?? 0.3,
          head_tag: params.head_tag || '',
          head_scale: params.head_scale ?? 1.5,
          eyes_enabled: params.eyes_enabled ?? false,
          eyes_conf: params.eyes_conf ?? 0.3,
          eyes_tag: params.eyes_tag || '',
          eyes_scale: params.eyes_scale ?? 2.0,
          keep_original_tags: params.keep_original_tags ?? true,
          recursive: recursiveInput,
        },
      };

    case 'aesthetic':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          use_gpu: true,
          // 工作流内必须复制而非移动：输出多半是 .workflow_temp，
          // 移动会让原图唯一副本落在临时目录里，清理临时目录=删数据集
          move_files: true,
          copy_files: true,
          batch_size: params.batch_size ?? 1,
          recursive: recursiveInput,
        },
      };

    case 'tagger': {
      const cats = ['general', 'character', 'rating', 'artist', 'copyright', 'meta'];
      const enabledCats = cats.filter(c => params[`cat_${c}`] ?? (c === 'general' || c === 'character'));
      return {
        options: {
          input_path: inputPath,
          model_id: params.model_id || 'wd-swinv2-tagger-v3',
          general_threshold: params.general_threshold ?? 0.35,
          character_threshold: params.character_threshold ?? 0.85,
          enabled_categories: enabledCats,
          use_gpu: true,
          exclude_tags: params.exclude_tags || '',
          append_tags: params.append_tags || '',
          append_position: params.append_position || 'append',
          replace_underscore: params.replace_underscore ?? true,
          output_format: (params.output_format === 'json_simplified') ? 'json' : (params.output_format || 'txt'),
          json_simplified: params.output_format === 'json_simplified',
          escape_parentheses: params.escape_parentheses ?? false,
          sort_by: params.sort_by || 'confidence',
          existing_tags_action: params.existing_tags_action || 'overwrite',
          batch_size: params.batch_size ?? 1,
          recursive: recursiveInput,
        },
      };
    }

    case 'llm-tagger':
      return {
        options: {
          input_path: inputPath,
          api_endpoint: params.api_endpoint || '',
          api_key: params.api_key || '',
          model_name: params.model_name || '',
          system_prompt: params.system_prompt || '',
          user_prompt: params.user_prompt || '',
          temperature: params.temperature ?? 0.7,
          max_tokens: params.max_tokens ?? -1,
          image_size: params.image_size ?? 1024,
          top_p: 0.9,
          skip_existing: params.skip_existing ?? false,
          output_format: (params.output_format === 'json_simplified') ? 'json' : (params.output_format || 'txt'),
          json_simplified: params.output_format === 'json_simplified',
          request_interval_ms: -1,
          concurrency: params.concurrency ?? 1,
          recursive: recursiveInput,
        },
      };

    case 'filter':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          action: params.action || 'copy',
          condition: params.condition || 'below_resolution',
          width: params.width || 512,
          height: params.height || 512,
          recursive: recursiveInput,
        },
      };

    case 'rename':
      return {
        options: {
          input_path: inputPath,
          prefix: params.prefix || 'img_',
          start_number: params.start_number ?? 1,
          digit_count: params.digit_count ?? 4,
          shuffle: params.shuffle ?? false,
          rename_tags: params.rename_tags ?? true,
        },
      };

    case 'bucket-assign':
      return {
        options: {
          input_path: inputPath,
          res_width: params.res_width || 1024,
          res_height: params.res_height || 1024,
          steps: Number(params.steps) || 64,
          no_upscale: params.no_upscale ?? false,
          recursive: recursiveInput,
        },
      };

    default:
      return null;
  }
}

/**
 * 工作流执行引擎
 * 顺序执行拓扑排序后的节点，通过临时目录传递中间文件
 */
export class WorkflowEngine {
  private cancelFlag = false;
  /// 成功时是否保留临时目录（有链条终点的产物留在里面）
  private keepTempOnSuccess = false;
  private status: ExecutionStatus = 'idle';
  private unlisteners: UnlistenFn[] = [];
  private currentNodeId = '';
  /** 当前正在执行的节点类型，用于取消时定位对应的 Rust 取消命令 */
  private currentNodeType = '';

  getStatus() { return this.status; }

  /**
   * 取消工作流。
   * 仅置本地标志不足以停止正在运行的节点——Rust 侧的处理循环和 Python
   * 子进程都不知情，会继续跑完当前节点。必须同时调用该节点的取消命令。
   */
  cancel() {
    this.cancelFlag = true;

    // 终止当前节点正在执行的后端任务（含其 Python 子进程树）
    if (this.currentNodeType) {
      const cancelCommand = getNodeDef(this.currentNodeType)?.cancelCommand;
      if (cancelCommand) {
        invoke(cancelCommand).catch((e) => {
          console.warn(`取消命令 ${cancelCommand} 调用失败:`, e);
        });
      }
    }
  }

  async execute(
    nodes: Node<WorkflowNodeData>[],
    edges: Edge[],
    rawCallbacks: ExecutionCallbacks,
  ): Promise<void> {
    if (this.status === 'running') return;
    const runSeq = ++globalRunSeq;
    const live = () => runSeq === globalRunSeq;
    const callbacks: ExecutionCallbacks = {
      onNodeStatusChange: (...a) => { if (live()) rawCallbacks.onNodeStatusChange(...a); },
      onStepStart: (...a) => { if (live()) rawCallbacks.onStepStart(...a); },
      onStepDone: (...a) => { if (live()) rawCallbacks.onStepDone(...a); },
      onComplete: (...a) => { if (live()) rawCallbacks.onComplete(...a); },
      onError: (...a) => { if (live()) rawCallbacks.onError(...a); },
      onProgress: (...a) => { if (live()) rawCallbacks.onProgress?.(...a); },
    };
    this.status = 'running';
    this.cancelFlag = false;
    const startTime = Date.now();
    // 为 true 时 finally 跳过清理。目前唯一场景：输入位于 .workflow_temp 内被拒跑——
    // 拒跑就是为了保护它，finally 若照常清理等于把用户数据删了
    let preserveTempOnExit = false;

    // 当前节点的进度监听器（每个步骤动态切换）
    let currentProgressUnlisten: UnlistenFn | null = null;
    const stopProgressListener = () => {
      if (currentProgressUnlisten) {
        currentProgressUnlisten();
        currentProgressUnlisten = null;
      }
    };

    try {
      // 0. 清理上一次运行的中间产物。
      // 临时目录名按 step_{序号}_{类型} 生成，多次运行之间完全相同，
      // 若上次被取消/失败留下了残留文件，本次会把它们当成上游产物读进来。
      // 输入目录若位于 .workflow_temp 内（上次无输出节点的运行把成品留在那里），
      // 下面的残留清理会连输入一起删掉——直接拒跑并提示搬出
      const tempPathRe = /[\\/]\.workflow_temp([\\/]|$)/;
      const inputInTemp = nodes.find(
        n => n.data.type === 'image-folder' && tempPathRe.test(String(n.data.params.path || '')),
      );
      if (inputInTemp) {
        preserveTempOnExit = true;
        callbacks.onNodeStatusChange(inputInTemp.id, 'error', '输入位于临时目录内');
        callbacks.onError(
          inputInTemp.id,
          '输入目录位于 .workflow_temp 内，运行前的残留清理会把它删除；请先把上次产物移动到正式目录再作为输入',
        );
        this.status = 'error';
        return;
      }

      await cleanupWorkflowTemp(nodes);

      // 1. 拓扑排序
      const sortedIds = topologicalSort(nodes, edges);
      const nodeMap = new Map(nodes.map(n => [n.id, n]));
      const totalSteps = sortedIds.length;

      // 用于跟踪每个节点的输出目录（作为下游的输入）
      const nodeOutputs = new Map<string, string>();
      // 输出目录带 <label>/ 子层级的节点（美学评分）：下游收集必须递归，且沿链传递
      const nodeNested = new Map<string, boolean>();
      // 用于跟踪被条件分支跳过的节点
      const skippedNodes = new Set<string>();

      // 2. 节点类型 → Rust 进度事件名映射
      const startProgressListener = async (nodeType: string) => {
        // 先清理上一个
        stopProgressListener();
        // 进度事件名跟随节点定义（nodeDefinitions.ts 的 progressEvent），不再单独维护映射表
        const eventName = getNodeDef(nodeType)?.progressEvent;
        if (!eventName || !callbacks.onProgress) return;

        currentProgressUnlisten = await listen<{ current: number; total: number }>(eventName, (event) => {
          if (this.status === 'running' && this.currentNodeId) {
            callbacks.onProgress!(this.currentNodeId, event.payload.current, event.payload.total);
          }
        });
      };

      // 3. 标记所有节点为等待
      for (const id of sortedIds) {
        callbacks.onNodeStatusChange(id, 'waiting');
      }

      // 4. 依次执行每个节点
      for (let i = 0; i < sortedIds.length; i++) {
        if (this.cancelFlag) {
          this.status = 'cancelled';
          for (let j = i; j < sortedIds.length; j++) {
            callbacks.onNodeStatusChange(sortedIds[j], 'idle');
          }
          return;
        }

        const nodeId = sortedIds[i];
        const node = nodeMap.get(nodeId)!;
        const data = node.data;
        const def = getNodeDef(data.type);

        this.currentNodeId = nodeId;
        this.currentNodeType = data.type;
        callbacks.onStepStart(nodeId, i, totalSteps);
        callbacks.onNodeStatusChange(nodeId, 'running', `执行中 (${i + 1}/${totalSteps})`);

        // ── 输入节点：直接使用用户指定的路径 ──
        if (data.type === 'image-folder') {
          const folderPath = data.params.path as string;
          if (!folderPath) {
            callbacks.onNodeStatusChange(nodeId, 'error', '未指定输入路径');
            callbacks.onError(nodeId, '输入路径为空');
            this.status = 'error';
            return;
          }
          nodeOutputs.set(nodeId, folderPath);
          // 递归扫描开关此前从未被消费：嵌套数据集（10_charA/ 等）会收集到 0 张图
          nodeNested.set(nodeId, !!data.params.recursive);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          callbacks.onStepDone(nodeId, i, totalSteps);
          continue;
        }

        // ── 输出节点：标记完成 ──
        if (data.type === 'output-folder') {
          const outputPath = data.params.path as string;
          if (outputPath) {
            nodeOutputs.set(nodeId, outputPath);
          }
          // nested 标记要随路径一起透传，否则"美学→输出文件夹→打标"这类链会丢递归
          const ofParents = getParentNodes(nodeId, edges);
          nodeNested.set(nodeId, ofParents.length > 0 ? (nodeNested.get(ofParents[0]) ?? false) : false);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          callbacks.onStepDone(nodeId, i, totalSteps);
          continue;
        }

        // ── 被条件分支跳过的节点 ──
        if (skippedNodes.has(nodeId)) {
          callbacks.onNodeStatusChange(nodeId, 'idle', '⏭ skipped');
          callbacks.onStepDone(nodeId, i, totalSteps);
          continue;
        }

        // ── 分桶条件分支节点 ──
        if (data.type === 'bucket-assign') {
          const parents = getParentNodes(nodeId, edges);
          const inputPath = parents.length > 0 ? (nodeOutputs.get(parents[0]) || '') : '';
          const baNested = parents.length > 0 ? (nodeNested.get(parents[0]) ?? false) : false;
          if (!inputPath) {
            callbacks.onNodeStatusChange(nodeId, 'error', '无输入');
            callbacks.onError(nodeId, '该节点没有输入路径');
            this.status = 'error';
            return;
          }

          // 调用 analyze_buckets 获取分桶结果
          try {
            const result = await invoke<{
              bucket_count: number;
              buckets: { bucket_width: number; bucket_height: number; image_count: number }[];
            }>('analyze_buckets', {
              options: {
                input_path: inputPath,
                res_width: data.params.res_width || 1024,
                res_height: data.params.res_height || 1024,
                steps: Number(data.params.steps) || 64,
                no_upscale: data.params.no_upscale ?? false,
                recursive: baNested,
              },
            });

            const buckets = result.buckets || [];
            const totalImages = buckets.reduce((s, b) => s + b.image_count, 0);

            // 找最大桶
            let maxBucket = buckets[0];
            for (const b of buckets) {
              if (b.image_count > maxBucket.image_count) maxBucket = b;
            }

            // 均匀度判断
            const uniformThreshold = (data.params.uniform_threshold ?? 70) / 100;
            const maxOutlierBuckets = data.params.max_outlier_buckets ?? 2;
            const maxBucketRatio = totalImages > 0 ? maxBucket.image_count / totalImages : 1;
            const outlierCount = buckets.filter(b => b !== maxBucket && b.image_count > 0).length;

            const isUniform = maxBucketRatio >= uniformThreshold || outlierCount <= maxOutlierBuckets;

            // 确定活跃分支，标记非活跃分支的所有下游为 skipped
            const inactiveHandle = isUniform ? 'output-b' : 'output-a';

            // 收集非活跃分支的下游节点
            const inactiveEdges = edges.filter(e => e.source === nodeId && e.sourceHandle === inactiveHandle);
            const markSkipped = (startIds: string[]) => {
              // 直接目标无条件跳过；再往下游只有"全部父节点都被跳过"的节点才跳过——
              // 两条分支汇合的节点仍由活跃分支供给，绝不能连带跳过
              const seeds = new Set(startIds);
              const queue = [...startIds];
              while (queue.length > 0) {
                const id = queue.shift()!;
                if (skippedNodes.has(id)) continue;
                if (!seeds.has(id)) {
                  const parentIds = edges.filter(e => e.target === id).map(e => e.source);
                  const allParentsSkipped =
                    parentIds.length > 0 && parentIds.every(pid => skippedNodes.has(pid));
                  if (!allParentsSkipped) continue;
                }
                skippedNodes.add(id);
                edges.filter(e => e.source === id).forEach(e => queue.push(e.target));
              }
            };
            markSkipped(inactiveEdges.map(e => e.target));

            // 输出路径传递给活跃分支（nested 标记同样要透传，否则美学下游在此断链）
            nodeOutputs.set(nodeId, inputPath);
            nodeNested.set(nodeId, baNested);

            const branchLabel = isUniform ? 'A (均匀)' : 'B (分散)';
            const info = `→ ${branchLabel} | 桶${buckets.length}个 | 最大桶${maxBucket.image_count}/${totalImages}张 (${(maxBucketRatio * 100).toFixed(0)}%)`;
            callbacks.onNodeStatusChange(nodeId, 'done', info);
          } catch (e: any) {
            const errorMsg = typeof e === 'string' ? e : e?.message || '未知错误';
            callbacks.onNodeStatusChange(nodeId, 'error', errorMsg);
            callbacks.onError(nodeId, errorMsg);
            this.status = 'error';
            return;
          }

          callbacks.onStepDone(nodeId, i, totalSteps);
          continue;
        }

        // ── resize-strategy（兼容旧工作流）──
        if (data.type === 'resize-strategy') {
          const parents = getParentNodes(nodeId, edges);
          if (parents.length > 0) {
            nodeOutputs.set(nodeId, nodeOutputs.get(parents[0]) || '');
            nodeNested.set(nodeId, nodeNested.get(parents[0]) ?? false);
          }
          callbacks.onNodeStatusChange(nodeId, 'done', '✓ (passthrough)');
          callbacks.onStepDone(nodeId, i, totalSteps);
          continue;
        }

        // ── 处理节点：调用 Tauri 命令 ──
        if (!def?.tauriCommand) {
          callbacks.onNodeStatusChange(nodeId, 'done', '✓ (no-op)');
          callbacks.onStepDone(nodeId, i, totalSteps);
          continue;
        }

        // 确定输入路径（从上游节点获取）
        const parents = getParentNodes(nodeId, edges);
        // 多条入边时只认"实际执行并产出输出"的上游：分桶 A/B 分支的汇合节点
        // 天然有两条入边，但被跳过的分支不会写 nodeOutputs——那不算多输入。
        // 仍有多个活跃上游才是真正不支持的场景（会静默丢弃其余输入），明确报错
        const activeParents = parents.filter(pid => nodeOutputs.has(pid));
        if (activeParents.length > 1) {
          callbacks.onNodeStatusChange(nodeId, 'error', '多个上游输入');
          callbacks.onError(nodeId, '该节点有多个实际产出的上游输入，目前只支持单输入（多余的会被静默丢弃），请合并为一条链');
          this.status = 'error';
          return;
        }
        let inputPath = '';
        if (activeParents.length > 0) {
          inputPath = nodeOutputs.get(activeParents[0]) || '';
        }
        const inputNested = activeParents.length > 0 ? (nodeNested.get(activeParents[0]) ?? false) : false;
        // 重命名后端只扫顶层（无递归支持），嵌套输入必然找到 0 张图——给出可操作的报错
        if (inputNested && data.type === 'rename') {
          callbacks.onNodeStatusChange(nodeId, 'error', '不支持嵌套输入');
          callbacks.onError(nodeId, '重命名节点不支持递归处理子目录（上游输出按分类分层或开启了递归扫描）——请调整链路');
          this.status = 'error';
          return;
        }
        if (!inputPath) {
          callbacks.onNodeStatusChange(nodeId, 'error', '无输入');
          callbacks.onError(nodeId, '该节点没有输入路径（需要连接上游节点）');
          this.status = 'error';
          return;
        }

        // 确定输出路径
        // 在全部下游里找 output-folder（不能只看第一条边——先画的边不一定是它）
        const children = edges.filter(e => e.source === nodeId).map(e => e.target);
        let outputPath = '';

        const outFolder = children
          .map(cid => nodeMap.get(cid))
          .find(c => c?.data.type === 'output-folder');
        if (outFolder) {
          const outFolderPath = outFolder.data.params.path as string;
          if (!outFolderPath) {
            // 空路径若静默跳过，成品会写进临时目录并在成功清理时被删
            callbacks.onNodeStatusChange(outFolder.id, 'error', '未设置路径');
            callbacks.onError(outFolder.id, '输出文件夹节点未设置路径');
            this.status = 'error';
            return;
          }
          outputPath = outFolderPath;
        }

        // 否则使用临时目录
        if (!outputPath) {
          outputPath = buildTempOutputPath(inputPath, i, data.type);
        }

        const cmdOptions = buildCommandOptions(node, inputPath, outputPath, inputNested);
        if (!cmdOptions) {
          callbacks.onNodeStatusChange(nodeId, 'error', '不支持的命令');
          callbacks.onError(nodeId, `不支持的节点类型: ${data.type}`);
          this.status = 'error';
          return;
        }

        try {
          // 节点边界取消竞态：cancel_* 可能赶在 start_* 之前落地并被其入口复位，
          // 进入 invoke 前最后确认一次本地取消标志
          if (this.cancelFlag) {
            this.status = 'cancelled';
            callbacks.onNodeStatusChange(nodeId, 'idle', '已取消');
            return;
          }
          await startProgressListener(data.type);
          const result = await invoke<unknown>(def.tauriCommand, cmdOptions);
          stopProgressListener();

          // ProcessResult 不能丢弃：全部失败/输入为空时若照样标 ✓，
          // 空目录会沿链传下去，最后显示"运行完成"却什么都没有
          let doneMsg = '✓';
          const r = result as { success_count?: number; fail_count?: number; total?: number } | null;
          if (r && typeof r === 'object' && typeof r.success_count === 'number' && typeof r.total === 'number') {
            if (r.total === 0 || r.success_count === 0) {
              // 取消会让命令带着 0 成功正常返回——按取消收尾，不能报成错误
              if (this.cancelFlag) {
                this.status = 'cancelled';
                callbacks.onNodeStatusChange(nodeId, 'idle', '已取消');
                return;
              }
              // 过滤节点的 success 只计"匹配"数：0 匹配是正常的空跑（delete 模式尤其如此），
              // 不能中止整条链；copy 模式的空产物会在下游以清晰的 total=0 报出
              if (data.type !== 'filter') {
                throw `没有任何文件处理成功（成功 ${r.success_count}/${r.total}），已中止后续节点`;
              }
            }
            if ((r.fail_count ?? 0) > 0) doneMsg = `✓ (${r.fail_count} 个失败)`;
          }

          // filter 的 delete 模式是就地删除、不产出输出目录，必须按就地节点透传输入
          const isInPlaceNode = IN_PLACE_NODE_TYPES.has(data.type)
            || (data.type === 'filter' && (data.params.action || 'copy') === 'delete');

          // 图像节点只搬图片：把上游同名 .txt/.json/.caption 一起带上，
          // 否则"打标在前、图像处理在后"的链会把标签留在临时目录里随清理丢失
          if (SIDECAR_CARRY_NODE_TYPES.has(data.type) && !isInPlaceNode) {
            try {
              await invoke('carry_tag_sidecars', { inputPath, outputPath, recursive: inputNested });
            } catch (e) {
              console.warn('携带标签文件失败:', e);
            }
          }

          // 原地操作节点（打标/重命名/过滤删除）不产出新目录，输出即输入，需透传给下游，
          // 否则下游会拿到一个从未被创建的临时目录路径而报「输入路径无效」
          nodeOutputs.set(nodeId, isInPlaceNode ? inputPath : outputPath);
          nodeNested.set(nodeId, data.type === 'aesthetic' ? true : inputNested);
          callbacks.onNodeStatusChange(nodeId, 'done', doneMsg);
          callbacks.onStepDone(nodeId, i, totalSteps);
        } catch (e: any) {
          stopProgressListener();
          const errorMsg = typeof e === 'string' ? e : e?.message || '未知错误';
          callbacks.onNodeStatusChange(nodeId, 'error', errorMsg);
          callbacks.onError(nodeId, errorMsg);
          this.status = 'error';
          return;
        }
      }

      // 5. 全部完成
      // 记录是否有"链条终点产物仍留在临时目录"（该链没接输出文件夹）——
      // 那些 step_N 就是用户的最终结果，成功后的清理不能删
      {
        const processedIds = new Set(nodeOutputs.keys());
        const hasDownstreamProcessing = (id: string) =>
          edges.some(
            e => e.source === id && processedIds.has(e.target)
              && nodeMap.get(e.target)?.data.type !== 'output-folder',
          );
        this.keepTempOnSuccess = [...nodeOutputs.entries()].some(
          ([id, p]) => !hasDownstreamProcessing(id) && tempPathRe.test(p),
        );
      }
      const elapsed = Date.now() - startTime;
      this.status = 'done';
      callbacks.onComplete(elapsed);

    } catch (e: any) {
      this.status = 'error';
      const errorMsg = typeof e === 'string' ? e : e?.message || '未知错误';
      callbacks.onError('', errorMsg);
    } finally {
      // 清理事件监听
      stopProgressListener();
      for (const unlisten of this.unlisteners) {
        unlisten();
      }
      this.unlisteners = [];

      // 清理中间产物临时目录。
      // 失败/取消：中间产物是无用垃圾，直接清理。
      // 成功：仅当存在 output-folder 节点时清理——否则最后一个节点的产物就在
      // 临时目录里，删掉会连用户的结果一起毁掉。
      const shouldCleanup = !preserveTempOnExit && (this.status !== 'done' || !this.keepTempOnSuccess);
      // live()：被新一轮运行取代的旧 execute 不得清理（会删掉新一轮正在读写的目录）
      if (shouldCleanup && live()) {
        await cleanupWorkflowTemp(nodes);
      }
    }
  }
}
