import { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { getNodeDefsByCategory, CATEGORY_COLORS } from './nodeDefinitions';
import type { NodeCategory } from './workflowTypes';
import { ChevronRight, ChevronDown } from 'lucide-react';

const CATEGORY_ORDER: NodeCategory[] = ['input', 'process', 'ai', 'tag', 'condition', 'file', 'output'];

const CATEGORY_LABEL_KEYS: Record<NodeCategory, string> = {
  input: 'workflow.catInput',
  process: 'workflow.catProcess',
  ai: 'workflow.catAI',
  tag: 'workflow.catTag',
  analysis: 'workflow.catAnalysis',
  file: 'workflow.catFile',
  condition: 'workflow.catCondition',
  output: 'workflow.catOutput',
};

interface Props {
  onAddNode: (nodeType: string) => void;
  onDragStarted?: (nodeType: string) => void;
  onDragEnded?: () => void;
}

export default function NodePanel({ onAddNode, onDragStarted, onDragEnded }: Props) {
  const { t } = useTranslation();
  const groups = getNodeDefsByCategory();
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [dragging, setDragging] = useState(false);

  const toggle = (cat: string) => setExpanded(prev => ({ ...prev, [cat]: !prev[cat] }));

  // mousedown 开始拖拽
  const handleMouseDown = useCallback((nodeType: string) => {
    setDragging(true);
    onDragStarted?.(nodeType);

    const handleGlobalMouseUp = () => {
      setDragging(false);
      onDragEnded?.();
      window.removeEventListener('mouseup', handleGlobalMouseUp);
    };
    window.addEventListener('mouseup', handleGlobalMouseUp);
  }, [onDragStarted, onDragEnded]);

  // click = 直接添加（只在非拖拽时）
  const handleClick = useCallback((nodeType: string) => {
    if (!dragging) {
      onAddNode(nodeType);
    }
  }, [dragging, onAddNode]);

  return (
    <div className="wf-node-panel">
      <div className="wf-panel-header">{t('workflow.nodeLibrary')}</div>
      <div className="wf-panel-body">
        {CATEGORY_ORDER.map(cat => {
          const nodes = groups[cat];
          if (!nodes?.length) return null;
          const isExpanded = expanded[cat];
          const color = CATEGORY_COLORS[cat];
          return (
            <div key={cat} className="wf-category">
              <div className="wf-category-header" onClick={() => toggle(cat)}>
                {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                <span className="wf-category-dot" style={{ background: color }} />
                <span>{t(CATEGORY_LABEL_KEYS[cat])}</span>
              </div>
              {isExpanded && (
                <div className="wf-category-items">
                  {nodes.map(def => (
                    <div
                      key={def.type}
                      className="wf-node-item"
                      onMouseDown={() => handleMouseDown(def.type)}
                      onClick={() => handleClick(def.type)}
                      style={{ '--item-color': color } as React.CSSProperties}
                    >
                      <span className="wf-node-item-dot" />
                      <span>{t(def.nameKey)}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
