// ═══════════════ 工作流执行引擎 ═══════════════
import { invoke } from '@tauri-apps/api/core';
// 走 tauriRuntime 封装：浏览器模式下（无 Tauri 运行时）listen 为 noop
import { listen } from '../../utils/tauriRuntime';
type UnlistenFn = () => void;
import type { Node, Edge } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import { getNodeDef, withDefaults } from './nodeDefinitions';

type ExecutionStatus = 'idle' | 'running' | 'done' | 'error' | 'cancelled';

interface ExecutionCallbacks {
  onNodeStatusChange: (nodeId: string, status: WorkflowNodeData['status'], message?: string) => void;
  onStepStart: (nodeId: string, stepIndex: number, totalSteps: number) => void;
  onComplete: (elapsed: number) => void;
  onError: (nodeId: string, error: string) => void;
  onProgress?: (nodeId: string, current: number, total: number) => void;
}

function errMsg(e: any): string {
  return typeof e === 'string' ? e : e?.message || '未知错误';
}

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

const TEMP_DIR_NAME = '.workflow_temp';
const TEMP_PATH_RE = /[\\/]\.workflow_temp(?=[\\/]|$)/;

/**
 * 推导临时目录的根（.workflow_temp 所在的目录），分隔符无关。
 * 创建（buildTempOutputPath）与清理（cleanup_workflow_temp）必须共用此逻辑，
 * 否则清理会指向一个不存在的路径而静默什么都不删。
 */
