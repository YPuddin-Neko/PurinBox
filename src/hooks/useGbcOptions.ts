import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GBC_DEFAULTS } from '../utils/gbcSettings';

export function useGbcOptions(useGpu: boolean) {
  const [local, setLocal] = useState({ python_path: '', detector_path: '', embedder_path: '', gpu_device: '' });
  useEffect(() => {
    let active = true;
    void invoke<typeof local>('get_gbc_local_paths').then(paths => {
      if (active) setLocal(paths);
    }).catch(() => {});
    return () => { active = false; };
  }, []);
  return { ...GBC_DEFAULTS, python_path: local.python_path, detector_path: local.detector_path,
    embedder_path: local.embedder_path, device: useGpu ? local.gpu_device : 'cpu' };
}
