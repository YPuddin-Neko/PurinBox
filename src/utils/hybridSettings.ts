/** 辅助打标上一次使用的设置：重启后回填，免去每次重复设置 */
import type { TagOutputChoice } from './taggerOptions';

const SETTINGS_KEY = 'hybrid_tagger_settings_v1';

/** 数值项沿用字符串存储，与早先版本保存的设置互相兼容 */
export interface HybridSettings {
  modelId?: string;
  genTh?: number;
  charTh?: number;
  useGpu?: boolean;
  replaceUnderscore?: boolean;
  escapeParentheses?: boolean;
  preferExisting?: boolean;
  skipExisting?: boolean;
  enabledCats?: string[];
  modelName?: string;
  temperature?: string;
  topP?: string;
  imageSize?: string;
  imageDetail?: string;
  concurrency?: string;
  intervalSec?: string;
  outputFormat?: TagOutputChoice;
}

/** 没有保存过设置（或内容无法解析）时返回 null */
export function loadHybridSettings(): HybridSettings | null {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as HybridSettings : null;
  } catch {
    return null;
  }
}

export function saveHybridSettings(settings: HybridSettings) {
  try { localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings)); } catch { /* 配额满等，忽略 */ }
}

/**
 * 「跳过已有标签文件」的初值：全新安装默认开启；保存过设置但没有这一项
 * （该选项出现之前保存的）按关闭处理，升级后不改变原来的处理方式。
 * 「优先使用已有标签」关闭时恒为关闭。
 */
export function initialSkipExisting(saved: HybridSettings | null): boolean {
  if (!saved || Object.keys(saved).length === 0) return true;
  return (saved.preferExisting ?? true) && (saved.skipExisting ?? false);
}

/** 保存的数值项（字符串或数字）；缺省或不是有限数时用 fallback */
export function storedNumber(value: unknown, fallback: number): number {
  const n = typeof value === 'number' ? value
    : typeof value === 'string' && value.trim() !== '' ? Number(value)
      : NaN;
  return Number.isFinite(n) ? n : fallback;
}

/**
 * 保存的计数项（并发、图片发送尺寸，后端是 u32）：旧版存的是输入框原文，"2.5"、"-1"、"20" 都可能出现。
 * 取整数部分，缺省、非法或小于 1 时用 fallback，超过 max 时取 max。
 */
export function storedCount(value: unknown, fallback: number, max: number): number {
  const n = typeof value === 'number' ? Math.trunc(value)
    : typeof value === 'string' ? parseInt(value, 10)
      : NaN;
  return Number.isFinite(n) && n >= 1 ? Math.min(n, max) : fallback;
}