function resolveTempRoot(inputPath: string): string {
  // 统一按两种分隔符处理，末尾分隔符先剥掉
  let rootDir = inputPath.replace(/[/\\]+$/, '');

  // 若输入已位于临时目录内，回退到临时目录的父级，避免层层嵌套
  const tempIdx = rootDir.search(TEMP_PATH_RE);
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
 * 工作流执行引擎
 * 顺序执行拓扑排序后的节点，通过临时目录传递中间文件
 */
export class WorkflowEngine {
  private cancelFlag = false;
  private status: ExecutionStatus = 'idle';
  private currentNodeId = '';
  /** 当前正在执行的节点类型，用于取消时定位对应的 Rust 取消命令 */
  private currentNodeType = '';
  private cancelPromise: Promise<unknown> = Promise.resolve();

  /**
   * 取消工作流。
   * 仅置本地标志不足以停止正在运行的节点——Rust 侧的处理循环和 Python
   * 子进程都不知情，会继续跑完当前节点。必须同时调用该节点的取消命令。
   */
  cancel() {
    if (this.cancelFlag) return;
    this.cancelFlag = true;

    // 终止当前节点正在执行的后端任务（含其 Python 子进程树）
    if (this.currentNodeType) {
      const cancelCommand = getNodeDef(this.currentNodeType)?.cancelCommand;
      if (cancelCommand) {
        this.cancelPromise = invoke(cancelCommand).catch((e) => {
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
    const fail = (id: string, short: string, detail = short) => {
      callbacks.onNodeStatusChange(id, 'error', short);
      callbacks.onError(id, detail);
      this.status = 'error';
    };
    this.status = 'running';
    const startTime = Date.now();
    // 为 true 时 finally 跳过清理。目前唯一场景：输入位于 .workflow_temp 内被拒跑——
    // 拒跑就是为了保护它，finally 若照常清理等于把用户数据删了
    let preserveTempOnExit = false;
    // 成功时是否保留临时目录（有链条终点的产物留在里面）
    let keepTempOnSuccess = false;

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
      // 临时目录名按 step_{序号}_{类型} 生成。清理残留目录，避免旧文件被当成上游产物。
      // 输入目录若位于 .workflow_temp 内（上次无输出节点的运行把成品留在那里），
      // 下面的残留清理会连输入一起删掉——直接拒跑并提示搬出
      const inputInTemp = nodes.find(
        n => n.data.type === 'image-folder' && TEMP_PATH_RE.test(String(n.data.params.path || '')),
      );
      if (inputInTemp) {
        preserveTempOnExit = true;
        fail(
          inputInTemp.id,
          '输入位于临时目录内',
          '输入目录位于 .workflow_temp 内，运行前的残留清理会把它删除；请先把上次产物移动到正式目录再作为输入',
        );
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
      const inactiveEdges = new Set<string>();

      // 2. 把当前节点的后端进度事件（节点定义的 progressEvent）转发给 onProgress
      const startProgressListener = async (nodeType: string) => {
        stopProgressListener();
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
        const def = getNodeDef(node.data.type);
        const data = { ...node.data, params: withDefaults(def, node.data.params) };

        this.currentNodeId = nodeId;
        callbacks.onStepStart(nodeId, i, totalSteps);
        callbacks.onNodeStatusChange(nodeId, 'running', `执行中 (${i + 1}/${totalSteps})`);

        // ── 输入节点：直接使用用户指定的路径 ──
        if (data.type === 'image-folder') {
          const folderPath = data.params.path as string;
          if (!folderPath) {
            fail(nodeId, '未指定输入路径', '输入路径为空');
            return;
          }
          nodeOutputs.set(nodeId, folderPath);
          // 开启递归扫描时下游也必须递归收集，否则嵌套数据集（10_charA/ 等）会收集到 0 张图
          nodeNested.set(nodeId, !!data.params.recursive);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          continue;
        }

        const parentEdges = edges.filter(edge => edge.target === nodeId);
        const activeParents = [...new Set(parentEdges
          .filter(edge => !inactiveEdges.has(edge.id) && nodeOutputs.has(edge.source))
          .map(edge => edge.source))];
        if (parentEdges.length > 0 && activeParents.length === 0) {
          callbacks.onNodeStatusChange(nodeId, 'idle');
          continue;
        }
        if (activeParents.length > 1) {
          fail(nodeId, '多个上游输入', '该节点有多个实际产出的上游输入，目前只支持单输入，请合并为一条链');
          return;
        }
        const upstream = activeParents[0];
        const inputPath = nodeOutputs.get(upstream) || '';
        const inputNested = nodeNested.get(upstream) ?? false;

        if (data.type === 'output-folder') {
          const outputPath = data.params.path as string;
          if (!outputPath || !inputPath) {
            fail(nodeId, !outputPath ? '未设置路径' : '无输入');
            return;
          }
          await invoke('carry_tag_sidecars', { inputPath, outputPath, recursive: inputNested, copyImages: true });
          if (this.cancelFlag) return;
          nodeOutputs.set(nodeId, outputPath);
          nodeNested.set(nodeId, inputNested);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          continue;
        }

        // ── 分桶条件分支节点 ──
        if (data.type === 'bucket-assign') {
          if (!inputPath) {
            fail(nodeId, '无输入', '该节点没有输入路径');
            return;
          }

          try {
            const call = await def!.buildOptions!(data.params, { input_path: inputPath, output_path: '', recursive: inputNested });
            await startProgressListener(data.type);
            if (this.cancelFlag) return;
            this.currentNodeType = data.type;
            const result = await invoke<{
              bucket_count: number;
              buckets: { bucket_width: number; bucket_height: number; image_count: number }[];
            }>(call.command, { options: call.options });
            this.currentNodeType = '';
            stopProgressListener();
            if (this.cancelFlag) return;

            const buckets = result.buckets || [];
            const totalImages = buckets.reduce((s, b) => s + b.image_count, 0);

            let maxBucket = buckets[0];
            for (const b of buckets) {
              if (b.image_count > maxBucket.image_count) maxBucket = b;
            }

            const uniformThreshold = data.params.uniform_threshold / 100;
            const maxOutlierBuckets = data.params.max_outlier_buckets;
            const maxBucketRatio = totalImages > 0 ? maxBucket.image_count / totalImages : 1;
            const outlierCount = buckets.filter(b => b !== maxBucket && b.image_count > 0).length;

            const isUniform = maxBucketRatio >= uniformThreshold || outlierCount <= maxOutlierBuckets;

            // 确定活跃分支，标记非活跃分支的所有下游为 skipped
            const inactiveHandle = isUniform ? 'output-b' : 'output-a';

            for (const edge of edges) {
              if (edge.source === nodeId && edge.sourceHandle === inactiveHandle) inactiveEdges.add(edge.id);
            }

            // 输出路径传递给活跃分支（nested 标记同样要透传，否则美学下游在此断链）
            nodeOutputs.set(nodeId, inputPath);
            nodeNested.set(nodeId, inputNested);

            const branchLabel = isUniform ? 'A (均匀)' : 'B (分散)';
            const info = `→ ${branchLabel} | 桶${buckets.length}个 | 最大桶${(maxBucket?.image_count ?? 0)}/${totalImages}张 (${(maxBucketRatio * 100).toFixed(0)}%)`;
            callbacks.onNodeStatusChange(nodeId, 'done', info);
          } catch (e) {
            if (!this.cancelFlag) fail(nodeId, errMsg(e));
            return;
          }

          continue;
        }

        if (!def?.buildOptions) {
          if (inputPath) {
            nodeOutputs.set(nodeId, inputPath);
            nodeNested.set(nodeId, inputNested);
          }
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          continue;
        }

        // 重命名后端只扫顶层（无递归支持），嵌套输入必然找到 0 张图——给出可操作的报错
        if (inputNested && def.flatInputOnly) {
          fail(nodeId, '不支持嵌套输入', '重命名节点不支持递归处理子目录（上游输出按分类分层或开启了递归扫描）——请调整链路');
          return;
        }
        if (!inputPath) {
          fail(nodeId, '无输入', '该节点没有输入路径（需要连接上游节点）');
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
            fail(outFolder.id, '未设置路径', '输出文件夹节点未设置路径');
            return;
          }
          outputPath = outFolderPath;
        }

        if (!outputPath) {
          outputPath = buildTempOutputPath(inputPath, i, data.type);
        }

        try {
          const call = await def.buildOptions(data.params, { input_path: inputPath, output_path: outputPath, recursive: inputNested });
          // 在调用后端前再次检查取消标志，覆盖取消请求早于节点启动的情况。
          if (this.cancelFlag) {
            this.status = 'cancelled';
            callbacks.onNodeStatusChange(nodeId, 'idle', '已取消');
            return;
          }
          await startProgressListener(data.type);
          if (this.cancelFlag) return;
          this.currentNodeType = data.type;
          const result = await invoke<unknown>(call.command, { options: call.options });
          this.currentNodeType = '';
          if (this.cancelFlag) return;
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
              if (!def.allowEmptyResult) {
                throw `没有任何文件处理成功（成功 ${r.success_count}/${r.total}），已中止后续节点`;
              }
            }
            if ((r.fail_count ?? 0) > 0) doneMsg = `✓ (${r.fail_count} 个失败)`;
          }

          // filter 的 delete 模式是就地删除、不产出输出目录，必须按就地节点透传输入
          const isInPlaceNode = typeof def.inPlace === 'function' ? def.inPlace(data.params) : !!def.inPlace;

          // 图像节点只搬图片：把上游同名 .txt/.json/.caption 一起带上，
          // 否则"打标在前、图像处理在后"的链会把标签留在临时目录里随清理丢失
          if (def.carrySidecars && !isInPlaceNode) {
            try {
              await invoke('carry_tag_sidecars', { inputPath, outputPath, recursive: inputNested });
            } catch (e) {
              console.warn('携带标签文件失败:', e);
            }
          }

          // 原地操作节点（打标/重命名/过滤删除）不产出新目录，输出即输入，需透传给下游，
          // 否则下游会拿到一个从未被创建的临时目录路径而报「输入路径无效」
          nodeOutputs.set(nodeId, isInPlaceNode ? inputPath : outputPath);
          nodeNested.set(nodeId, !!def.nestedOutput || inputNested);
          callbacks.onNodeStatusChange(nodeId, 'done', doneMsg);
        } catch (e) {
          stopProgressListener();
          if (!this.cancelFlag) fail(nodeId, errMsg(e));
          return;
        }
      }

      if (this.cancelFlag) return;

      // 5. 全部完成
      // 记录是否有"链条终点产物仍留在临时目录"（该链没接输出文件夹）——
      // 那些 step_N 就是用户的最终结果，成功后的清理不能删
      {
        const processedIds = new Set(nodeOutputs.keys());
        const hasDownstreamProcessing = (id: string) =>
          edges.some(
            e => e.source === id && processedIds.has(e.target),
          );
        keepTempOnSuccess = [...nodeOutputs.entries()].some(
          ([id, p]) => !hasDownstreamProcessing(id) && TEMP_PATH_RE.test(p),
        );
      }
      const elapsed = Date.now() - startTime;
      this.status = 'done';
      callbacks.onComplete(elapsed);

    } catch (e) {
      if (!this.cancelFlag) {
        this.status = 'error';
        callbacks.onError(this.currentNodeId, errMsg(e));
      }
    } finally {
      this.currentNodeType = '';
      await this.cancelPromise;
      stopProgressListener();
      if (this.cancelFlag) this.status = 'cancelled';

      // 清理中间产物临时目录。
      // 失败/取消：中间产物是无用垃圾，直接清理。
      // 成功：有链条终点产物留在临时目录里时（keepTempOnSuccess）不清理，那些就是用户的结果。
      const shouldCleanup = !preserveTempOnExit && (this.status !== 'done' || !keepTempOnSuccess);
      if (shouldCleanup) {
        await cleanupWorkflowTemp(nodes);
      }
    }
  }
}
