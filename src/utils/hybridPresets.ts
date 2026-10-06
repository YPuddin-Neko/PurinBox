import type { LlmSampling } from '../components/LlmSamplingFields';
import { IMAGE_DETAILS, SHORT_REPLY_THRESHOLD, isOneOf } from '../api/commandOptions';
import type { TagOutputChoice } from './taggerOptions';
import { storedCount, storedNumber, type HybridSettings } from './hybridSettings';
import {
  HYBRID_PROMPT_DETAILED_CAPTION, HYBRID_PROMPT_JSON, HYBRID_PROMPT_NL_ONLY,
  HYBRID_PROMPT_SORT_ONLY, HYBRID_PROMPT_TXT,
} from './llmPrompts';

const OUTPUT_CHOICES: readonly TagOutputChoice[] = ['txt', 'json', 'json_simplified'];

export interface HybridPromptMode {
  captionMode?: boolean;
  preserveTags?: boolean;
  nlOnly?: boolean;
}

export interface HybridVlmValues {
  prompt: string;
  triggerWord: string;
  shortReplyThreshold: number;
  sampling: LlmSampling;
  outputFormat: TagOutputChoice;
  skipExisting: boolean;
}

export interface HybridPromptPreset extends Partial<HybridVlmValues>, HybridPromptMode {
  id: string;
  name: string;
  prompt: string;
}

export const defaultHybridPrompt = (format: TagOutputChoice) => format === 'txt' ? HYBRID_PROMPT_TXT : HYBRID_PROMPT_JSON;

export function hybridBuiltinPreset(id: string, format: TagOutputChoice) {
  switch (id) {
    case 'builtin_full': return { id, prompt: defaultHybridPrompt(format) };
    case 'builtin_sort': return { id, prompt: HYBRID_PROMPT_SORT_ONLY, preserveTags: true };
    case 'builtin_nl': return { id, prompt: HYBRID_PROMPT_NL_ONLY, nlOnly: true };
    case 'builtin_caption': return { id, prompt: HYBRID_PROMPT_DETAILED_CAPTION, captionMode: true };
    default: return undefined;
  }
}

export function compatibleHybridFormat(mode: HybridPromptMode, format: TagOutputChoice): TagOutputChoice {
  if (mode.captionMode) return 'txt';
  if ((mode.nlOnly || mode.preserveTags) && format === 'txt') return 'json';
  return format;
}

export function parseHybridPresets(raw: string | null): HybridPromptPreset[] {
  try {
    const list: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter((p): p is HybridPromptPreset =>
      p != null && typeof p === 'object' && typeof p.id === 'string' && p.id.length > 0
      && !p.id.startsWith('builtin_') && typeof p.name === 'string' && p.name.length > 0
      && typeof p.prompt === 'string') : [];
  } catch { return []; }
}

const bounded = (value: unknown, fallback: number, min: number, max: number) =>
  Math.min(max, Math.max(min, storedNumber(value, fallback)));

export function applyHybridPreset(preset: HybridPromptPreset, current: HybridVlmValues, preferExisting: boolean): HybridVlmValues {
  const sampling = preset.sampling;
  const format = isOneOf(OUTPUT_CHOICES, preset.outputFormat) ? preset.outputFormat : current.outputFormat;
  return {
    prompt: preset.prompt,
    triggerWord: typeof preset.triggerWord === 'string' ? preset.triggerWord : current.triggerWord,
    shortReplyThreshold: storedCount(preset.shortReplyThreshold, current.shortReplyThreshold, SHORT_REPLY_THRESHOLD.max),
    outputFormat: compatibleHybridFormat(preset, format),
    skipExisting: preferExisting && (typeof preset.skipExisting === 'boolean' ? preset.skipExisting : current.skipExisting),
    sampling: sampling ? {
      temperature: bounded(sampling.temperature, current.sampling.temperature, 0, 2),
      topP: bounded(sampling.topP, current.sampling.topP, 0, 1),
      imageSize: Math.max(256, storedCount(sampling.imageSize, current.sampling.imageSize, 4096)),
      imageDetail: isOneOf(IMAGE_DETAILS, sampling.imageDetail) ? sampling.imageDetail : current.sampling.imageDetail,
      concurrency: storedCount(sampling.concurrency, current.sampling.concurrency, 16),
      intervalSec: bounded(sampling.intervalSec, current.sampling.intervalSec, -1, 120),
    } : { ...current.sampling },
  };
}

export function saveHybridPreset(
  presets: HybridPromptPreset[], name: string, id: string, values: HybridVlmValues, mode: HybridPromptMode,
): HybridPromptPreset[] {
  const existing = presets.find(p => p.name === name);
  const preset: HybridPromptPreset = {
    id: existing?.id ?? id, name,
    prompt: values.prompt, triggerWord: values.triggerWord, shortReplyThreshold: values.shortReplyThreshold,
    sampling: { ...values.sampling }, outputFormat: compatibleHybridFormat(mode, values.outputFormat),
    skipExisting: values.skipExisting,
    captionMode: !!mode.captionMode, preserveTags: !!mode.preserveTags, nlOnly: !!mode.nlOnly,
  };
  return existing ? presets.map(p => p.id === existing.id ? preset : p) : [...presets, preset];
}

export function restoreHybridSelection(saved: HybridSettings, presets: HybridPromptPreset[]) {
  const format = isOneOf(OUTPUT_CHOICES, saved.outputFormat) ? saved.outputFormat : 'txt';
  const preset = presets.find(p => p.id === saved.presetId) ?? hybridBuiltinPreset(saved.presetId ?? 'builtin_full', format);
  if (!preset) return { presetId: 'builtin_full', outputFormat: format, prompt: defaultHybridPrompt(format) };
  const outputFormat = compatibleHybridFormat(preset, format);
  // 恢复编辑中的提示词和参数，不用预设快照覆盖上次未另存的调整。
  return { presetId: preset.id, outputFormat, prompt: typeof saved.prompt === 'string' ? saved.prompt : preset.prompt };
}
