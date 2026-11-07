import { useCallback } from 'react';
import { invoke as nativeInvoke } from '@tauri-apps/api/core';
import { useTaskQueue } from '../components/TaskContext';
import { QUEUED_COMMANDS } from '../queuedCommands';
import { listen } from '../utils/tauriRuntime';
import i18n from '../i18n';
import { createQueuedInvoker } from '../utils/queuedInvoker';

export function useQueuedInvoke(): typeof nativeInvoke {
  const actions = useTaskQueue();
  return useCallback(createQueuedInvoker({
    invoke: nativeInvoke, listen, actions, commands: QUEUED_COMMANDS,
    name: command => i18n.t(`queuedTasks.${command}`),
  }), [actions]);
}
