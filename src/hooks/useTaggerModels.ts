import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TaggerCategory } from '../api/commandOptions';
import type { TaggerModelInfo } from '../utils/taggerModelGroups';
import { defaultTaggerCategories, pruneTaggerCategories } from '../utils/taggerOptions';

const MODELS_CHANGED_EVENT = 'purin:tagger-models-changed';

/**
 * 模型列表有变化（导入、删除、打标时下载）后调用：打标的几个子页常驻挂载，
 * 各自的模型列表都要重新拉取，否则另一子页还选着已删除的模型、看不到新模型。
 */
export function notifyTaggerModelsChanged() {
  window.dispatchEvent(new Event(MODELS_CHANGED_EVENT));
}

export function useTaggerModels(options: { initialId?: string; categories?: TaggerCategory[]; general?: number; character?: number } = {}) {
  const [models, setModels] = useState<TaggerModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState(options.initialId ?? '');
  const [enabled, setEnabled] = useState<Set<TaggerCategory>>(() => options.categories ? new Set(options.categories) : defaultTaggerCategories());
  const [genTh, setGenTh] = useState(options.general ?? 0.55);
  const [charTh, setCharTh] = useState(options.character ?? 0.85);
  const requestRef = useRef(0);
  const reload = useCallback(async () => {
    const request = ++requestRef.current;
    const list = await invoke<TaggerModelInfo[]>('get_tagger_models');
    // 先发的请求晚到时不能盖掉新列表
    if (request !== requestRef.current) return;
    setModels(list);
    setSelectedModel(id => list.some(m => m.id === id) ? id : (list.find(m => m.is_downloaded) ?? list[0])?.id ?? '');
  }, []);
  useEffect(() => {
    const refresh = () => { void reload().catch(() => {}); };
    refresh();
    window.addEventListener(MODELS_CHANGED_EVENT, refresh);
    return () => {
      window.removeEventListener(MODELS_CHANGED_EVENT, refresh);
      requestRef.current++;
    };
  }, [reload]);
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
  return { models, selectedModel, setSelectedModel: selectModel, genTh, setGenTh, charTh, setCharTh, enabled, setEnabled, cur };
}
