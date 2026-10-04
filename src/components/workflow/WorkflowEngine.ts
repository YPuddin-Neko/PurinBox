// ═══════════════ 工作流执行引擎 ═══════════════
import { invoke } from '@tauri-apps/api/core';
// 走 tauriRuntime 封装：浏览器模式下（无 Tauri 运行时）listen 为 noop
import { listen } from '../../utils/tauriRuntime';
import type { Node, Edge } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import type { ProcessResult } from '../../api/commandOptions';
import { getNodeDef, isInPlaceNode, withDefaults } from './nodeDefinitions';
import { planWorkflow, TEMP_PATH_RE, type WorkflowIssueKind } from './workflowValidation';

type UnlistenFn = () => void;
type ExecutionStatus = 'idle' | 'running' | 'done' | 'error' | 'cancelled';

/** i18next 的 t：节点状态与错误按当前界面语言生成 */
export type Translate = (key: string, params?: Record<string, unknown>) => string;

interface ExecutionCallbacks {
  onNodeStatusChange: (nodeId: string, status: WorkflowNodeData['status'], message?: string) => void;
  onStepStart: (nodeId: string, stepIndex: number, totalSteps: number) => void;
  onComplete: (elapsed: number) => void;
  onError: (nodeId: string, error: string) => void;
  onProgress?: (nodeId: string, current: number, total: number) => void;
}

