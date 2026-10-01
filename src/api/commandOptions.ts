/**
 * 页面与 WorkflowEngine 共用命令的 options 类型，逐字段对应 src-tauri 里的 Rust 结构体（字段名即 serde 名）。
 * Rust 侧改了结构体，这里同步改。
 *
 * 可选性按 serde 规则写：
 * - 无默认值的字段必填；
 * - 带 `#[serde(default)]` 的字段可省略（省略即用 Rust 默认值），但不能传 null；
 * - `Option<T>` 字段可省略也可传 null。
 *
 * 数字字段经 JSON 传输：NaN / Infinity 会序列化成 null，非 Option 字段会因此反序列化失败；
 * u32 / i32 / i64 字段只接受整数，u32 还不能为负。
 *
 * 用法：在拼装 options 的对象字面量上标注类型，tsc 会报出多余、缺失或拼错的字段：
 *   invoke('scale_images', { options: { ... } satisfies ScaleOptions })
 */

// ── 取值受限的字符串字段 ──

export const SCALE_MODES = ['upscale', 'downscale', 'both'] as const;
export type ScaleMode = (typeof SCALE_MODES)[number];

export const CROP_MODES = ['center', 'cover', 'aspect', 'edges'] as const;
export type CropMode = (typeof CROP_MODES)[number];

export const CROP_ANCHORS = ['center', 'top', 'bottom', 'left', 'right'] as const;
export type CropAnchor = (typeof CROP_ANCHORS)[number];

export const FLIP_DIRECTIONS = ['horizontal', 'vertical', 'both'] as const;
export type FlipDirection = (typeof FLIP_DIRECTIONS)[number];

export const ALPHA_BACKGROUNDS = ['white', 'black'] as const;
export type AlphaBackground = (typeof ALPHA_BACKGROUNDS)[number];

/** 格式转换的目标格式（jpeg 与 jpg 等价） */
export const CONVERT_FORMATS = ['png', 'jpg', 'jpeg', 'bmp', 'webp'] as const;
export type ConvertFormat = (typeof CONVERT_FORMATS)[number];

export const FILTER_ACTIONS = ['copy', 'delete'] as const;
export type FilterAction = (typeof FILTER_ACTIONS)[number];

export const FILTER_CONDITIONS = ['min_width', 'min_height', 'below_resolution', 'above_resolution'] as const;
export type FilterCondition = (typeof FILTER_CONDITIONS)[number];

/** 打标模型的标签类别（后端 normalize_category_value 的全部取值；quality/model 只有 CL 系模型提供） */
export const TAGGER_CATEGORY_KEYS = [
  'general', 'character', 'rating', 'artist', 'style', 'copyright', 'meta', 'quality', 'model',
] as const;
export type TaggerCategory = (typeof TAGGER_CATEGORY_KEYS)[number];

/** 标签文件格式；JSON 的完整/简化布局另由 json_simplified 区分 */
export const TAG_FILE_FORMATS = ['txt', 'json'] as const;
export type TagFileFormat = (typeof TAG_FILE_FORMATS)[number];

export const APPEND_POSITIONS = ['prepend', 'append'] as const;
export type AppendPosition = (typeof APPEND_POSITIONS)[number];

/** JSON 输出时追加标签的目标字段（tagger_inference.py 的 _JSON_APPEND_FIELD_MAP） */
export const JSON_APPEND_FIELD_KEYS = [
  'tags', 'appearance', 'environment', 'quality', 'character', 'series', 'artist', 'count',
] as const;
export type JsonAppendField = (typeof JSON_APPEND_FIELD_KEYS)[number];

export const TAG_SORT_ORDERS = ['confidence', 'frequency'] as const;
export type TagSortOrder = (typeof TAG_SORT_ORDERS)[number];

export const EXISTING_TAGS_ACTIONS = ['overwrite', 'skip', 'prepend', 'append'] as const;
export type ExistingTagsAction = (typeof EXISTING_TAGS_ACTIONS)[number];

/** OpenAI Vision 的 image_url.detail；'' 表示整个字段不发送（取值说明见 utils/imageDetail.ts） */
export const IMAGE_DETAILS = ['', 'auto', 'low', 'high', 'original'] as const;
export type ImageDetail = (typeof IMAGE_DETAILS)[number];

export const BUCKET_MODES = ['legacy', 'nearest_only', 'diffusion_pipe'] as const;
export type BucketMode = (typeof BUCKET_MODES)[number];

/** 把来自下拉框、localStorage 或工作流参数的字符串收窄成上面的取值之一 */
export function isOneOf<T extends string>(values: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (values as readonly string[]).includes(value);
}

// ── 图像处理 ──

/** image_scale.rs → scale_images */
export interface ScaleOptions {
  input_path: string;
  output_path: string;
  mode: ScaleMode;
  target_width: number;
  target_height: number;
  /** 默认 0；mode 为 both 时的下采样目标，0 表示沿用 target_width */
  down_target_width?: number;
  /** 默认 0；mode 为 both 时的下采样目标，0 表示沿用 target_height */
  down_target_height?: number;
  recursive?: boolean;
}

