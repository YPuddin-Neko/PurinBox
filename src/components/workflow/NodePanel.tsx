import { useState, useCallback, useRef, useEffect } from 'react';
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
  const dragged = useRef(false);
  const releaseListeners = useRef<(() => void) | null>(null);
  useEffect(() => () => releaseListeners.current?.(), []);

  const toggle = (cat: string) => setExpanded(prev => ({ ...prev, [cat]: !prev[cat] }));

  const handleMouseDown = useCallback((event: React.MouseEvent, nodeType: string) => {
    if (event.button !== 0) return;
    releaseListeners.current?.();
    dragged.current = false;
    const { clientX, clientY } = event;
    const handleMove = (move: MouseEvent) => {
      if (!dragged.current && Math.hypot(move.clientX - clientX, move.clientY - clientY) >= 4) {
        dragged.current = true;
        onDragStarted?.(nodeType);
      }
    };
    const handleGlobalMouseUp = () => {
      if (dragged.current) onDragEnded?.();
      releaseListeners.current?.();
    };
    releaseListeners.current = () => {
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleGlobalMouseUp);
      releaseListeners.current = null;
    };
    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleGlobalMouseUp);
  }, [onDragStarted, onDragEnded]);

  const handleClick = useCallback((nodeType: string) => {
    if (!dragged.current) {
      onAddNode(nodeType);
    }
  }, [onAddNode]);

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
                      onMouseDown={event => handleMouseDown(event, def.type)}
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
