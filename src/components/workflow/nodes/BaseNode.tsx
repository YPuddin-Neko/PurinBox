import { memo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { useTranslation } from 'react-i18next';
import type { WorkflowNodeData } from '../workflowTypes';
import { getNodeDef } from '../nodeDefinitions';
import ParamControl from '../ParamControl';
import { useNodeParamUpdater } from '../useNodeParamUpdater';

function BaseNode({ id, data, selected }: NodeProps & { data: WorkflowNodeData }) {
  const { t } = useTranslation();
  const def = getNodeDef(data.type);

  const { params, updateParam, isDisabled } = useNodeParamUpdater(id, data);
  if (!def) return null;
  const statusClass = `wf-node-status-${data.status}`;


  const nonBoolParams = def.params.filter(p => p.type !== 'boolean');
  const boolParams = def.params.filter(p => p.type === 'boolean');

  return (
    <div
      className={`wf-node ${statusClass} ${selected ? 'wf-node-selected' : ''}`}
      style={{ '--node-color': def.color } as React.CSSProperties}
    >
      {/* 标题栏 */}
      <div className="wf-node-header">
        <div className="wf-node-color-dot" />
        <span className="wf-node-title">{t(def.nameKey)}</span>
        {data.status === 'running' && <span className="wf-node-spinner" />}
        {data.status === 'done' && <span className="wf-node-check">✓</span>}
        {data.status === 'error' && <span className="wf-node-error">✗</span>}
      </div>

      {/* Slot 行：输入标签(左) + 输出标签(右)，ComfyUI 风格 */}
      <div className="wf-node-slot-row">
        {/* 输入 slot */}
        <div className="wf-node-slot-left">
          {def.hasInput && (
            <div className="wf-node-slot wf-slot-in">
              <Handle type="target" position={Position.Left} className="wf-handle wf-handle-input" />
              <span className="wf-slot-label">{t(def.inputLabelKey || 'workflow.slotImage')}</span>
            </div>
          )}
        </div>
        {/* 输出 slots */}
        <div className="wf-node-slot-right">
          {def.hasOutputB ? (
            <>
              <div className="wf-node-slot wf-slot-out">
                <span className="wf-slot-label wf-slot-a">{t(def.outputLabelKey || 'workflow.branchUniform')}</span>
                <Handle type="source" position={Position.Right} id="output-a" className="wf-handle wf-handle-output wf-handle-a" />
              </div>
              <div className="wf-node-slot wf-slot-out">
                <span className="wf-slot-label wf-slot-b">{t(def.outputBLabelKey || 'workflow.branchScattered')}</span>
                <Handle type="source" position={Position.Right} id="output-b" className="wf-handle wf-handle-output wf-handle-b" />
              </div>
            </>
          ) : def.hasOutput && (
            <div className="wf-node-slot wf-slot-out">
              <span className="wf-slot-label">{t(def.outputLabelKey || 'workflow.slotImage')}</span>
              <Handle type="source" position={Position.Right} className="wf-handle wf-handle-output" />
            </div>
          )}
        </div>
      </div>

      {/* 内联参数编辑 */}
      <div className="wf-node-body nodrag nowheel">
        {/* 非 boolean 参数 */}
        {nonBoolParams.map(p => (
          <div key={p.key} className="wf-node-field">
            <label className="wf-node-field-label">{t(p.labelKey)}</label>
            <ParamControl param={p} value={params[p.key]} onChange={updateParam} inline />
          </div>
        ))}

        {/* boolean 参数 - 两列网格 */}
        {boolParams.length > 0 && (
          <div className="wf-node-bool-grid">
            {boolParams.map(p => {
              const disabled = isDisabled(p.key);
              return (
                <div key={p.key} className="wf-node-field-bool">
                  <ParamControl param={p} value={params[p.key]} onChange={updateParam} disabled={disabled} inline />
                </div>
              );
            })}
          </div>
        )}
      </div>
      {/* 进度条 */}
      {data.status === 'running' && data.progressTotal != null && (data.progressTotal as number) > 0 && (
        <div className="wf-node-progress">
          <div className="wf-node-progress-bar">
            <div
              className="wf-node-progress-fill"
              style={{ width: `${Math.min(100, ((data.progressCurrent as number) / (data.progressTotal as number)) * 100)}%` }}
            />
          </div>
          <span className="wf-node-progress-text">{data.progressCurrent}/{data.progressTotal}</span>
        </div>
      )}

      {/* 状态消息（无进度条时显示） */}
      {data.statusMessage && !(data.status === 'running' && data.progressTotal != null && (data.progressTotal as number) > 0) && (
        <div className="wf-node-status-msg">{data.statusMessage}</div>
      )}
    </div>
  );
}

export default memo(BaseNode);
