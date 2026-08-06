import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import type { Node } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import { getNodeDef } from './nodeDefinitions';
import { FolderOpen } from 'lucide-react';
import DynamicSelect from './nodes/DynamicSelect';

interface Props {
  selectedNode: Node<WorkflowNodeData> | null;
  onParamChange: (nodeId: string, key: string, value: any) => void;
}

export default function PropertyPanel({ selectedNode, onParamChange }: Props) {
  const { t } = useTranslation();

  if (!selectedNode) {
    return (
      <div className="wf-prop-panel">
        <div className="wf-panel-header">{t('workflow.properties')}</div>
        <div className="wf-prop-empty">{t('workflow.noSelection')}</div>
      </div>
    );
  }

  const data = selectedNode.data;
  const def = getNodeDef(data.type);
  if (!def) return null;

  const handlePathSelect = async (key: string) => {
    const selected = await open({ directory: true, multiple: false, title: t('workflow.selectFolder') });
    if (selected) onParamChange(selectedNode.id, key, selected);
  };

  return (
    <div className="wf-prop-panel">
      <div className="wf-panel-header">
        <span className="wf-prop-dot" style={{ background: def.color }} />
        {t(def.nameKey)}
      </div>
      <div className="wf-prop-body">
        {def.params.map(p => (
          <div key={p.key} className="wf-prop-field">
            {p.type !== 'boolean' && <label className="wf-prop-label">{t(p.labelKey)}</label>}
            {p.type === 'path' && (
              <div className="wf-prop-path">
                <input
                  className="form-input"
                  value={String(data.params[p.key] ?? '')}
                  onChange={e => onParamChange(selectedNode.id, p.key, e.target.value)}
                  placeholder={t('workflow.selectFolder')}
                  style={{ flex: 1 }}
                />
                <button className="btn btn-secondary" onClick={() => handlePathSelect(p.key)} style={{ padding: '4px 8px' }}>
                  <FolderOpen size={14} />
                </button>
              </div>
            )}
            {p.type === 'string' && (
              <input
                className="form-input"
                value={String(data.params[p.key] ?? p.default)}
                onChange={e => onParamChange(selectedNode.id, p.key, e.target.value)}
              />
            )}
            {p.type === 'number' && (
              <input
                className="form-input"
                type="number"
                value={Number(data.params[p.key] ?? p.default)}
                min={p.min}
                max={p.max}
                step={p.step}
                onChange={e => onParamChange(selectedNode.id, p.key, Number(e.target.value))}
              />
            )}
            {p.type === 'boolean' && (
              <label className="wf-prop-checkbox">
                <input
                  type="checkbox"
                  checked={Boolean(data.params[p.key] ?? p.default)}
                  onChange={e => onParamChange(selectedNode.id, p.key, e.target.checked)}
                />
                <span>{t(p.labelKey)}</span>
              </label>
            )}
            {p.type === 'select' && (
              <select
                className="form-input"
                value={String(data.params[p.key] ?? p.default)}
                onChange={e => onParamChange(selectedNode.id, p.key, e.target.value)}
              >
                {p.options?.map(opt => (
                  <option key={opt.value} value={opt.value}>{t(opt.labelKey)}</option>
                ))}
              </select>
            )}
            {p.type === 'dynamic-select' && (
              <DynamicSelect
                param={p}
                value={String(data.params[p.key] ?? p.default)}
                onChange={v => onParamChange(selectedNode.id, p.key, v)}
              />
            )}
          </div>
        ))}
        {def.params.length === 0 && (
          <div className="wf-prop-empty" style={{ fontSize: 11 }}>
            {t('workflow.noSelection')}
          </div>
        )}
      </div>
    </div>
  );
}