/** image_crop.rs → crop_images */
export interface CropOptions {
  input_path: string;
  output_path: string;
  mode: CropMode;
  /** 默认 'center'；cover 模式保留的方向 */
  crop_anchor?: CropAnchor;
  target_width: number;
  target_height: number;
  /** aspect 模式的宽高比（宽 / 高） */
  aspect_ratio: number;
  crop_top: number;
  crop_bottom: number;
  crop_left: number;
  crop_right: number;
  recursive?: boolean;
}

/** image_flip.rs → flip_images */
export interface FlipOptions {
  input_path: string;
  output_path: string;
  direction: FlipDirection;
  recursive?: boolean;
}

/** perspective.rs → perspective_transform */
export interface PerspectiveOptions {
  input_path: string;
  output_path: string;
  intensity: number;
  recursive?: boolean;
}

/** blur_noise.rs → blur_noise_images */
export interface BlurNoiseOptions {
  input_path: string;
  output_path: string;
  /** f64，0 表示不模糊 */
  blur_radius: number;
  /** u32，0 表示不加噪点 */
  noise_strength: number;
  recursive?: boolean;
}

/** alpha_convert.rs → convert_alpha */
export interface AlphaConvertOptions {
  input_path: string;
  output_path: string;
  background: AlphaBackground;
  recursive?: boolean;
}

/** format_convert.rs → convert_format */
export interface FormatConvertOptions {
  input_path: string;
  output_path: string;
  target_format: ConvertFormat;
  recursive?: boolean;
}

// ── AI 处理 ──

/** upscale.rs → start_upscale */
export interface UpscaleOptions {
  input_path: string;
  output_path: string;
  engine_id: string;
  model_id: string;
  scale: number;
  denoise_level: number;
  tta: boolean;
  /** 负数表示用 CPU */
  gpu_id: number;
  /** NCNN 引擎小于 32 时自动分块 */
  tile_size: number;
  recursive?: boolean;
}

/** person_crop.rs → start_person_crop */
export interface PersonCropOptions {
  input_path: string;
  output_path: string;
  use_gpu: boolean;
  person_enabled: boolean;
  person_conf: number;
  upper_enabled: boolean;
  upper_conf: number;
  upper_tag: string;
  head_enabled: boolean;
  head_conf: number;
  head_tag: string;
  head_scale: number;
  eyes_enabled: boolean;
  eyes_conf: number;
  eyes_tag: string;
  eyes_scale: number;
  keep_original_tags: boolean;
  recursive?: boolean;
}

/** aesthetic.rs → start_aesthetic_scoring */
export interface AestheticOptions {
  input_path: string;
  /** 默认 ''：分级子目录建在原图所在目录下 */
  output_path?: string;
  use_gpu?: boolean;
  /** 默认 false（移动）；工作流必须为 true，否则原图会随临时目录一起被清理 */
  copy_files?: boolean;
  /** 默认 1 */
  batch_size?: number;
  recursive?: boolean;
}

// ── 文件操作 ──

/** resolution_filter.rs → filter_by_resolution */
export interface FilterOptions {
  input_path: string;
  output_path: string;
  action: FilterAction;
  condition: FilterCondition;
  width: number;
  height: number;
  recursive?: boolean;
}

/** batch_rename.rs → preview_rename / execute_rename */
export interface RenameOptions {
  input_path: string;
  prefix: string;
  start_number: number;
  digit_count: number;
  shuffle: boolean;
  /** u64；预览与执行传同一个种子时打乱顺序一致，省略则随机 */
  shuffle_seed?: number | null;
  /** 默认 false */
  rename_tags?: boolean;
}

// ── 打标与标签 ──

/** tagger/mod.rs → start_tagging */
export interface TaggerOptions {
  input_path: string;
  model_id: string;
  general_threshold: number;
  character_threshold: number;
  enabled_categories: readonly TaggerCategory[];
  use_gpu: boolean;
  /** 默认 ''，逗号分隔 */
  exclude_tags?: string;
  /** 默认 ''，逗号分隔 */
  append_tags?: string;
  /** 默认 'append' */
  append_position?: AppendPosition;
  /** 默认 'tags'；只在 output_format 为 json 时有意义 */
  json_append_field?: JsonAppendField;
  /** 默认 true */
  replace_underscore?: boolean;
  /** 默认 'txt' */
  output_format?: TagFileFormat;
  json_simplified?: boolean;
  escape_parentheses?: boolean;
  /** 默认 'confidence' */
  sort_by?: TagSortOrder;
  /** 默认 'overwrite' */
  existing_tags_action?: ExistingTagsAction;
  /** txt 输出且 skip 时，同名 .json 也算已有标签 */
  also_skip_json?: boolean;
  /** 默认 1 */
  batch_size?: number;
  recursive?: boolean;
}

