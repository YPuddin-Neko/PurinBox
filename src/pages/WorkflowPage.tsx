import { useState, useCallback, useRef, useMemo, useEffect } from 'react';
import {
  ReactFlow,
  MiniMap,
  Controls,
  Background,
  addEdge,
  useNodesState,
  useEdgesState,
  type Connection,
  type Node,
  type Edge,
  BackgroundVariant,
  ReactFlowProvider,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { save, open, confirm, message } from '@tauri-apps/plugin-dialog';
import {
  Workflow as WorkflowIcon,
  Plus, Save, FolderOpen, Play, Square, Trash2,
  PanelRightClose, PanelRightOpen, Map, ChevronDown,
} from 'lucide-react';
import NodePanel from '../components/workflow/NodePanel';
import PropertyPanel from '../components/workflow/PropertyPanel';
import BaseNode from '../components/workflow/nodes/BaseNode';
import { getNodeDef, withDefaults } from '../components/workflow/nodeDefinitions';
import type { WorkflowNodeData, WorkflowData } from '../components/workflow/workflowTypes';
import { WorkflowEngine } from '../components/workflow/WorkflowEngine';
import '../components/workflow/workflow.css';

let nodeIdCounter = 0;
function nextNodeId() { return `node_${++nodeIdCounter}`; }

const nodeTypes = { baseNode: BaseNode };

function WorkflowEditor() {
  const { t } = useTranslation();
  const reactFlowWrapper = useRef<HTMLDivElement>(null);
  const [nodes, setNodes, onNodesChange] = useNodesState<Node<WorkflowNodeData>>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [rfInstance, setRfInstance] = useState<any>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [showProps, setShowProps] = useState(true);
  const [showMinimap, setShowMinimap] = useState(true);
  const [showNodePanel, setShowNodePanel] = useState(false);
  const engineRef = useRef<WorkflowEngine | null>(null);
  const requestRef = useRef(0);
  const runningRef = useRef(false);
  useEffect(() => () => {
    requestRef.current++;
    engineRef.current?.cancel();
  }, []);
  // 上一次运行的 execute promise：取消后引擎还要收尾（等当前节点返回 + 清理临时目录），
  // 立刻重跑会让旧运行的清理删掉新运行刚建的目录、旧节点的后端互斥闸也还没释放
  const execPromiseRef = useRef<Promise<void> | null>(null);
  const [runningMessage, setRunningMessage] = useState('');
  const isDraggingRef = useRef(false);

  const onConnect = useCallback((connection: Connection) => {
    setEdges(eds => addEdge(connection, eds));
  }, [setEdges]);

  const onNodeClick = useCallback((_: any, node: Node<WorkflowNodeData>) => {
    setSelectedNodeId(node.id);
    setShowProps(true);
  }, []);

  const onSelectionChange = useCallback(({ nodes: selectedNodes }: { nodes: Node<WorkflowNodeData>[] }) => {
    if (selectedNodes.length === 1) {
      setSelectedNodeId(selectedNodes[0].id);
    } else if (selectedNodes.length === 0) {
      setSelectedNodeId(null);
    }
  }, []);

  const onPaneClick = useCallback(() => {
    setSelectedNodeId(null);
    // 拖拽过程中不关闭节点库
    if (!isDraggingRef.current) {
      setShowNodePanel(false);
    }
  }, []);

  // ── 拖拽添加节点：用全局 mouseup 代替 HTML5 Drag API ──
  const pendingNodeTypeRef = useRef<string | null>(null);

  const addNodeAt = useCallback((nodeType: string, position: { x: number; y: number }) => {
    const def = getNodeDef(nodeType);
    if (!def) return;
    const defaultParams = withDefaults(def);
    const id = nextNodeId();
    setNodes(nds => [...nds, {
      id, type: 'baseNode', position,
      data: { type: nodeType, params: defaultParams, status: 'idle' },
    }]);
    setShowNodePanel(false);
  }, [setNodes]);

  const onCanvasMouseUp = useCallback((e: React.MouseEvent) => {
    const nodeType = pendingNodeTypeRef.current;
    if (!nodeType || !rfInstance || !reactFlowWrapper.current) return;
    pendingNodeTypeRef.current = null;
    isDraggingRef.current = false;

    addNodeAt(nodeType, rfInstance.screenToFlowPosition({ x: e.clientX, y: e.clientY }));
  }, [rfInstance, addNodeAt]);

  // 从 NodePanel mousedown 时触发
  const onNodeDragStarted = useCallback((nodeType: string) => {
    pendingNodeTypeRef.current = nodeType;
    isDraggingRef.current = true;
  }, []);

  const onNodeDragEnded = useCallback(() => {
    pendingNodeTypeRef.current = null;
    isDraggingRef.current = false;
  }, []);

  const handleAddNode = useCallback((nodeType: string) => {
    // 只在非拖拽时才走点击添加
    if (isDraggingRef.current) return;
    addNodeAt(nodeType, { x: 200 + Math.random() * 200, y: 100 + Math.random() * 200 });
  }, [addNodeAt]);

  const handleNew = useCallback(async () => {
    if (nodes.length > 0 && !await confirm(t('workflow.clearConfirm'), { title: t('workflow.newWorkflow'), kind: 'warning' })) return;
    setNodes([]); setEdges([]); setSelectedNodeId(null); nodeIdCounter = 0;
    setRunningMessage('');
  }, [nodes, setNodes, setEdges, t]);

  const handleSave = useCallback(async () => {
    try {
      const path = await save({ title: t('workflow.save'), filters: [{ name: t('workflow.workflowFile'), extensions: ['purin'] }] });
      if (!path) return;
      const data: WorkflowData = {
        version: 1, name: path.split(/[\\/]/).pop()?.replace('.purin', '') || 'workflow',
        nodes: nodes.map(n => ({ id: n.id, type: n.data.type, position: n.position, data: { type: n.data.type, params: withDefaults(getNodeDef(n.data.type), n.data.params), status: 'idle' } })),
        edges: edges.map(e => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? undefined, targetHandle: e.targetHandle ?? undefined })),
      };
      await invoke('save_workflow', { path, data: JSON.stringify(data) });
    } catch (e: unknown) { await message(`${t('workflow.saveFailed')}: ${String(e)}`, { title: t('workflow.save'), kind: 'error' }); }
  }, [nodes, edges, t]);

  const handleLoad = useCallback(async () => {
    try {
      const path = await open({ title: t('workflow.load'), filters: [{ name: t('workflow.workflowFile'), extensions: ['purin'] }], multiple: false }) as string | null;
      if (!path) return;
      const json = await invoke<string>('load_workflow', { path });
      const data: WorkflowData = JSON.parse(json);
      if (!Array.isArray(data.nodes) || !Array.isArray(data.edges)) throw new Error(t('workflow.loadFailed'));
      if (data.nodes.some(node => !node.id || !node.data?.type || !node.position)) throw new Error(t('workflow.loadFailed'));
      setNodes(data.nodes.map(n => ({ id: n.id, type: 'baseNode', position: n.position, data: { type: n.data.type, params: withDefaults(getNodeDef(n.data.type), n.data.params), status: 'idle' } })));
      setEdges(data.edges.map(e => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle, targetHandle: e.targetHandle })));
      setSelectedNodeId(null);
      const maxId = data.nodes.reduce((max, n) => { const num = parseInt(n.id.replace('node_', '')); return isNaN(num) ? max : Math.max(max, num); }, 0);
      nodeIdCounter = maxId;
      setRunningMessage('');
    } catch (e: unknown) { await message(`${t('workflow.loadFailed')}: ${String(e)}`, { title: t('workflow.load'), kind: 'error' }); }
  }, [setNodes, setEdges, t]);

  const handleRun = useCallback(async () => {
    if (runningRef.current) {
      runningRef.current = false;
      requestRef.current++;
      engineRef.current?.cancel();
      setNodes(nds => nds.map(n => ({
        ...n,
        data: { ...n.data, status: 'idle', statusMessage: undefined, progressCurrent: undefined, progressTotal: undefined },
      })));
      setIsRunning(false);
      setRunningMessage('');
      return;
    }

    if (nodes.length === 0) return;

    const request = ++requestRef.current;
    const isCurrent = () => request === requestRef.current;
    runningRef.current = true;
    setIsRunning(true);
    if (execPromiseRef.current) {
      setRunningMessage(t('workflow.waitPreviousRun'));
      await execPromiseRef.current.catch(() => {});
    }
    if (!isCurrent()) return;
    setRunningMessage(t('workflow.runStart'));

    const updateNodeStatus = (nodeId: string, status: WorkflowNodeData['status'], message?: string) => {
      if (!isCurrent()) return;
      setNodes(nds => nds.map(n => {
        if (n.id !== nodeId) return n;
        const updates: Partial<WorkflowNodeData> = { status, statusMessage: message };
        // 完成或出错时清除进度
        if (status === 'done' || status === 'error') {
          updates.progressCurrent = undefined;
          updates.progressTotal = undefined;
        }
        return { ...n, data: { ...n.data, ...updates } };
      }));
    };

    const updateNodeProgress = (nodeId: string, current: number, total: number) => {
      if (!isCurrent()) return;
      setNodes(nds => nds.map(n => {
        if (n.id !== nodeId) return n;
        return { ...n, data: { ...n.data, progressCurrent: current, progressTotal: total, statusMessage: `${current}/${total}` } };
      }));
    };

    const engine = new WorkflowEngine();
    engineRef.current = engine;

    const runPromise = engine.execute(nodes, edges, {
      onNodeStatusChange: updateNodeStatus,
      onProgress: updateNodeProgress,
      onStepStart: (_nodeId, step, total) => {
        if (!isCurrent()) return;
        const node = nodes.find(n => n.id === _nodeId);
        const name = node ? t(getNodeDef(node.data.type)?.nameKey || '') : '';
        setRunningMessage(t('workflow.runStep', { name, current: step + 1, total }));
      },
      onComplete: (elapsed) => {
        if (!isCurrent()) return;
        const secs = (elapsed / 1000).toFixed(1);
        setRunningMessage(t('workflow.runDone', { time: `${secs}s` }));
      },
      onError: (_nodeId, error) => {
        if (!isCurrent()) return;
        setRunningMessage(t('workflow.runError', { error }));
      },
    });
    execPromiseRef.current = runPromise;
    try {
      await runPromise;
    } finally {
      if (execPromiseRef.current === runPromise) execPromiseRef.current = null;
      if (engineRef.current === engine) engineRef.current = null;
      if (isCurrent()) {
        runningRef.current = false;
        setIsRunning(false);
      }
    }
  }, [nodes, edges, setNodes, t]);

  const currentSelected = useMemo(() => {
    if (!selectedNodeId) return null;
    return nodes.find(n => n.id === selectedNodeId) ?? null;
  }, [nodes, selectedNodeId]);

  return (
    <div className="page" style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* 页面标题 */}
      <div className="page-header" style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
        <div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
            <WorkflowIcon style={{ width: 28, height: 28, color: '#7c5cfc' }} />
            <h1 className="page-title">{t('workflow.title')}</h1>
          </div>
          <p className="page-subtitle">{t('workflow.subtitle')}</p>
        </div>
        {/* 工具栏 - 右对齐，纯图标 */}
        <div className="wf-toolbar">
          <div className="wf-tb-group">
            <button className="wf-tb-btn" onClick={() => setShowNodePanel(!showNodePanel)} title={t('workflow.nodeLibrary')}>
              <Plus size={15} />
              <ChevronDown size={10} style={{ opacity: 0.5, transform: showNodePanel ? 'rotate(180deg)' : 'none', transition: 'transform 0.2s' }} />
            </button>
            {showNodePanel && (
              <div className="wf-tb-dropdown">
                <NodePanel onAddNode={handleAddNode} onDragStarted={onNodeDragStarted} onDragEnded={onNodeDragEnded} />
              </div>
            )}
          </div>
          <button className="wf-tb-btn" onClick={handleNew} disabled={isRunning} title={t('workflow.newWorkflow')}><Trash2 size={15} /></button>
          <button className="wf-tb-btn" onClick={handleSave} title={t('workflow.save')}><Save size={15} /></button>
          <button className="wf-tb-btn" onClick={handleLoad} disabled={isRunning} title={t('workflow.load')}><FolderOpen size={15} /></button>
          <div className="wf-tb-divider" />
          <button className="wf-tb-btn" onClick={() => setShowMinimap(!showMinimap)} title="Minimap">
            <Map size={15} style={{ opacity: showMinimap ? 1 : 0.4 }} />
          </button>
          <button className="wf-tb-btn" onClick={() => setShowProps(!showProps)} title={t('workflow.properties')}>
            {showProps ? <PanelRightClose size={15} /> : <PanelRightOpen size={15} />}
          </button>
          <div className="wf-tb-divider" />
          <button className={`wf-tb-btn ${isRunning ? 'wf-tb-danger' : 'wf-tb-run'}`} onClick={handleRun} title={isRunning ? t('workflow.stop') : t('workflow.run')}>
            {isRunning ? <Square size={15} /> : <Play size={15} />}
          </button>
        </div>
      </div>

      {/* 画布区域 */}
      <div className="wf-main">
        <div className="wf-canvas-area" ref={reactFlowWrapper} onMouseUp={onCanvasMouseUp}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={onNodeClick}
            onPaneClick={onPaneClick}
            onSelectionChange={onSelectionChange}
            onInit={setRfInstance}
            nodeTypes={nodeTypes}
            fitView
            deleteKeyCode={['Backspace', 'Delete']}
            proOptions={{ hideAttribution: true }}
          >
            <Controls position="bottom-left" />
            {showMinimap && (
              <MiniMap
                nodeColor={(n: Node) => {
                  const def = getNodeDef((n.data as WorkflowNodeData)?.type);
                  return def?.color ?? '#666';
                }}
                position="bottom-right"
                pannable zoomable
                style={{ width: 140, height: 90 }}
              />
            )}
            <Background variant={BackgroundVariant.Dots} gap={20} size={1.5} color="var(--color-text-tertiary)" />
          </ReactFlow>
        </div>

        {/* 右侧属性面板 */}
        {showProps && <PropertyPanel selectedNode={currentSelected} />}
      </div>

      {/* 执行状态消息 */}
      {runningMessage && (
        <div className="wf-run-msg">
          {isRunning && <span className="wf-run-spinner" />}
          <span>{runningMessage}</span>
        </div>
      )}
    </div>
  );
}

export default function WorkflowPage() {
  return (
    <ReactFlowProvider>
      <WorkflowEditor />
    </ReactFlowProvider>
  );
}
