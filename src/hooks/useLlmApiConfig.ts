import { useCallback, useEffect, useRef, useState } from 'react';
import {
  DEFAULT_LLM_PRESET,
  fetchLlmModels,
  loadLlmApiConfig,
  resolveEndpoint,
  saveLlmApiConfig,
  type LlmPresetId,
} from '../utils/llmPresets';

export type LlmFetchModelsResult = { ok: true; count: number } | { ok: false; error: string };
export type LlmSaveConfigResult = { ok: true } | { ok: false; error: string };

const FETCH_RESULT_MS = 3000;
const SAVE_RESULT_MS = 2000;
const CONFIG_SAVED_EVENT = 'purin:llm-api-config-saved';

export interface UseLlmApiConfigOptions {
  /** 模型名初值（辅助打标从上次的设置回填），只在首次渲染时读取 */
  initialModelName?: string;
}

export interface LlmApiConfigController {
  preset: LlmPresetId;
  setPreset: (preset: LlmPresetId) => void;
  customEndpoint: string;
  setCustomEndpoint: (url: string) => void;
  /** 当前预设的端点基址（custom 时为自定义地址） */
  endpoint: string;
  /** 当前预设的 API Key；每个预设各存一份，切换预设时跟着换 */
  apiKey: string;
  setApiKey: (key: string) => void;
  modelName: string;
  setModelName: (name: string) => void;
  /** 最近一次获取到的模型列表；为空时面板显示手填输入框 */
  modelList: readonly string[];
  fetchingModels: boolean;
  /** 获取模型列表的结果，3 秒后清除 */
  fetchResult: LlmFetchModelsResult | null;
  /** 保存配置的结果，2 秒后清除 */
  saveResult: LlmSaveConfigResult | null;
  /** 拉取模型列表；当前模型名不在列表里时改选第一个 */
  fetchModels: () => Promise<void>;
  /** 保存预设、自定义端点和各预设的 key */
  saveConfig: () => Promise<void>;
  /** 端点和模型名都已填写 */
  ready: boolean;
}

/** 定时清除的结果提示：再次设置会重新计时 */
function useTimedResult<T>(durationMs: number): [T | null, (value: T) => void] {
  const [value, setValue] = useState<T | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timerRef.current), []);
  const show = useCallback((next: T) => {
    setValue(next);
    clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setValue(null), durationMs);
  }, [durationMs]);
  return [value, show];
}

/**
 * LLM API 设置的状态与操作，配合 LlmApiPanel 使用。
 * 保存后通知其他保活面板刷新配置，模型名仍由各面板单独维护。
 */
export function useLlmApiConfig(options: UseLlmApiConfigOptions = {}): LlmApiConfigController {
  const [preset, setPreset] = useState<LlmPresetId>(DEFAULT_LLM_PRESET);
  const [customEndpoint, setCustomEndpoint] = useState('');
  const [apiKeys, setApiKeys] = useState<Record<string, string>>({});
  const [modelName, setModelName] = useState(options.initialModelName ?? '');
  const [modelList, setModelList] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchResult, showFetchResult] = useTimedResult<LlmFetchModelsResult>(FETCH_RESULT_MS);
  const [saveResult, showSaveResult] = useTimedResult<LlmSaveConfigResult>(SAVE_RESULT_MS);

  useEffect(() => {
    let active = true;
    let request = 0;
    const reload = () => {
      const id = ++request;
      void loadLlmApiConfig()
      .then(cfg => {
        if (!active || id !== request) return;
        setPreset(cfg.preset);
        setCustomEndpoint(cfg.customEndpoint);
        setApiKeys(cfg.apiKeys);
        setModelList([]);
      })
      .catch(() => { /* 读取失败沿用默认值，用户仍可手填后保存 */ });
    };
    reload();
    window.addEventListener(CONFIG_SAVED_EVENT, reload);
    return () => { active = false; window.removeEventListener(CONFIG_SAVED_EVENT, reload); };
  }, []);

  const endpoint = resolveEndpoint(preset, customEndpoint);
  const apiKey = apiKeys[preset] ?? '';

  const setApiKey = useCallback((key: string) => {
    setApiKeys(prev => ({ ...prev, [preset]: key }));
  }, [preset]);

  const fetchModels = useCallback(async () => {
    if (!endpoint) return;
    setFetchingModels(true);
    try {
      const models = await fetchLlmModels(endpoint, apiKey);
      setModelList(models);
      if (models.length > 0) setModelName(cur => (models.includes(cur) ? cur : models[0]));
      showFetchResult({ ok: true, count: models.length });
    } catch (e) {
      showFetchResult({ ok: false, error: String(e) });
    } finally {
      setFetchingModels(false);
    }
  }, [endpoint, apiKey, showFetchResult]);

  const saveConfig = useCallback(async () => {
    try {
      await saveLlmApiConfig({ preset, customEndpoint, apiKeys });
      window.dispatchEvent(new Event(CONFIG_SAVED_EVENT));
      showSaveResult({ ok: true });
    } catch (e) {
      showSaveResult({ ok: false, error: String(e) });
    }
  }, [preset, customEndpoint, apiKeys, showSaveResult]);

  return {
    preset,
    setPreset,
    customEndpoint,
    setCustomEndpoint,
    endpoint,
    apiKey,
    setApiKey,
    modelName,
    setModelName,
    modelList,
    fetchingModels,
    fetchResult,
    saveResult,
    fetchModels,
    saveConfig,
    ready: !!endpoint && !!modelName,
  };
}