/** tagger/mod.rs ConvertTagsOptions → convert_tags_to_json */
export interface ConvertTagsOptions {
  input_path: string;
  /** 提供标签分类词表的模型（须已下载） */
  model_id: string;
  json_simplified?: boolean;
  recursive?: boolean;
}

/** tagger/llm_tagger.rs → start_llm_tagging */
export interface LlmTaggerOptions {
  input_path: string;
  api_endpoint: string;
  api_key: string;
  model_name: string;
  system_prompt: string;
  user_prompt: string;
  temperature: number;
  /** i32，≤0 时请求里不带 max_tokens */
  max_tokens: number;
  /** 默认 1024；发送前把图片长边缩到此值 */
  image_size?: number;
  /** 默认 0；≤0 时请求里不带 top_p */
  top_p?: number;
  skip_existing?: boolean;
  /** 默认 'txt' */
  output_format?: TagFileFormat;
  json_simplified?: boolean;
  /** i64，默认 -1；≤0 表示无间隔 */
  request_interval_ms?: number;
  /** 默认 1 */
  concurrency?: number;
  recursive?: boolean;
  /** 默认 '' */
  image_detail?: ImageDetail;
}

/** tag_refine.rs → start_tag_refining */
export interface TagRefineOptions {
  input_path: string;
  output_path: string;
  api_endpoint: string;
  api_key: string;
  model_name: string;
  prompt: string;
  temperature: number;
  /** 默认 1024 */
  image_size?: number;
  /** 默认 0；≤0 时请求里不带 top_p */
  top_p?: number;
  /** i64，默认 -1；≤0 表示无间隔 */
  request_interval_ms?: number;
  /** 默认 1 */
  concurrency?: number;
  recursive?: boolean;
  /** 默认 'txt' */
  file_format?: TagFileFormat;
  /** 默认 '' */
  image_detail?: ImageDetail;
  /** 仅 txt：回复整段写入标签文件，不做标签解析 */
  caption_mode?: boolean;
  /** 默认 ''；txt 置于最前，JSON 追加进 artist 字段 */
  trigger_word?: string;
  /** 仅 JSON：只重新归类，标签集合保持不变 */
  preserve_tags?: boolean;
}

/** tag_sort.rs → start_tag_sorting（间隔与并发没有 serde 默认值，必填） */
export interface TagSortOptions {
  input_path: string;
  output_path: string;
  api_endpoint: string;
  api_key: string;
  model_name: string;
  prompt: string;
  temperature: number;
  /** i64，≤0 表示无间隔 */
  request_interval_ms: number;
  concurrency: number;
  /** 默认 0；≤0 时请求里不带 top_p */
  top_p?: number;
}

// ── 分桶 ──

/** bucket_preview.rs → analyze_buckets（可选字段都是 Option，可传 null） */
export interface BucketOptions {
  input_path: string;
  res_width: number;
  res_height: number;
  steps: number;
  no_upscale: boolean;
  min_bucket_reso?: number | null;
  max_bucket_reso?: number | null;
  /** 缺省按 'legacy' */
  bucket_mode?: BucketMode | null;
  recursive?: boolean | null;
  dp_min_ar?: number | null;
  dp_max_ar?: number | null;
  dp_num_ar_buckets?: number | null;
  /** 缺省按 1 */
  batch_size?: number | null;
  /** 缺省时仅 diffusion_pipe 模式丢弃 */
  drop_last?: boolean | null;
}

// ── 命令名与 options 的对应 ──

/** 以 `{ options }` 单参数调用的命令 → 其 options 类型 */
export interface OptionsCommandMap {
  scale_images: ScaleOptions;
  crop_images: CropOptions;
  flip_images: FlipOptions;
  perspective_transform: PerspectiveOptions;
  blur_noise_images: BlurNoiseOptions;
  convert_alpha: AlphaConvertOptions;
  convert_format: FormatConvertOptions;
  start_upscale: UpscaleOptions;
  start_person_crop: PersonCropOptions;
  start_aesthetic_scoring: AestheticOptions;
  filter_by_resolution: FilterOptions;
  preview_rename: RenameOptions;
  execute_rename: RenameOptions;
  start_tagging: TaggerOptions;
  convert_tags_to_json: ConvertTagsOptions;
  start_llm_tagging: LlmTaggerOptions;
  start_tag_refining: TagRefineOptions;
  start_tag_sorting: TagSortOptions;
  analyze_buckets: BucketOptions;
}

export type OptionsCommand = keyof OptionsCommandMap;

/** 命令名与 options 成对出现的可辨识联合，供按节点类型动态选命令的调用方（WorkflowEngine）标注返回值 */
export type OptionsCommandCall = {
  [C in OptionsCommand]: { command: C; options: OptionsCommandMap[C] };
}[OptionsCommand];

/** 批处理命令的返回值（commands/mod.rs ProcessResult；analyze_buckets 与 preview_rename 除外） */
export interface ProcessResult {
  success_count: number;
  fail_count: number;
  total: number;
  errors: string[];
}
