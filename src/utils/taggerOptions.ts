/**
 * 打标类命令共用的选项表与界面值换算：类别与追加字段的选项表、输出格式拆分、
 * 请求间隔从秒到 request_interval_ms 的换算。
 * 各页面自己的默认值不放在这里。
 */
import {
  JSON_APPEND_FIELD_KEYS,
  TAGGER_CATEGORY_KEYS,
  isOneOf,
  type JsonAppendField,
  type TagFileFormat,
  type TaggerCategory,
} from '../api/commandOptions';

// ── 标签类别 ──

export interface TaggerCategoryDef {
  key: TaggerCategory;
  labelKey: string;
  /** 新建设置时是否默认勾选 */
  defaultOn: boolean;
}

const CATEGORY_META: Record<TaggerCategory, Omit<TaggerCategoryDef, 'key'>> = {
  general: { labelKey: 'aiTagger.catGeneral', defaultOn: true },
  character: { labelKey: 'aiTagger.catCharacter', defaultOn: true },
  rating: { labelKey: 'aiTagger.catRating', defaultOn: false },
  artist: { labelKey: 'aiTagger.catArtist', defaultOn: false },
  style: { labelKey: 'aiTagger.catStyle', defaultOn: false },
  copyright: { labelKey: 'aiTagger.catCopyright', defaultOn: false },
  meta: { labelKey: 'aiTagger.catMeta', defaultOn: false },
  quality: { labelKey: 'aiTagger.catQuality', defaultOn: false },
  model: { labelKey: 'aiTagger.catModel', defaultOn: false },
};

/** 全部 9 个类别，按界面展示顺序 */
export const TAGGER_CATEGORIES: readonly TaggerCategoryDef[] =
  TAGGER_CATEGORY_KEYS.map(key => ({ key, ...CATEGORY_META[key] }));

export function isTaggerCategory(value: unknown): value is TaggerCategory {
  return isOneOf(TAGGER_CATEGORY_KEYS, value);
}

/** 默认勾选的类别（general、character） */
export function defaultTaggerCategories(): Set<TaggerCategory> {
  return new Set(TAGGER_CATEGORIES.filter(c => c.defaultOn).map(c => c.key));
}

/**
 * 切换模型后去掉它不支持的类别。全被去掉时先补回它支持的默认类别，
 * 仍为空再选它支持的第一个类别。supported 为 get_tagger_models 的 supported_categories。
 */
export function pruneTaggerCategories(
  enabled: ReadonlySet<TaggerCategory>,
  supported: readonly string[],
): Set<TaggerCategory> {
  const sup = new Set(supported);
  const next = new Set([...enabled].filter(k => sup.has(k)));
  if (next.size === 0) {
    for (const c of TAGGER_CATEGORIES) if (c.defaultOn && sup.has(c.key)) next.add(c.key);
  }
  if (next.size === 0) {
    const first = supported.find(isTaggerCategory);
    if (first) next.add(first);
  }
  return next;
}

/**
 * 从工作流打标节点的 `cat_<类别>` 布尔参数取出启用的类别。
 * 旧工作流缺少的参数（如 cat_quality、cat_model）按 defaultOn 处理。
 */
export function categoriesFromFlags(params: Readonly<Record<string, unknown>>): TaggerCategory[] {
  return TAGGER_CATEGORIES
    .filter(c => {
      const v = params[`cat_${c.key}`];
      return typeof v === 'boolean' ? v : c.defaultOn;
    })
    .map(c => c.key);
}

// ── JSON 追加字段 ──

export interface JsonAppendFieldDef {
  value: JsonAppendField;
  labelKey: string;
}

/** 与 JSON 标签编辑器同一套字段名 */
const APPEND_FIELD_LABELS: Record<JsonAppendField, string> = {
  tags: 'jsonTag.fieldTags',
  appearance: 'jsonTag.fieldAppearance',
  environment: 'jsonTag.fieldEnvironment',
  quality: 'jsonTag.fieldQuality',
  character: 'jsonTag.fieldCharacter',
  series: 'jsonTag.fieldSeries',
  artist: 'jsonTag.fieldArtist',
  count: 'jsonTag.fieldCount',
};

/** JSON 输出时“追加标签”可选的目标字段，默认 tags */
export const JSON_APPEND_FIELDS: readonly JsonAppendFieldDef[] =
  JSON_APPEND_FIELD_KEYS.map(value => ({ value, labelKey: APPEND_FIELD_LABELS[value] }));

// ── 输出格式 ──

/** 界面上的三选一输出格式；提交时拆成 output_format + json_simplified */
export type TagOutputChoice = 'txt' | 'json' | 'json_simplified';

export function splitOutputFormat(choice: TagOutputChoice): { output_format: TagFileFormat; json_simplified: boolean } {
  return { output_format: choice === 'txt' ? 'txt' : 'json', json_simplified: choice === 'json_simplified' };
}

// ── LLM 请求参数 ──

/**
 * 请求间隔输入框（秒）→ request_interval_ms。
 * 空串、非数字、负数都按“无间隔”返回 -1；不能让 NaN 进 IPC（会变成 null，后端反序列化 i64 失败）。
 */
export function toIntervalMs(seconds: string | number): number {
  const sec = typeof seconds === 'number' ? seconds : parseFloat(seconds);
  if (!Number.isFinite(sec) || sec < 0) return -1;
  return Math.min(Math.round(sec * 1000), Number.MAX_SAFE_INTEGER);
}
