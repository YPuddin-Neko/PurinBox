import { useState, useCallback, useRef, useMemo } from 'react';
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
import { save, open } from '@tauri-apps/plugin-dialog';
import {
  Workflow as WorkflowIcon,
  Plus, Save, FolderOpen, Play, Square, Trash2,
  PanelRightClose, PanelRightOpen, Map, ChevronDown,
} from 'lucide-react';
import NodePanel from '../components/workflow/NodePanel';
import PropertyPanel from '../components/workflow/PropertyPanel';
import BaseNode from '../components/workflow/nodes/BaseNode';
import { getNodeDef } from '../components/workflow/nodeDefinitions';
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
  const [selectedNode, setSelectedNode] = useState<Node<WorkflowNodeData> | null>(null);
  const [rfInstance, setRfInstance] = useState<any>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [showProps, setShowProps] = useState(true);
  const [showMinimap, setShowMinimap] = useState(true);
  const [showNodePanel, setShowNodePanel] = useState(false);
  const engineRef = useRef(new WorkflowEngine());
  // 上一次运行的 execute promise：取消后引擎还要收尾（等当前节点返回 + 清理临时目录），
  // 立刻重跑会让旧运行的清理删掉新运行刚建的目录、旧节点的后端互斥闸也还没释放
  const execPromiseRef = useRef<Promise<void> | null>(null);
  const [runningMessage, setRunningMessage] = useState('');
  const isDraggingRef = useRef(false);

  const onConnect = useCallback((connection: Connection) => {
    setEdges(eds => addEdge({ ...connection, animated: false, style: { stroke: 'var(--color-border-active)', strokeWidth: 2 } }, eds));
  }, [setEdges]);

  const onNodeClick = useCallback((_: any, node: Node<WorkflowNodeData>) => {
    setSelectedNode(node);
    setShowProps(true);
  }, []);

  const onSelectionChange = useCallback(({ nodes: selectedNodes }: { nodes: Node<WorkflowNodeData>[] }) => {
    if (selectedNodes.length === 1) {
      setSelectedNode(selectedNodes[0]);
    } else if (selectedNodes.length === 0) {
      setSelectedNode(null);
    }
  }, []);

  const onPaneClick = useCallback(() => {
    setSelectedNode(null);
    // 拖拽过程中不关闭节点库
    if (!isDraggingRef.current) {
      setShowNodePanel(false);
    }
  }, []);

  const onParamChange = useCallback((nodeId: string, key: string, value: any) => {
    setNodes(nds => nds.map(n => {
      if (n.id !== nodeId) return n;
      return { ...n, data: { ...n.data, params: { ...n.data.params, [key]: value } } };
    }));
    setSelectedNode(prev => {
      if (!prev || prev.id !== nodeId) return prev;
      return { ...prev, data: { ...prev.data, params: { ...prev.data.params, [key]: value } } };
    });
  }, [setNodes]);

  // ── 拖拽添加节点：用全局 mouseup 代替 HTML5 Drag API ──
  const pendingNodeTypeRef = useRef<string | null>(null);

  // LLM API 预设端点
  const LLM_PRESETS: Record<string, string> = {
    openai: 'https://api.openai.com/v1/',
    gemini: 'https://generativelanguage.googleapis.com/v1beta/openai/',
    deepseek: 'https://api.deepseek.com/v1/',
  };

  // 创建节点后，异步加载已保存的 API 配置
  const loadSavedConfig = useCallback(async (nodeId: string, nodeType: string) => {
    if (nodeType === 'llm-tagger') {
      try {
        const cfg = await invoke<{ preset: string; custom_endpoint: string; api_keys: Record<string, string> }>('load_api_config');
        const endpoint = cfg.preset === 'custom' ? cfg.custom_endpoint : (LLM_PRESETS[cfg.preset] || cfg.custom_endpoint || '');
        const apiKey = cfg.api_keys?.[cfg.preset] || '';
        setNodes(nds => nds.map(n => {
          if (n.id !== nodeId) return n;
          const d = n.data as WorkflowNodeData;
          return { ...n, data: { ...d, params: { ...d.params,
            api_endpoint: endpoint || d.params.api_endpoint,
            api_key: apiKey || d.params.api_key,
          }}};
        }));
      } catch {}
    }
  }, [setNodes]);

  const onCanvasMouseUp = useCallback((e: React.MouseEvent) => {
    const nodeType = pendingNodeTypeRef.current;
    if (!nodeType || !rfInstance || !reactFlowWrapper.current) return;
    pendingNodeTypeRef.current = null;
    isDraggingRef.current = false;

    const def = getNodeDef(nodeType);
    if (!def) return;
    const bounds = reactFlowWrapper.current.getBoundingClientRect();
    const position = rfInstance.screenToFlowPosition({ x: e.clientX - bounds.left, y: e.clientY - bounds.top });
    const defaultParams: Record<string, any> = {};
    def.params.forEach(p => { defaultParams[p.key] = p.default; });
    const id = nextNodeId();
    setNodes(nds => [...nds, {
      id, type: 'baseNode', position,
      data: { type: nodeType, label: nodeType, params: defaultParams, status: 'idle' },
    }]);
    setShowNodePanel(false);
    loadSavedConfig(id, nodeType);
  }, [rfInstance, setNodes, loadSavedConfig]);

  // 从 NodePanel mousedown 时触发
  const onNodeDragStarted = useCallback((nodeType: string) => {
    pendingNodeTypeRef.current = nodeType;
    isDraggingRef.current = true;
  }, []);

  const onNodeDragEnded = useCallback(() => {
    // 如果没在画布上 mouseup，清除 pending
    setTimeout(() => {
      pendingNodeTypeRef.current = null;
      isDraggingRef.current = false;
    }, 100);
  }, []);

  const handleAddNode = useCallback((nodeType: string) => {
    // 只在非拖拽时才走点击添加
    if (isDraggingRef.current) return;
    const def = getNodeDef(nodeType);
    if (!def) return;
    const defaultParams: Record<string, any> = {};
    def.params.forEach(p => { defaultParams[p.key] = p.default; });
    const id = nextNodeId();
    setNodes(nds => [...nds, {
      id, type: 'baseNode',
      position: { x: 200 + Math.random() * 200, y: 100 + Math.random() * 200 },
      data: { type: nodeType, label: nodeType, params: defaultParams, status: 'idle' },
    }]);
    setShowNodePanel(false);
    loadSavedConfig(id, nodeType);
  }, [setNodes, loadSavedConfig]);

  const handleNew = useCallback(() => {
    if (nodes.length > 0 && !window.confirm(t('workflow.clearConfirm'))) return;
    setNodes([]); setEdges([]); setSelectedNode(null); nodeIdCounter = 0;
  }, [nodes, setNodes, setEdges, t]);

  const handleSave = useCallback(async () => {
    try {
      const path = await save({ title: t('workflow.save'), filters: [{ name: t('workflow.workflowFile'), extensions: ['purin'] }] });
      if (!path) return;
      const data: WorkflowData = {
        version: 1, name: path.split(/[\\/]/).pop()?.replace('.purin', '') || 'workflow',
        nodes: nodes.map(n => ({ id: n.id, type: n.data.type, position: n.position, data: n.data })),
        edges: edges.map(e => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? undefined, targetHandle: e.targetHandle ?? undefined })),
      };
      await invoke('save_workflow', { path, data: JSON.stringify(data) });
    } catch (e: any) { console.error('Save failed:', e); }
  }, [nodes, edges, t]);

  const handleLoad = useCallback(async () => {
    try {
      const path = await open({ title: t('workflow.load'), filters: [{ name: t('workflow.workflowFile'), extensions: ['purin'] }], multiple: false }) as string | null;
      if (!path) return;
      const json = await invoke<string>('load_workflow', { path });
      const data: WorkflowData = JSON.parse(json);
      setNodes(data.nodes.map(n => ({ id: n.id, type: 'baseNode', position: n.position, data: { ...n.data, status: 'idle', statusMessage: undefined } })));
      setEdges(data.edges.map(e => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle, targetHandle: e.targetHandle, style: { stroke: 'var(--color-border-active)', strokeWidth: 2 } })));
      setSelectedNode(null);
      const maxId = data.nodes.reduce((max, n) => { const num = parseInt(n.id.replace('node_', '')); return isNaN(num) ? max : Math.max(max, num); }, 0);
      nodeIdCounter = maxId;
    } catch (e: any) { console.error('Load failed:', e); }
  }, [setNodes, setEdges, t]);

  const handleRun = useCallback(async () => {
    if (isRunning) {
      engineRef.current.cancel();
      // 重置所有节点状态
      setNodes(nds => nds.map(n => ({
        ...n,
        data: { ...n.data, status: 'idle', statusMessage: undefined, progressCurrent: undefined, progressTotal: undefined },
      })));
      setIsRunning(false);
      setRunningMessage('');
      return;
    }

    if (nodes.length === 0) return;

    setIsRunning(true);
    if (execPromiseRef.current) {
      setRunningMessage('等待上一次运行收尾...');
      await execPromiseRef.current.catch(() => {});
    }
    setRunningMessage(t('workflow.runStart'));

    // 更新节点状态的回调
    const updateNodeStatus = (nodeId: string, status: WorkflowNodeData['status'], message?: string) => {
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

    // 更新节点进度的回调
    const updateNodeProgress = (nodeId: string, current: number, total: number) => {
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
        const node = nodes.find(n => n.id === _nodeId);
        const name = node ? t(getNodeDef(node.data.type)?.nameKey || '') : '';
        setRunningMessage(t('workflow.runStep', { name, current: step + 1, total }));
      },
      onStepDone: () => {},
      onComplete: (elapsed) => {
        const secs = (elapsed / 1000).toFixed(1);
        setRunningMessage(t('workflow.runDone', { time: `${secs}s` }));
        setIsRunning(false);
        setTimeout(() => setRunningMessage(''), 5000);
      },
      onError: (_nodeId, error) => {
        setRunningMessage(t('workflow.runError', { error }));
        setIsRunning(false);
      },
    });
    execPromiseRef.current = runPromise;
    await runPromise;
  }, [isRunning, nodes, edges, setNodes, t]);

  const currentSelected = useMemo(() => {
    if (!selectedNode) return null;
    return nodes.find(n => n.id === selectedNode.id) ?? null;
  }, [nodes, selectedNode]);

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
          <button className="wf-tb-btn" onClick={handleNew} title={t('workflow.newWorkflow')}><Trash2 size={15} /></button>
          <button className="wf-tb-btn" onClick={handleSave} title={t('workflow.save')}><Save size={15} /></button>
          <button className="wf-tb-btn" onClick={handleLoad} title={t('workflow.load')}><FolderOpen size={15} /></button>
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
        {showProps && <PropertyPanel selectedNode={currentSelected} onParamChange={onParamChange} />}
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
