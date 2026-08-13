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
      await invoke('cleanup_workflow_temp', { dir: root });
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
          recursive: false,
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
          recursive: false,
        },
      };

    case 'flip':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          direction: params.direction || 'horizontal',
          recursive: false,
        },
      };

    case 'format-convert':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          target_format: params.target_format || 'png',
          recursive: false,
        },
      };

    case 'alpha-convert':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          background: params.background || 'white',
          recursive: false,
        },
      };

    case 'blur-noise':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          blur_radius: params.blur_radius || 0,
          noise_strength: params.noise_strength || 0,
          recursive: false,
        },
      };

    case 'perspective':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          intensity: params.intensity || 0.1,
          recursive: false,
        },
      };

    case 'upscale':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          engine_id: params.engine_id || 'realcugan',
          model_id: params.model_id || 'models-se',
          scale: Number(params.scale) || 2,
          denoise_level: params.denoise_level ?? -1,
          tta: params.tta || false,
          gpu_id: 0,
          tile_size: 0,
          recursive: false,
        },
      };

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
          recursive: false,
        },
      };

    case 'aesthetic':
      return {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          use_gpu: true,
          move_files: true,
          batch_size: params.batch_size ?? 1,
          recursive: false,
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
          recursive: false,
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
          recursive: false,
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
          recursive: false,
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
          recursive: false,
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
    callbacks: ExecutionCallbacks,
  ): Promise<void> {
    if (this.status === 'running') return;
    this.status = 'running';
    this.cancelFlag = false;
    const startTime = Date.now();

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
      await cleanupWorkflowTemp(nodes);

      // 1. 拓扑排序
      const sortedIds = topologicalSort(nodes, edges);
      const nodeMap = new Map(nodes.map(n => [n.id, n]));
      const totalSteps = sortedIds.length;

      // 用于跟踪每个节点的输出目录（作为下游的输入）
      const nodeOutputs = new Map<string, string>();
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
                recursive: false,
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
              const queue = [...startIds];
              while (queue.length > 0) {
                const id = queue.shift()!;
                if (skippedNodes.has(id)) continue;
                skippedNodes.add(id);
                // 继续标记该节点的所有下游
                edges.filter(e => e.source === id).forEach(e => queue.push(e.target));
              }
            };
            markSkipped(inactiveEdges.map(e => e.target));

            // 输出路径传递给活跃分支
            nodeOutputs.set(nodeId, inputPath);

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
        let inputPath = '';
        if (parents.length > 0) {
          inputPath = nodeOutputs.get(parents[0]) || '';
        }
        if (!inputPath) {
          callbacks.onNodeStatusChange(nodeId, 'error', '无输入');
          callbacks.onError(nodeId, '该节点没有输入路径（需要连接上游节点）');
          this.status = 'error';
          return;
        }

        // 确定输出路径
        // 检查下游是否有 output-folder 节点
        const children = edges.filter(e => e.source === nodeId).map(e => e.target);
        let outputPath = '';
        
        // 如果最后的下游是 output-folder，用它的路径
        const lastChild = children.length > 0 ? nodeMap.get(children[0]) : null;
        if (lastChild?.data.type === 'output-folder' && lastChild.data.params.path) {
          outputPath = lastChild.data.params.path as string;
        }

        // 否则使用临时目录
        if (!outputPath) {
          outputPath = buildTempOutputPath(inputPath, i, data.type);
        }

        const cmdOptions = buildCommandOptions(node, inputPath, outputPath);
        if (!cmdOptions) {
          callbacks.onNodeStatusChange(nodeId, 'error', '不支持的命令');
          callbacks.onError(nodeId, `不支持的节点类型: ${data.type}`);
          this.status = 'error';
          return;
        }

        try {
          await startProgressListener(data.type);
          await invoke(def.tauriCommand, cmdOptions);
          stopProgressListener();
          // 原地操作节点（打标/重命名）不产出新目录，输出即输入，需透传给下游，
          // 否则下游会拿到一个从未被创建的临时目录路径而报「输入路径无效」
          nodeOutputs.set(nodeId, IN_PLACE_NODE_TYPES.has(data.type) ? inputPath : outputPath);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
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
      const shouldCleanup =
        this.status !== 'done' || nodes.some(n => n.data.type === 'output-folder');
      if (shouldCleanup) {
        await cleanupWorkflowTemp(nodes);
      }
    }
  }
}
