/**
 * LLM 接口预设与已保存的 API 配置。
 *
 * 所有 LLM 功能（VLM 打标、辅助打标、标签排序/细化、工作流 VLM 节点）只走 OpenAI Chat Completions 协议，
 * 预设只是常用服务的兼容端点基址；后端在基址后拼 `chat/completions` 发请求、拼 `models` 拉模型列表。
 */
import { invoke } from '@tauri-apps/api/core';

export const LLM_PRESETS = [
  { id: 'openai', label: 'OpenAI', url: 'https://api.openai.com/v1/' },
  { id: 'gemini', label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai/' },
  { id: 'deepseek', label: 'DeepSeek', url: 'https://api.deepseek.com/v1/' },
] as const;

export type LlmServicePresetId = (typeof LLM_PRESETS)[number]['id'];
/** 'custom' 使用用户填写的端点 */
export type LlmPresetId = LlmServicePresetId | 'custom';

/** 与 Rust ApiConfig::default 一致 */
export const DEFAULT_LLM_PRESET: LlmPresetId = 'openai';

export function isLlmPresetId(value: unknown): value is LlmPresetId {
  return value === 'custom' || LLM_PRESETS.some(p => p.id === value);
}

/**
 * 配置里存的预设名 → 当前支持的预设。空值用默认预设；
 * 已移除的旧预设（如 'vertex'）和未知值回退到 custom，端点取配置里的 custom_endpoint。
 */
export function normalizePreset(raw: string | null | undefined): LlmPresetId {
  if (!raw) return DEFAULT_LLM_PRESET;
  return isLlmPresetId(raw) ? raw : 'custom';
}

/** 预设对应的端点基址；custom（含回退成 custom 的旧预设）返回自定义端点 */
export function resolveEndpoint(preset: string, customEndpoint: string): string {
  const id = normalizePreset(preset);
  if (id === 'custom') return customEndpoint;
  return LLM_PRESETS.find(p => p.id === id)?.url ?? '';
}

/** 实际请求的 chat/completions 地址，拼接规则与后端一致 */
export function chatCompletionsUrl(endpoint: string): string {
  return `${endpoint}${endpoint.endsWith('/') ? '' : '/'}chat/completions`;
}

// ── 已保存的 API 配置 ──

/** load_api_config 的返回（Rust ApiConfigResponse） */
interface ApiConfigResponse {
  preset: string;
  custom_endpoint: string;
  api_keys: Record<string, string>;
}

export interface LlmApiConfig {
  preset: LlmPresetId;
  customEndpoint: string;
  /** 每个预设各存一份 key，键为预设名（可能含已移除预设的旧 key，原样保留） */
  apiKeys: Record<string, string>;
}

/** 读取已保存的配置，预设名已规整；读取失败时抛出后端的错误文本 */
export async function loadLlmApiConfig(): Promise<LlmApiConfig> {
  const cfg = await invoke<ApiConfigResponse>('load_api_config');
  return {
    preset: normalizePreset(cfg.preset),
    customEndpoint: cfg.custom_endpoint,
    apiKeys: cfg.api_keys,
  };
}

/** 保存配置。后端按预设合并 key：没传的预设保留原值，传空串表示清除该预设的 key */
export async function saveLlmApiConfig(config: LlmApiConfig): Promise<void> {
  await invoke('save_api_config', {
    preset: config.preset,
    customEndpoint: config.customEndpoint,
    apiKeys: config.apiKeys,
  });
}

/** GET {endpoint}/models，返回排好序的模型 id */
export function fetchLlmModels(endpoint: string, apiKey: string): Promise<string[]> {
  return invoke<string[]>('fetch_llm_models', { apiEndpoint: endpoint, apiKey });
}

/** 配置当前预设对应的端点与 key（工作流等不经过 useLlmApiConfig 的调用方用） */
export function resolveLlmApi(config: LlmApiConfig): { endpoint: string; apiKey: string } {
  return {
    endpoint: resolveEndpoint(config.preset, config.customEndpoint),
    apiKey: config.apiKeys[config.preset] ?? '',
  };
}
