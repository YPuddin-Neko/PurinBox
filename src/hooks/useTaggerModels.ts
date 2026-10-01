import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TaggerCategory } from '../api/commandOptions';
import type { TaggerModelInfo } from '../utils/taggerModelGroups';
import { defaultTaggerCategories, pruneTaggerCategories } from '../utils/taggerOptions';

export function useTaggerModels(options: { initialId?: string; categories?: TaggerCategory[]; general?: number; character?: number } = {}) {
  const [models, setModels] = useState<TaggerModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState(options.initialId ?? '');
  const [enabled, setEnabled] = useState<Set<TaggerCategory>>(() => options.categories ? new Set(options.categories) : defaultTaggerCategories());
  const [genTh, setGenTh] = useState(options.general ?? 0.55);
  const [charTh, setCharTh] = useState(options.character ?? 0.85);
  const reload = useCallback(async () => {
    const list = await invoke<TaggerModelInfo[]>('get_tagger_models');
    setModels(list);
    setSelectedModel(id => list.some(m => m.id === id) ? id : (list.find(m => m.is_downloaded) ?? list[0])?.id ?? '');
  }, []);
  useEffect(() => { void reload().catch(() => {}); }, [reload]);
  const cur = models.find(m => m.id === selectedModel);
  useEffect(() => {
    if (cur) setEnabled(prev => pruneTaggerCategories(prev, cur.supported_categories));
  }, [cur]);
  const selectModel = (id: string) => {
    const model = models.find(m => m.id === id) ?? models.find(m => m.is_downloaded) ?? models[0];
    setSelectedModel(model?.id ?? '');
    if (model?.general_threshold != null) setGenTh(model.general_threshold);
    if (model?.character_threshold != null) setCharTh(model.character_threshold);
  };
  return { models, selectedModel, setSelectedModel: selectModel, genTh, setGenTh, charTh, setCharTh, enabled, setEnabled, cur, reload };
}
