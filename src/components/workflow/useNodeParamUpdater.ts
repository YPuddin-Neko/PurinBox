import { useCallback } from 'react';
import { useReactFlow } from '@xyflow/react';
import type { WorkflowNodeData } from './workflowTypes';
import { getNodeDef, withDefaults } from './nodeDefinitions';
import { categoriesFromFlags, pruneTaggerCategories, TAGGER_CATEGORIES } from '../../utils/taggerOptions';
import { switchDefaultPrompts } from '../../utils/llmPrompts';
import { useDynamicItems } from './useDynamicItems';

export function useNodeParamUpdater(id: string, data: WorkflowNodeData) {
  const { setNodes } = useReactFlow();
  const { items } = useDynamicItems(data.type === 'tagger' ? 'get_tagger_models' : undefined);
  const params = withDefaults(getNodeDef(data.type), data.params);
  const model = items.find(item => item.id === params.model_id);
  const supported = model?.supported_categories as string[] | undefined;
  const isDisabled = (key: string) => !!supported && key.startsWith('cat_') && !supported.includes(key.slice(4));

  const updateParam = useCallback((key: string, value: any) => {
    setNodes(nodes => nodes.map(node => {
      if (node.id !== id) return node;
      const current = node.data as WorkflowNodeData;
      const previous = withDefaults(getNodeDef(current.type), current.params);
      const updates = { ...previous, [key]: value };
      if (current.type === 'tagger' && key === 'model_id') {
        const selected = items.find(item => item.id === value);
        if (selected?.general_threshold != null) updates.general_threshold = selected.general_threshold;
        if (selected?.character_threshold != null) updates.character_threshold = selected.character_threshold;
        if (selected?.supported_categories) {
          const enabled = pruneTaggerCategories(new Set(categoriesFromFlags(previous)), selected.supported_categories);
          for (const category of TAGGER_CATEGORIES) updates[`cat_${category.key}`] = enabled.has(category.key);
        }
      }
      if (current.type === 'llm-tagger' && key === 'output_format') {
        const prompts = switchDefaultPrompts(
          { sys: previous.system_prompt, user: previous.user_prompt },
          value === 'txt' ? 'txt' : 'json',
          value === 'json_simplified',
        );
        updates.system_prompt = prompts.sys;
        updates.user_prompt = prompts.user;
      }
      return { ...node, data: { ...current, params: withDefaults(getNodeDef(current.type), updates) } };
    }));
  }, [id, items, setNodes]);
  return { params, updateParam, isDisabled };
}
