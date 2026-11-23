import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import catalog from '../../src-tauri/scripts/gbc_models.json';
import { GBC_DEFAULTS } from '../utils/gbcSettings';

export type ModelRole = keyof typeof catalog;
const modelOptions = (role: ModelRole) => catalog[role].map(({ repo, label }) => ({ value: repo.split('/')[1], label }));
export const GBC_MODEL_OPTIONS = { detector: modelOptions('detector'), embedder: modelOptions('embedder') };
type Selection = Record<ModelRole, string>;
const STORAGE_KEY = 'gbc_model_selection';

export function useGbcModels() {
  const [value, setValue] = useState<Selection>(() => {
    const defaults = { detector: GBC_MODEL_OPTIONS.detector[0].value, embedder: GBC_MODEL_OPTIONS.embedder[0].value };
    try {
      const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}');
      for (const role of ['detector', 'embedder'] as const) {
        if (GBC_MODEL_OPTIONS[role].some(option => option.value === saved?.[role])) defaults[role] = saved[role];
      }
    } catch {}
    return defaults;
  });
  useEffect(() => { try { localStorage.setItem(STORAGE_KEY, JSON.stringify(value)); } catch {} }, [value]);
  return { value, onChange: (role: ModelRole, model: string) => setValue(prev => ({ ...prev, [role]: model })) };
}
export type GbcModels = ReturnType<typeof useGbcModels>;

export function useGbcOptions(useGpu: boolean, models: Selection) {
  const [local, setLocal] = useState({ python_path: '', gpu_device: '' });
  useEffect(() => {
    let active = true;
    void invoke<typeof local>('get_gbc_local_paths').then(paths => {
      if (active) setLocal(paths);
    }).catch(() => {});
    return () => { active = false; };
  }, []);
  return { ...GBC_DEFAULTS, python_path: local.python_path, detector_path: models.detector,
    embedder_path: models.embedder, device: useGpu ? local.gpu_device : 'cpu' };
}
