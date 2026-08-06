import { memo, useCallback, useState } from 'react';
import { Handle, Position, useReactFlow, type NodeProps } from '@xyflow/react';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import type { WorkflowNodeData } from '../workflowTypes';
import type { ParamDef } from '../workflowTypes';
import { getNodeDef } from '../nodeDefinitions';
import { FolderOpen } from 'lucide-react';
import DynamicSelect from './DynamicSelect';

/** 分类参数的 key 前缀 */
const CAT_PREFIX = 'cat_';

function BaseNode({ id, data, selected }: NodeProps & { data: WorkflowNodeData }) {
  const { t } = useTranslation();
  const { setNodes } = useReactFlow();
  const def = getNodeDef(data.type);

  // 动态加载的模型列表（用于 tagger 分类过滤）
  const [dynamicItems, setDynamicItems] = useState<any[]>([]);

  if (!def) return null;

  const statusClass = `wf-node-status-${data.status}`;

  const updateParam = useCallback((key: string, value: any) => {
    setNodes(nds => nds.map(n => {
      if (n.id !== id) return n;
      return { ...n, data: { ...n.data, params: { ...(n.data as WorkflowNodeData).params, [key]: value } } };
    }));
  }, [id, setNodes]);

  const handlePathSelect = useCallback(async (key: string) => {
    const selected = await open({ directory: true, multiple: false, title: t('workflow.selectFolder') });
    if (selected) updateParam(key, selected);
  }, [t, updateParam]);

  // 根据当前选中模型获取支持的分类
  const getSupportedCategories = (): Set<string> | null => {
    if (data.type !== 'tagger' || dynamicItems.length === 0) return null;
    const modelId = data.params.model_id || def.params.find(p => p.key === 'model_id')?.default;
    const model = dynamicItems.find((m: any) => m.id === modelId);
    if (!model?.supported_categories) return null;
    return new Set(model.supported_categories as string[]);
  };

  const supportedCats = getSupportedCategories();

  const isCatDisabled = (paramKey: string): boolean => {
    if (!supportedCats) return false;
    if (!paramKey.startsWith(CAT_PREFIX)) return false;
    const cat = paramKey.slice(CAT_PREFIX.length);
    return !supportedCats.has(cat);
  };

  const renderControl = (p: ParamDef, disabled = false) => {
    const val = data.params[p.key] ?? p.default;

    switch (p.type) {
      case 'path':
        return (
          <div className="wf-inline-path">
            <input
              className="wf-inline-input"
              value={String(val)}
              onChange={e => updateParam(p.key, e.target.value)}
              placeholder={t('workflow.selectFolder')}
            />
            <button
              className="wf-inline-path-btn"
              onClick={(e) => { e.stopPropagation(); handlePathSelect(p.key); }}
            >
              <FolderOpen size={13} />
            </button>
          </div>
        );

      case 'number':
        return (
          <input
            className="wf-inline-input wf-inline-number"
            type="number"
            value={Number(val)}
            min={p.min}
            max={p.max}
            step={p.step}
            onChange={e => updateParam(p.key, Number(e.target.value))}
          />
        );

      case 'boolean':
        return (
          <label className={`wf-inline-checkbox ${disabled ? 'wf-inline-checkbox-disabled' : ''}`}>
            <input
              type="checkbox"
              checked={disabled ? false : Boolean(val)}
              disabled={disabled}
              onChange={e => updateParam(p.key, e.target.checked)}
            />
            <span className="wf-inline-checkbox-text">{t(p.labelKey)}</span>
          </label>
        );

      case 'select':
        return (
          <select
            className="wf-inline-select"
            value={String(val)}
            onChange={e => updateParam(p.key, e.target.value)}
          >
            {p.options?.map(opt => (
              <option key={opt.value} value={opt.value}>{t(opt.labelKey)}</option>
            ))}
          </select>
        );

      case 'dynamic-select':
        return (
          <DynamicSelect
            param={p}
            value={String(val)}
            onChange={v => updateParam(p.key, v)}
            onItemsLoaded={setDynamicItems}
          />
        );

      case 'string':
      default:
        return (
          <input
            className="wf-inline-input"
            value={String(val)}
            onChange={e => updateParam(p.key, e.target.value)}
          />
        );
    }
  };

  // 分离 boolean 和非 boolean 参数
  const nonBoolParams = def.params.filter(p => p.type !== 'boolean');
  const boolParams = def.params.filter(p => p.type === 'boolean');
  // 是否有任何 slot 要显示
  const hasSlots = def.hasInput || def.hasOutput;

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
      {hasSlots && (
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
      )}

      {/* 内联参数编辑 */}
      {def.params.length > 0 && (
        <div className="wf-node-body nodrag nowheel">
          {/* 非 boolean 参数 */}
          {nonBoolParams.map(p => (
            <div key={p.key} className="wf-node-field">
              <label className="wf-node-field-label">{t(p.labelKey)}</label>
              {renderControl(p)}
            </div>
          ))}

          {/* boolean 参数 - 两列网格 */}
          {boolParams.length > 0 && (
            <div className="wf-node-bool-grid">
              {boolParams.map(p => {
                const disabled = isCatDisabled(p.key);
                return (
                  <div key={p.key} className="wf-node-field-bool">
                    {renderControl(p, disabled)}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
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