/** 运行前校验问题 → [节点上的简短状态, 底栏的完整提示]（workflow.* 键，可带节点名 name） */
const ISSUE_MESSAGES: Record<WorkflowIssueKind, [string, string]> = {
  inputInTemp: ['workflow.statusInputInTemp', 'workflow.errInputInTemp'],
  cycle: ['workflow.errCycle', 'workflow.errCycle'],
  missingInputPath: ['workflow.errNoInputPath', 'workflow.errNoInputPath'],
  missingOutputPath: ['workflow.errNoOutputPath', 'workflow.errNoOutputPath'],
  missingModel: ['workflow.errNoModel', 'workflow.errNoModel'],
  noInput: ['workflow.statusNoInput', 'workflow.errNoInput'],
  multipleInputs: ['workflow.statusMultipleInputs', 'workflow.errMultipleInputs'],
  needsOutput: ['workflow.errNeedsOutput', 'workflow.errNeedsOutput'],
};

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

  constructor(private readonly t: Translate) {}

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
    const t = this.t;
    const errMsg = (e: unknown): string =>
      typeof e === 'string' ? e : (e as { message?: string } | null)?.message || t('workflow.errUnknown');
    const fail = (id: string, short: string, detail = short) => {
      callbacks.onNodeStatusChange(id, 'error', short);
      callbacks.onError(id, detail);
      this.status = 'error';
    };
    this.status = 'running';
    const startTime = Date.now();
    // 校验通过后才会动临时目录：没通过时什么都没执行，输入还可能就在 .workflow_temp 里
    let started = false;

    // 当前节点的进度监听器（每个步骤动态切换）
    let currentProgressUnlisten: UnlistenFn | null = null;
    const stopProgressListener = () => {
      if (currentProgressUnlisten) {
        currentProgressUnlisten();
        currentProgressUnlisten = null;
      }
    };

    try {
      // 0. 运行前校验整张图，有问题就一个节点都不跑
      const { order, issue } = planWorkflow(nodes, edges);
      if (issue) {
        const node = nodes.find(n => n.id === issue.nodeId);
        const name = node ? t(getNodeDef(node.data.type)?.nameKey ?? node.data.type) : '';
        const [short, detail] = ISSUE_MESSAGES[issue.kind];
        fail(issue.nodeId, t(short, { name }), t(detail, { name }));
        return;
      }
      started = true;

      // 1. 清理上一次运行的中间产物：临时目录按 step_{序号}_{类型} 命名，残留的旧文件会被当成上游产物
      await cleanupWorkflowTemp(nodes);

      const nodeMap = new Map(nodes.map(n => [n.id, n]));
      const totalSteps = order.length;

      // 用于跟踪每个节点的输出目录（作为下游的输入）
      const nodeOutputs = new Map<string, string>();
      // 输出目录带 <label>/ 子层级的节点（美学评分）：下游收集必须递归，且沿链传递
      const nodeNested = new Map<string, boolean>();
      // 分桶节点没走的那个出口上的连线
      const inactiveEdges = new Set<string>();

      // 2. 把当前节点的后端进度事件（节点定义的 progressEvent）转发给 onProgress
      const startProgressListener = async (nodeType: string, nodeId: string) => {
        stopProgressListener();
        const requestId = crypto.randomUUID();
        const eventName = getNodeDef(nodeType)?.progressEvent;
        if (!eventName || !callbacks.onProgress) return requestId;

        let active = true;
        const unlisten = await listen<{ current: number; total: number; request_id?: string }>(eventName, (event) => {
          if (active && !this.cancelFlag && this.status === 'running'
            && this.currentNodeId === nodeId && event.payload.request_id === requestId) {
            callbacks.onProgress!(nodeId, event.payload.current, event.payload.total);
          }
        });
        currentProgressUnlisten = () => { active = false; unlisten(); };
        return requestId;
      };

      // 3. 标记所有节点为等待
      for (const id of order) {
        callbacks.onNodeStatusChange(id, 'waiting');
      }

      // 4. 依次执行每个节点
      for (let i = 0; i < order.length; i++) {
        if (this.cancelFlag) {
          this.status = 'cancelled';
          for (let j = i; j < order.length; j++) {
            callbacks.onNodeStatusChange(order[j], 'idle');
          }
          return;
        }

        const nodeId = order[i];
        const node = nodeMap.get(nodeId)!;
        const type = node.data.type;
        const def = getNodeDef(type);
        const params = withDefaults(def, node.data.params);

        this.currentNodeId = nodeId;
        callbacks.onStepStart(nodeId, i, totalSteps);
        callbacks.onNodeStatusChange(nodeId, 'running', t('workflow.statusRunning', { current: i + 1, total: totalSteps }));

        // ── 输入节点：直接使用用户指定的路径 ──
        if (type === 'image-folder') {
          nodeOutputs.set(nodeId, params.path);
          // 开启递归扫描时下游也必须递归收集，否则嵌套数据集（10_charA/ 等）会收集到 0 张图
          nodeNested.set(nodeId, !!params.recursive);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          continue;
        }

        // 校验保证最多一个上游实际产出；一个都没有说明在分桶没走的分支上
        const upstream = edges.find(
          edge => edge.target === nodeId && !inactiveEdges.has(edge.id) && nodeOutputs.has(edge.source),
        )?.source;
        if (upstream === undefined) {
          callbacks.onNodeStatusChange(nodeId, 'idle');
          continue;
        }
        const inputPath = nodeOutputs.get(upstream)!;
        const inputNested = nodeNested.get(upstream) ?? false;

        if (type === 'output-folder') {
          // 上游已直接写进这个目录时两边相同，后端不再复制
          try {
            await invoke('carry_tag_sidecars', { inputPath, outputPath: params.path, recursive: inputNested, copyImages: true });
          } catch (e) {
            if (!this.cancelFlag) fail(nodeId, errMsg(e));
            return;
          }
          if (this.cancelFlag) return;
          nodeOutputs.set(nodeId, params.path);
          nodeNested.set(nodeId, inputNested);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          continue;
        }

        // ── 分桶条件分支节点 ──
        if (type === 'bucket-assign') {
          try {
            const call = await def!.buildOptions!(params, { input_path: inputPath, output_path: '', recursive: inputNested });
            const requestId = await startProgressListener(type, nodeId);
            if (this.cancelFlag) return;
            this.currentNodeType = type;
            const result = await invoke<{
              bucket_count: number;
              buckets: { bucket_width: number; bucket_height: number; image_count: number }[];
            }>('execute_workflow_node', { command: call.command, options: call.options, requestId });
            this.currentNodeType = '';
            stopProgressListener();
            if (this.cancelFlag) return;

            const buckets = result.buckets || [];
            const totalImages = buckets.reduce((s, b) => s + b.image_count, 0);

            let maxBucket = buckets[0];
            for (const b of buckets) {
              if (b.image_count > maxBucket.image_count) maxBucket = b;
            }

            const uniformThreshold = params.uniform_threshold / 100;
            const maxOutlierBuckets = params.max_outlier_buckets;
            const maxBucketRatio = totalImages > 0 ? maxBucket.image_count / totalImages : 1;
            const outlierCount = buckets.filter(b => b !== maxBucket && b.image_count > 0).length;

            const isUniform = maxBucketRatio >= uniformThreshold || outlierCount <= maxOutlierBuckets;

            // 没走的出口上的连线失效，只连在它上面的下游整条跳过
            const inactiveHandle = isUniform ? 'output-b' : 'output-a';
            for (const edge of edges) {
              if (edge.source === nodeId && edge.sourceHandle === inactiveHandle) inactiveEdges.add(edge.id);
            }

            // 输出路径传递给活跃分支（nested 标记同样要透传，否则美学下游在此断链）
            nodeOutputs.set(nodeId, inputPath);
            nodeNested.set(nodeId, inputNested);

            callbacks.onNodeStatusChange(nodeId, 'done', t('workflow.bucketResult', {
              branch: t(isUniform ? 'workflow.bucketBranchUniform' : 'workflow.bucketBranchScattered'),
              buckets: buckets.length,
              max: maxBucket?.image_count ?? 0,
              total: totalImages,
              percent: (maxBucketRatio * 100).toFixed(0),
            }));
          } catch (e) {
            if (!this.cancelFlag) fail(nodeId, errMsg(e));
            return;
          }

          continue;
        }

        // 旧版工作流里已移除的节点类型：原样透传
        if (!def?.buildOptions) {
          nodeOutputs.set(nodeId, inputPath);
          nodeNested.set(nodeId, inputNested);
          callbacks.onNodeStatusChange(nodeId, 'done', '✓');
          continue;
        }

        // 重命名后端只扫顶层（无递归支持），嵌套输入必然找到 0 张图——给出可操作的报错
        if (inputNested && def.flatInputOnly) {
          fail(nodeId, t('workflow.statusNestedInput'), t('workflow.errNestedInput'));
          return;
        }

        // 下游接了输出文件夹就直接写进去（不能只看第一条边——先画的边不一定是它），否则写进临时目录
        const outFolder = edges
          .filter(e => e.source === nodeId)
          .map(e => nodeMap.get(e.target))
          .find(c => c?.data.type === 'output-folder');
        const outputPath = outFolder ? String(outFolder.data.params.path) : buildTempOutputPath(inputPath, i, type);

        try {
          const call = await def.buildOptions(params, { input_path: inputPath, output_path: outputPath, recursive: inputNested });
          // 在调用后端前再次检查取消标志，覆盖取消请求早于节点启动的情况。
          if (this.cancelFlag) {
            this.status = 'cancelled';
            callbacks.onNodeStatusChange(nodeId, 'idle', t('workflow.statusCancelled'));
            return;
          }
          const requestId = await startProgressListener(type, nodeId);
          if (this.cancelFlag) return;
          this.currentNodeType = type;
          const result = await invoke<unknown>('execute_workflow_node', { command: call.command, options: call.options, requestId });
          this.currentNodeType = '';
          if (this.cancelFlag) return;
          stopProgressListener();

          // ProcessResult 不能丢弃：全部失败/输入为空时若照样标 ✓，
          // 空目录会沿链传下去，最后显示"运行完成"却什么都没有
          let doneMsg = '✓';
          const r = result as Partial<ProcessResult> | null;
          if (r && typeof r === 'object' && typeof r.success_count === 'number' && typeof r.total === 'number') {
            // 过滤节点的 success 只计"匹配"数：0 匹配是正常的空跑（delete 模式尤其如此），
            // 不能中止整条链；copy 模式的空产物会在下游以清晰的 total=0 报出
            if ((r.total === 0 || r.success_count === 0) && !def.allowEmptyResult) {
              throw t('workflow.errNothingProcessed', { success: r.success_count, total: r.total });
            }
            if ((r.fail_count ?? 0) > 0) doneMsg = t('workflow.doneWithFailures', { count: r.fail_count });
          }

          // filter 的 delete 模式是就地删除、不产出输出目录，必须按就地节点透传输入
          const inPlace = isInPlaceNode(def, params);

          // 图像节点只搬图片：把上游同名 .txt/.json/.caption 一起带上，
          // 否则"打标在前、图像处理在后"的链会把标签留在临时目录里随清理丢失
          if (def.carrySidecars && !inPlace) {
            try {
              await invoke('carry_tag_sidecars', { inputPath, outputPath, recursive: inputNested });
            } catch (e) {
              console.warn('携带标签文件失败:', e);
            }
          }

          // 原地操作节点（打标/重命名/过滤删除）不产出新目录，输出即输入，需透传给下游，
          // 否则下游会拿到一个从未被创建的临时目录路径而报「输入路径无效」
          nodeOutputs.set(nodeId, inPlace ? inputPath : outputPath);
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
      this.status = 'done';
      callbacks.onComplete(Date.now() - startTime);

    } catch (e) {
      if (!this.cancelFlag) fail(this.currentNodeId, errMsg(e));
    } finally {
      this.currentNodeType = '';
      await this.cancelPromise;
      stopProgressListener();
      if (this.cancelFlag) this.status = 'cancelled';

      // 校验保证每条链的结果都在输入或输出文件夹里，临时目录只剩中间产物，运行结束一律清理
      if (started) {
        await cleanupWorkflowTemp(nodes);
      }
    }
  }
}
