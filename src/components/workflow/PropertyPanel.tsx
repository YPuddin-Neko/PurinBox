import { useTranslation } from 'react-i18next';
import type { Node } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import { getNodeDef } from './nodeDefinitions';
import ParamControl from './ParamControl';
import { useNodeParamUpdater } from './useNodeParamUpdater';

function NodeProperties({ node }: { node: Node<WorkflowNodeData> }) {
  const { t } = useTranslation();
  const def = getNodeDef(node.data.type);
  const { params, updateParam, isDisabled } = useNodeParamUpdater(node.id, node.data);
  if (!def) return null;
  return <>
    <div className="wf-panel-header"><span className="wf-prop-dot" style={{ background: def.color }} />{t(def.nameKey)}</div>
    <div className="wf-prop-body">
      {def.params.map(param => <div key={param.key} className="wf-prop-field">
        {param.type !== 'boolean' && <label className="wf-prop-label">{t(param.labelKey)}</label>}
        <ParamControl param={param} value={params[param.key]} onChange={updateParam} disabled={isDisabled(param.key)} />
      </div>)}
    </div>
  </>;
}

export default function PropertyPanel({ selectedNode }: { selectedNode: Node<WorkflowNodeData> | null }) {
  const { t } = useTranslation();
  return <div className="wf-prop-panel">
    {selectedNode ? <NodeProperties key={selectedNode.id} node={selectedNode} /> : <>
      <div className="wf-panel-header">{t('workflow.properties')}</div>
      <div className="wf-prop-empty">{t('workflow.noSelection')}</div>
    </>}
  </div>;
}
