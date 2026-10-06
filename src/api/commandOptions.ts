/**
 * 前端调用的命令参数类型，逐字段对应 src-tauri 里的 Rust 结构体（字段名即 serde 名）。
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
 * 页面和工作流节点都会调用的命令另有 `XXX_DEFAULTS` 与 `buildXxxOptions(io, settings)`，两边用它们构造参数：
 * - settings 里缺省或为 undefined 的项取 XXX_DEFAULTS，即页面的初始值；
 * - 数字按 Rust 类型规范（非有限值回退默认值，整数字段取整并夹在合法范围内），数字字符串也接受；
 *   取值受限的字符串不合法时回退默认值。
 *
 * 用法：
 *   invoke('crop_images', { options: buildCropOptions({ input_path, output_path, recursive }, { mode, target_width: w }) })
 * 手写对象字面量时用 `satisfies XxxOptions` 让 tsc 报出多余、缺失或拼错的字段。
 */

import { getDefaultPrompts } from '../utils/llmPrompts';

// ── 取值受限的字符串字段 ──

const SCALE_MODES = ['upscale', 'downscale', 'both'] as const;
export type ScaleMode = (typeof SCALE_MODES)[number];

const CROP_MODES = ['center', 'cover', 'aspect', 'edges'] as const;
export type CropMode = (typeof CROP_MODES)[number];

const CROP_ANCHORS = ['center', 'top', 'bottom', 'left', 'right'] as const;
export type CropAnchor = (typeof CROP_ANCHORS)[number];

const FLIP_DIRECTIONS = ['horizontal', 'vertical', 'both'] as const;
export type FlipDirection = (typeof FLIP_DIRECTIONS)[number];

const ALPHA_BACKGROUNDS = ['white', 'black'] as const;
export type AlphaBackground = (typeof ALPHA_BACKGROUNDS)[number];

/** 格式转换的目标格式（jpeg 与 jpg 等价） */
const CONVERT_FORMATS = ['png', 'jpg', 'jpeg', 'bmp', 'webp'] as const;
export type ConvertFormat = (typeof CONVERT_FORMATS)[number];

const FILTER_ACTIONS = ['copy', 'delete'] as const;
export type FilterAction = (typeof FILTER_ACTIONS)[number];

const FILTER_CONDITIONS = ['min_width', 'min_height', 'below_resolution', 'above_resolution'] as const;
export type FilterCondition = (typeof FILTER_CONDITIONS)[number];

/** 打标模型的标签类别（后端 normalize_category_value 的全部取值；quality/model 只有 CL 系模型提供） */
export const TAGGER_CATEGORY_KEYS = [
  'general', 'character', 'rating', 'artist', 'style', 'copyright', 'meta', 'quality', 'model',
] as const;
export type TaggerCategory = (typeof TAGGER_CATEGORY_KEYS)[number];

/** 标签文件格式；JSON 的完整/简化布局另由 json_simplified 区分 */
const TAG_FILE_FORMATS = ['txt', 'json'] as const;
export type TagFileFormat = (typeof TAG_FILE_FORMATS)[number];

const APPEND_POSITIONS = ['prepend', 'append'] as const;
export type AppendPosition = (typeof APPEND_POSITIONS)[number];

/** JSON 输出时追加标签的目标字段（tagger_inference.py 的 _JSON_APPEND_FIELD_MAP） */
export const JSON_APPEND_FIELD_KEYS = [
  'tags', 'appearance', 'environment', 'quality', 'character', 'series', 'artist', 'count',
] as const;
export type JsonAppendField = (typeof JSON_APPEND_FIELD_KEYS)[number];

const TAG_SORT_ORDERS = ['confidence', 'frequency'] as const;
export type TagSortOrder = (typeof TAG_SORT_ORDERS)[number];

const EXISTING_TAGS_ACTIONS = ['overwrite', 'skip', 'prepend', 'append'] as const;
export type ExistingTagsAction = (typeof EXISTING_TAGS_ACTIONS)[number];

/** OpenAI Vision 的 image_url.detail；'' 表示整个字段不发送（取值说明见 utils/imageDetail.ts） */
export const IMAGE_DETAILS = ['', 'auto', 'low', 'high', 'original'] as const;
export type ImageDetail = (typeof IMAGE_DETAILS)[number];

const BUCKET_MODES = ['legacy', 'nearest_only', 'diffusion_pipe'] as const;
export type BucketMode = (typeof BUCKET_MODES)[number];

/** 把来自下拉框、localStorage 或工作流参数的字符串收窄成上面的取值之一 */
export function isOneOf<T extends string>(values: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (values as readonly string[]).includes(value);
}

// ── 构造函数共用的规范化 ──

const U32_MAX = 0xffffffff;
const I32_MAX = 0x7fffffff;

function finite(value: unknown): number | undefined {
  const n = typeof value === 'number' ? value
    : typeof value === 'string' && value.trim() !== '' ? Number(value)
      : NaN;
  return Number.isFinite(n) ? n : undefined;
}

function num(value: unknown, fallback: number, min = -Infinity, max = Infinity): number {
  const n = finite(value);
  return n === undefined ? fallback : Math.min(max, Math.max(min, n));
}

function int(value: unknown, fallback: number, min: number, max: number): number {
  const n = finite(value);
  return n === undefined ? fallback : Math.min(max, Math.max(min, Math.round(n)));
}

function choice<T extends string>(values: readonly T[], value: unknown, fallback: T): T {
  return isOneOf(values, value) ? value : fallback;
}

function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function text(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback;
}

/** 输入、输出与递归扫描：页面来自路径面板，工作流节点来自引擎 */
export interface CommandIO {
  input_path: string;
  output_path: string;
  recursive?: boolean;
}

/** 没有输出目录的命令（打标、重命名、分桶） */
export interface InputIO {
  input_path: string;
  recursive?: boolean;
}

function ioFields(io: CommandIO) {
  return { input_path: io.input_path, output_path: io.output_path, recursive: io.recursive === true };
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

export interface ScaleSettings {
  mode: ScaleMode;
  /** mode 为 upscale / both 时的目标尺寸 */
  upscale_width: number;
  upscale_height: number;
  /** mode 为 downscale / both 时的目标尺寸 */
  downscale_width: number;
  downscale_height: number;
}

export const SCALE_DEFAULTS: Readonly<ScaleSettings> = {
  mode: 'upscale',
  upscale_width: 1024,
  upscale_height: 1024,
  downscale_width: 512,
  downscale_height: 512,
};

/**
 * mode 为 downscale 时目标取下采样尺寸，否则取上采样尺寸；只有 both 才发下采样目标。
 * 只有一组宽高的调用方（工作流节点）把两组都传成它。
 */
export function buildScaleOptions(io: CommandIO, settings: Partial<ScaleSettings> = {}): ScaleOptions {
  const d = SCALE_DEFAULTS;
  const mode = choice(SCALE_MODES, settings.mode, d.mode);
  const upWidth = int(settings.upscale_width, d.upscale_width, 1, U32_MAX);
  const upHeight = int(settings.upscale_height, d.upscale_height, 1, U32_MAX);
  const downWidth = int(settings.downscale_width, d.downscale_width, 1, U32_MAX);
  const downHeight = int(settings.downscale_height, d.downscale_height, 1, U32_MAX);
  return {
    ...ioFields(io),
    mode,
    target_width: mode === 'downscale' ? downWidth : upWidth,
    target_height: mode === 'downscale' ? downHeight : upHeight,
    down_target_width: mode === 'both' ? downWidth : 0,
    down_target_height: mode === 'both' ? downHeight : 0,
  };
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

export type CropSettings = Required<Omit<CropOptions, keyof CommandIO>>;

export const CROP_DEFAULTS: Readonly<CropSettings> = {
  mode: 'center',
  crop_anchor: 'center',
  target_width: 1024,
  target_height: 1024,
  aspect_ratio: 1,
  crop_top: 0,
  crop_bottom: 0,
  crop_left: 0,
  crop_right: 0,
};

export function buildCropOptions(io: CommandIO, settings: Partial<CropSettings> = {}): CropOptions {
  const d = CROP_DEFAULTS;
  const ratio = num(settings.aspect_ratio, d.aspect_ratio);
  return {
    ...ioFields(io),
    mode: choice(CROP_MODES, settings.mode, d.mode),
    crop_anchor: choice(CROP_ANCHORS, settings.crop_anchor, d.crop_anchor),
    target_width: int(settings.target_width, d.target_width, 1, U32_MAX),
    target_height: int(settings.target_height, d.target_height, 1, U32_MAX),
    aspect_ratio: ratio > 0 ? ratio : d.aspect_ratio,
    crop_top: int(settings.crop_top, d.crop_top, 0, U32_MAX),
    crop_bottom: int(settings.crop_bottom, d.crop_bottom, 0, U32_MAX),
    crop_left: int(settings.crop_left, d.crop_left, 0, U32_MAX),
    crop_right: int(settings.crop_right, d.crop_right, 0, U32_MAX),
  };
}

/** image_flip.rs → flip_images */
export interface FlipOptions {
  input_path: string;
  output_path: string;
  direction: FlipDirection;
  recursive?: boolean;
}

export type FlipSettings = Required<Omit<FlipOptions, keyof CommandIO>>;

export const FLIP_DEFAULTS: Readonly<FlipSettings> = { direction: 'horizontal' };

export function buildFlipOptions(io: CommandIO, settings: Partial<FlipSettings> = {}): FlipOptions {
  return { ...ioFields(io), direction: choice(FLIP_DIRECTIONS, settings.direction, FLIP_DEFAULTS.direction) };
}

/** perspective.rs → perspective_transform */
export interface PerspectiveOptions {
  input_path: string;
  output_path: string;
  /** 0 ~ 0.5，超出范围后端拒绝 */
  intensity: number;
  recursive?: boolean;
}

export type PerspectiveSettings = Required<Omit<PerspectiveOptions, keyof CommandIO>>;

export const PERSPECTIVE_DEFAULTS: Readonly<PerspectiveSettings> = { intensity: 0.1 };

export function buildPerspectiveOptions(io: CommandIO, settings: Partial<PerspectiveSettings> = {}): PerspectiveOptions {
  return { ...ioFields(io), intensity: num(settings.intensity, PERSPECTIVE_DEFAULTS.intensity, 0, 0.5) };
}

/** blur_noise.rs → blur_noise_images */
export interface BlurNoiseOptions {
  input_path: string;
  output_path: string;
  /** f64，0 表示不模糊 */
  blur_radius: number;
  /** u32，0 ~ 100，0 表示不加噪点 */
  noise_strength: number;
  recursive?: boolean;
}

export type BlurNoiseSettings = Required<Omit<BlurNoiseOptions, keyof CommandIO>>;

export const BLUR_NOISE_DEFAULTS: Readonly<BlurNoiseSettings> = { blur_radius: 2, noise_strength: 15 };

export function buildBlurNoiseOptions(io: CommandIO, settings: Partial<BlurNoiseSettings> = {}): BlurNoiseOptions {
  const d = BLUR_NOISE_DEFAULTS;
  return {
    ...ioFields(io),
    blur_radius: num(settings.blur_radius, d.blur_radius, 0),
    noise_strength: int(settings.noise_strength, d.noise_strength, 0, 100),
  };
}

/** alpha_convert.rs → convert_alpha */
export interface AlphaConvertOptions {
  input_path: string;
  output_path: string;
  background: AlphaBackground;
  recursive?: boolean;
}

export type AlphaConvertSettings = Required<Omit<AlphaConvertOptions, keyof CommandIO>>;

export const ALPHA_CONVERT_DEFAULTS: Readonly<AlphaConvertSettings> = { background: 'white' };

export function buildAlphaConvertOptions(io: CommandIO, settings: Partial<AlphaConvertSettings> = {}): AlphaConvertOptions {
  return { ...ioFields(io), background: choice(ALPHA_BACKGROUNDS, settings.background, ALPHA_CONVERT_DEFAULTS.background) };
}

/** format_convert.rs → convert_format */
export interface FormatConvertOptions {
  input_path: string;
  output_path: string;
  target_format: ConvertFormat;
  recursive?: boolean;
}

export type FormatConvertSettings = Required<Omit<FormatConvertOptions, keyof CommandIO>>;

export const FORMAT_CONVERT_DEFAULTS: Readonly<FormatConvertSettings> = { target_format: 'png' };

export function buildFormatConvertOptions(io: CommandIO, settings: Partial<FormatConvertSettings> = {}): FormatConvertOptions {
  return { ...ioFields(io), target_format: choice(CONVERT_FORMATS, settings.target_format, FORMAT_CONVERT_DEFAULTS.target_format) };
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
  /** 小于 32 时自动分块（NCNN）或不分块（Python 引擎） */
  tile_size: number;
  recursive?: boolean;
}

export type UpscaleEngineId = 'realcugan' | 'realesrgan' | 'waifu2x';

/** 各引擎的默认模型：get_upscale_engines 返回的模型列表里的第一个 */
const UPSCALE_DEFAULT_MODELS: Readonly<Record<UpscaleEngineId, string>> = {
  realcugan: 'models-se',
  realesrgan: 'realesrgan-x4plus',
  waifu2x: 'models-cunet',
};

export interface UpscaleSettings {
  engine_id: string;
  /** 空串取引擎的默认模型（UPSCALE_DEFAULT_MODELS） */
  model_id: string;
  scale: number;
  /** -1 ~ 3；不支持降噪的引擎忽略 */
  denoise_level: number;
  tta: boolean;
  /** false 时 gpu_id 发 -1（CPU） */
  use_gpu: boolean;
  tile_size: number;
}

export const UPSCALE_DEFAULTS: Readonly<UpscaleSettings> = {
  engine_id: 'realcugan',
  model_id: '',
  scale: 2,
  denoise_level: -1,
  tta: false,
  use_gpu: true,
  tile_size: -1,
};

export function buildUpscaleOptions(io: CommandIO, settings: Partial<UpscaleSettings> = {}): UpscaleOptions {
  const d = UPSCALE_DEFAULTS;
  const engineId = text(settings.engine_id, d.engine_id) || d.engine_id;
  const defaultModel = (UPSCALE_DEFAULT_MODELS as Readonly<Record<string, string>>)[engineId] ?? '';
  return {
    ...ioFields(io),
    engine_id: engineId,
    model_id: text(settings.model_id, d.model_id) || defaultModel,
    scale: int(settings.scale, d.scale, 1, U32_MAX),
    denoise_level: int(settings.denoise_level, d.denoise_level, -1, 3),
    tta: bool(settings.tta, d.tta),
    gpu_id: bool(settings.use_gpu, d.use_gpu) ? 0 : -1,
    tile_size: int(settings.tile_size, d.tile_size, -1, I32_MAX),
  };
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

export type PersonCropSettings = Required<Omit<PersonCropOptions, keyof CommandIO>>;

/** 人物裁切页面的初始值；工作流节点没有暴露的项也用它 */
export const PERSON_CROP_DEFAULTS: Readonly<PersonCropSettings> = {
  use_gpu: false,
  person_enabled: true,
  person_conf: 0.3,
  upper_enabled: true,
  upper_conf: 0.5,
  upper_tag: 'upper body',
  head_enabled: true,
  head_conf: 0.4,
  head_tag: 'head view',
  head_scale: 1.5,
  eyes_enabled: true,
  eyes_conf: 0.3,
  eyes_tag: 'eyes view',
  eyes_scale: 2.4,
  keep_original_tags: false,
};

export function buildPersonCropOptions(io: CommandIO, settings: Partial<PersonCropSettings> = {}): PersonCropOptions {
  const d = PERSON_CROP_DEFAULTS;
  const conf = (value: unknown, fallback: number) => num(value, fallback, 0, 1);
  return {
    ...ioFields(io),
    use_gpu: bool(settings.use_gpu, d.use_gpu),
    person_enabled: bool(settings.person_enabled, d.person_enabled),
    person_conf: conf(settings.person_conf, d.person_conf),
    upper_enabled: bool(settings.upper_enabled, d.upper_enabled),
    upper_conf: conf(settings.upper_conf, d.upper_conf),
    upper_tag: text(settings.upper_tag, d.upper_tag),
    head_enabled: bool(settings.head_enabled, d.head_enabled),
    head_conf: conf(settings.head_conf, d.head_conf),
    head_tag: text(settings.head_tag, d.head_tag),
    head_scale: num(settings.head_scale, d.head_scale, 1),
    eyes_enabled: bool(settings.eyes_enabled, d.eyes_enabled),
    eyes_conf: conf(settings.eyes_conf, d.eyes_conf),
    eyes_tag: text(settings.eyes_tag, d.eyes_tag),
    eyes_scale: num(settings.eyes_scale, d.eyes_scale, 1),
    keep_original_tags: bool(settings.keep_original_tags, d.keep_original_tags),
  };
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

export type AestheticSettings = Required<Pick<AestheticOptions, 'use_gpu' | 'copy_files' | 'batch_size'>>;

/** use_gpu：页面在 macOS 上初始为 false，其余平台为 true */
export const AESTHETIC_DEFAULTS: Readonly<AestheticSettings> = { use_gpu: true, copy_files: false, batch_size: 1 };

/** batch_size 只在 GPU 下生效，CPU 恒为 1；output_path 为空时分级目录建在原图旁 */
export function buildAestheticOptions(
  io: InputIO & { output_path?: string },
  settings: Partial<AestheticSettings> = {},
): AestheticOptions {
  const d = AESTHETIC_DEFAULTS;
  const useGpu = bool(settings.use_gpu, d.use_gpu);
  return {
    input_path: io.input_path,
    output_path: io.output_path ?? '',
    recursive: io.recursive === true,
    use_gpu: useGpu,
    copy_files: bool(settings.copy_files, d.copy_files),
    batch_size: useGpu ? int(settings.batch_size, d.batch_size, 1, 64) : 1,
  };
}

/** image_cluster.rs → start_image_cluster */
export interface ClusterOptions {
  input_path: string;
  output_path: string;
  algorithm: 'kmeans' | 'hdbscan';
  feature_type: 'style' | 'semantic' | 'fusion';
  /** u32，K-Means 的分组数 */
  n_clusters: number;
  /** u32，HDBSCAN 的最小簇大小 */
  min_cluster_size: number;
  /** auto 时有 GPU 就用 */
  device: 'auto' | 'cpu';
  /** feature_type 为 fusion 时三种特征的权重 */
  weight_style: number;
  weight_semantic: number;
  weight_color: number;
  /** 分布图的配色 */
  map_theme: 'light' | 'dark';
  recursive?: boolean;
}

// ── 去重与查重重命名 ──

/** image_dedup.rs → start_image_dedup */
export interface DedupOptions {
  folder_path: string;
  /** u32 */
  dhash_threshold: number;
  /** u32 */
  phash_threshold: number;
  /** 0 ~ 1 */
  color_threshold: number;
  recursive?: boolean;
}

export interface DupGroup {
  paths: string[];
  method: string;
}

/** start_image_dedup 的返回值 */
export interface DedupResult {
  total_images: number;
  duplicate_groups: DupGroup[];
  scan_time_ms: number;
  /** 指纹计算失败的文件（路径 + 原因） */
  failed_files: string[];
}

/** delete_dedup_files({ paths }) 的返回值；errors 每项为「路径: 原因」 */
export interface DedupDeleteResult {
  deleted: number;
  failed: number;
  errors: string[];
}

/** dedup_rename.rs → scan_dedup_rename（只扫两个文件夹的顶层） */
export interface DedupRenameOptions {
  folder_a: string;
  folder_b: string;
  dhash_threshold: number;
  phash_threshold: number;
  color_threshold: number;
}

export interface DedupPair {
  path_a: string;
  name_a: string;
  path_b: string;
  name_b: string;
}

/** scan_dedup_rename 的返回值 */
export interface DedupRenameScanResult {
  pairs: DedupPair[];
  total_a: number;
  total_b: number;
  unmatched_a: string[];
  unmatched_b: string[];
  scan_time_ms: number;
  /** 指纹计算失败的文件（路径 + 原因） */
  failed_files: string[];
}

/** execute_dedup_rename({ actions }) 的一项 */
export interface RenameAction {
  /** 要改名的文件 */
  src_path: string;
  /** 新文件名（不含目录） */
  target_name: string;
  /** 目标名被它占用时，先把它改名为 `{stem}_rename`（已占用时追加序号）让位 */
  conflict_path?: string | null;
}

/** execute_dedup_rename、export_unmatched_files 的返回值 */
export interface DedupRenameResult {
  success_count: number;
  fail_count: number;
  errors: string[];
}

/** export_unmatched_files 的参数（独立参数，不包在 options 里） */
export interface ExportUnmatchedArgs {
  sourceFolder: string;
  /** 源文件夹里的文件名 */
  filenames: string[];
  destFolder: string;
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

export type FilterSettings = Required<Omit<FilterOptions, keyof CommandIO>>;

export const FILTER_DEFAULTS: Readonly<FilterSettings> = {
  action: 'copy',
  condition: 'below_resolution',
  width: 512,
  height: 512,
};

/** 删除模式不需要输出目录：output_path 为空时发输入路径 */
export function buildFilterOptions(io: CommandIO, settings: Partial<FilterSettings> = {}): FilterOptions {
  const d = FILTER_DEFAULTS;
  return {
    ...ioFields(io),
    output_path: io.output_path || io.input_path,
    action: choice(FILTER_ACTIONS, settings.action, d.action),
    condition: choice(FILTER_CONDITIONS, settings.condition, d.condition),
    width: int(settings.width, d.width, 1, U32_MAX),
    height: int(settings.height, d.height, 1, U32_MAX),
  };
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

export interface RenameSettings {
  prefix: string;
  start_number: number;
  /** 1 ~ 10 */
  digit_count: number;
  shuffle: boolean;
  /** 只在 shuffle 时发送 */
  shuffle_seed?: number;
  /** 同步重命名同名的 .txt、.json、.caption */
  rename_tags: boolean;
}

export const RENAME_DEFAULTS: Readonly<Omit<RenameSettings, 'shuffle_seed'>> = {
  prefix: 'img_',
  start_number: 1,
  digit_count: 4,
  shuffle: false,
  rename_tags: true,
};

/** 重命名只处理输入目录的顶层，不递归 */
export function buildRenameOptions(io: Pick<InputIO, 'input_path'>, settings: Partial<RenameSettings> = {}): RenameOptions {
  const d = RENAME_DEFAULTS;
  const shuffle = bool(settings.shuffle, d.shuffle);
  const seed = finite(settings.shuffle_seed);
  return {
    input_path: io.input_path,
    prefix: text(settings.prefix, d.prefix),
    start_number: int(settings.start_number, d.start_number, 0, U32_MAX),
    digit_count: int(settings.digit_count, d.digit_count, 1, 10),
    shuffle,
    shuffle_seed: shuffle && seed !== undefined && seed >= 0 ? Math.floor(seed) : undefined,
    rename_tags: bool(settings.rename_tags, d.rename_tags),
  };
}

/** file_keeper.rs → keep_specified_files */
export interface FileKeeperOptions {
  folder_path: string;
  /** 不带点的扩展名，如 'png' */
  keep_extensions: string[];
}

/** resolution_analyze.rs → analyze_resolutions */
export interface ResolutionAnalyzeOptions {
  input_path: string;
  /** u32，默认 10：图片数不超过它的分辨率标为稀有并附文件列表 */
  rare_threshold?: number;
  recursive?: boolean;
}

export interface ResolutionGroup {
  width: number;
  height: number;
  count: number;
  /** 占总数的百分比 */
  percent: number;
  /** 常见宽高比标签，如 "16:9"；无法归类时为空 */
  aspect_label: string;
  is_rare: boolean;
  /** 只有稀有分辨率带文件路径 */
  files: string[];
}

/** analyze_resolutions 的返回值 */
export interface ResolutionAnalyzeResult {
  total_images: number;
  /** 读不出尺寸的文件数 */
  failed_count: number;
  failed_files: string[];
  distinct_count: number;
  groups: ResolutionGroup[];
  min_width: number;
  max_width: number;
  min_height: number;
  max_height: number;
}

/** 聚合导出计划的一个目标文件夹：命中任一分辨率的图片复制进 folder */
export interface AggregatePlanEntry {
  /** 通常为 "宽x高" */
  folder: string;
  /** [宽, 高] */
  resolutions: [number, number][];
}

/** resolution_analyze.rs → export_resolution_aggregation（返回完成消息） */
export interface ResolutionAggregateOptions {
  input_path: string;
  recursive: boolean;
  output_path: string;
  plan: AggregatePlanEntry[];
}

// ── SD 元数据 ──

/** scan_sd_metadata 的参数（独立参数，不包在 options 里） */
export interface ScanSdMetadataArgs {
  inputPath: string;
  /** 缺省按 false */
  recursive?: boolean | null;
}

export interface SdImageMeta {
  path: string;
  filename: string;
  positive: string;
  negative: string;
  params: string;
  /** a1111 / comfyui / novelai / unknown */
  source: string;
}

/** scan_sd_metadata 的返回值 */
export interface SdScanResult {
  items: SdImageMeta[];
  total_images: number;
  has_meta_count: number;
  no_meta_count: number;
  no_meta_files: string[];
  /** 来源 → 张数 */
  source_counts: Record<string, number>;
  scan_time_ms: number;
}

export interface ExportTagItem {
  source_path: string;
  positive: string;
}

/** sd_metadata.rs → export_sd_tags */
export interface ExportTagsOptions {
  /** same：标签写在图片旁；custom：写进 dest_folder */
  mode: 'same' | 'custom';
  /** mode 为 custom 时的目标文件夹 */
  dest_folder?: string | null;
  /** custom 时按它保留子目录结构；缺省不保留 */
  input_root?: string | null;
  items: ExportTagItem[];
}

/** export_sd_tags 的返回值 */
export interface ExportTagsResult {
  success_count: number;
  fail_count: number;
  skip_count: number;
  errors: string[];
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
  /** 辅助打标：本地标签写入专用中间文件 */
  hybrid_mode?: boolean;
  /** 默认 1 */
  batch_size?: number;
  recursive?: boolean;
}

export type TaggerSettings = Required<Omit<TaggerOptions, keyof InputIO>>;

/** 阈值随模型变化，页面以所选模型的推荐值为准 */
export const TAGGER_DEFAULTS: Readonly<TaggerSettings> = {
  model_id: 'wd-swinv2-tagger-v3',
  general_threshold: 0.35,
  character_threshold: 0.85,
  enabled_categories: ['general', 'character'],
  use_gpu: false,
  exclude_tags: '',
  append_tags: '',
  append_position: 'append',
  json_append_field: 'tags',
  replace_underscore: true,
  output_format: 'txt',
  json_simplified: false,
  escape_parentheses: false,
  sort_by: 'confidence',
  existing_tags_action: 'overwrite',
  hybrid_mode: false,
  batch_size: 1,
};

/** batch_size 只在 GPU 下生效，CPU 恒为 1；enabled_categories 去重并去掉不认识的类别 */
export function buildTaggerOptions(io: InputIO, settings: Partial<TaggerSettings> = {}): TaggerOptions {
  const d = TAGGER_DEFAULTS;
  const useGpu = bool(settings.use_gpu, d.use_gpu);
  const categories = Array.isArray(settings.enabled_categories)
    ? [...new Set(settings.enabled_categories.filter(c => isOneOf(TAGGER_CATEGORY_KEYS, c)))]
    : [...d.enabled_categories];
  return {
    input_path: io.input_path,
    recursive: io.recursive === true,
    model_id: text(settings.model_id, d.model_id) || d.model_id,
    general_threshold: num(settings.general_threshold, d.general_threshold, 0, 1),
    character_threshold: num(settings.character_threshold, d.character_threshold, 0, 1),
    enabled_categories: categories,
    use_gpu: useGpu,
    exclude_tags: text(settings.exclude_tags, d.exclude_tags),
    append_tags: text(settings.append_tags, d.append_tags),
    append_position: choice(APPEND_POSITIONS, settings.append_position, d.append_position),
    json_append_field: choice(JSON_APPEND_FIELD_KEYS, settings.json_append_field, d.json_append_field),
    replace_underscore: bool(settings.replace_underscore, d.replace_underscore),
    output_format: choice(TAG_FILE_FORMATS, settings.output_format, d.output_format),
    json_simplified: bool(settings.json_simplified, d.json_simplified),
    escape_parentheses: bool(settings.escape_parentheses, d.escape_parentheses),
    sort_by: choice(TAG_SORT_ORDERS, settings.sort_by, d.sort_by),
    existing_tags_action: choice(EXISTING_TAGS_ACTIONS, settings.existing_tags_action, d.existing_tags_action),
    hybrid_mode: bool(settings.hybrid_mode, d.hybrid_mode),
    batch_size: useGpu ? int(settings.batch_size, d.batch_size, 1, 64) : 1,
  };
}

/** tagger/mod.rs → prepare_hybrid_tags */
export interface PrepareHybridTagsOptions {
  input_path: string;
  /** 提供标签分类词表的模型 */
  model_id: string;
  file_format: TagFileFormat;
  json_simplified?: boolean;
  recursive?: boolean;
}

export const SHORT_REPLY_THRESHOLD = { min: 1, max: 500, default: 100 } as const;

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
  short_reply_threshold?: number;
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

export type LlmTaggerSettings = Required<Omit<LlmTaggerOptions, keyof InputIO | 'api_endpoint' | 'api_key'>>;

/** 提示词随输出格式变化，不在这里：缺省时取 getDefaultPrompts(output_format, json_simplified) */
export const LLM_TAGGER_DEFAULTS: Readonly<Omit<LlmTaggerSettings, 'system_prompt' | 'user_prompt'>> = {
  model_name: '',
  temperature: 0.2,
  max_tokens: -1,
  image_size: 1024,
  top_p: 0,
  short_reply_threshold: SHORT_REPLY_THRESHOLD.default,
  skip_existing: false,
  output_format: 'txt',
  json_simplified: false,
  request_interval_ms: -1,
  concurrency: 1,
  image_detail: '',
};

/**
 * request_interval_ms 是毫秒，输入框按秒时先用 utils/taggerOptions 的 toIntervalMs 换算；
 * system_prompt / user_prompt 不传时取输出格式的默认提示词（传空串则原样发送）。
 */
export function buildLlmTaggerOptions(
  io: InputIO,
  api: { api_endpoint: string; api_key: string },
  settings: Partial<LlmTaggerSettings> = {},
): LlmTaggerOptions {
  const d = LLM_TAGGER_DEFAULTS;
  const outputFormat = choice(TAG_FILE_FORMATS, settings.output_format, d.output_format);
  const simplified = bool(settings.json_simplified, d.json_simplified);
  const prompts = getDefaultPrompts(outputFormat, simplified);
  return {
    input_path: io.input_path,
    recursive: io.recursive === true,
    api_endpoint: api.api_endpoint,
    api_key: api.api_key,
    model_name: text(settings.model_name, d.model_name),
    system_prompt: text(settings.system_prompt, prompts.sys),
    user_prompt: text(settings.user_prompt, prompts.user),
    temperature: num(settings.temperature, d.temperature, 0, 2),
    max_tokens: int(settings.max_tokens, d.max_tokens, -1, I32_MAX),
    image_size: int(settings.image_size, d.image_size, 1, U32_MAX),
    top_p: num(settings.top_p, d.top_p, 0, 1),
    short_reply_threshold: int(settings.short_reply_threshold, d.short_reply_threshold, SHORT_REPLY_THRESHOLD.min, SHORT_REPLY_THRESHOLD.max),
    skip_existing: bool(settings.skip_existing, d.skip_existing),
    output_format: outputFormat,
    json_simplified: simplified,
    request_interval_ms: int(settings.request_interval_ms, d.request_interval_ms, -1, Number.MAX_SAFE_INTEGER),
    concurrency: int(settings.concurrency, d.concurrency, 1, U32_MAX),
    image_detail: choice(IMAGE_DETAILS, settings.image_detail, d.image_detail),
  };
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
  short_reply_threshold?: number;
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
  /** 仅 JSON：只补写 nl，未返回描述时报错并保留原文件 */
  nl_only?: boolean;
  hybrid_mode?: boolean;
  skip_existing_labels?: boolean;
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
  /** 缺省 256 */
  min_bucket_reso?: number | null;
  /** 缺省取 res_width、res_height 中较大的一个 */
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

export interface BucketSettings {
  res_width: number;
  res_height: number;
  steps: number;
  /** diffusion_pipe 模式恒为 true */
  no_upscale: boolean;
  /** 桶边长范围；no_upscale 或 diffusion_pipe 时不发送 */
  min_bucket_reso: number;
  max_bucket_reso: number;
  bucket_mode: BucketMode;
  /** 以下三项只在 diffusion_pipe 时发送 */
  dp_min_ar: number;
  dp_max_ar: number;
  dp_num_ar_buckets: number;
  batch_size: number;
  /** 只在 diffusion_pipe 时生效，其余模式恒为 false */
  drop_last: boolean;
}

/** 分桶预览页面的初始值 */
export const BUCKET_DEFAULTS: Readonly<BucketSettings> = {
  res_width: 1024,
  res_height: 1024,
  steps: 32,
  no_upscale: true,
  min_bucket_reso: 256,
  max_bucket_reso: 2048,
  bucket_mode: 'legacy',
  dp_min_ar: 0.5,
  dp_max_ar: 2,
  dp_num_ar_buckets: 7,
  batch_size: 1,
  drop_last: true,
};

export function buildBucketOptions(io: InputIO, settings: Partial<BucketSettings> = {}): BucketOptions {
  const d = BUCKET_DEFAULTS;
  const mode = choice(BUCKET_MODES, settings.bucket_mode, d.bucket_mode);
  const dp = mode === 'diffusion_pipe';
  const noUpscale = dp || bool(settings.no_upscale, d.no_upscale);
  return {
    input_path: io.input_path,
    recursive: io.recursive === true,
    res_width: int(settings.res_width, d.res_width, 1, U32_MAX),
    res_height: int(settings.res_height, d.res_height, 1, U32_MAX),
    steps: int(settings.steps, d.steps, 1, U32_MAX),
    no_upscale: noUpscale,
    min_bucket_reso: noUpscale ? null : int(settings.min_bucket_reso, d.min_bucket_reso, 1, U32_MAX),
    max_bucket_reso: noUpscale ? null : int(settings.max_bucket_reso, d.max_bucket_reso, 1, U32_MAX),
    bucket_mode: mode,
    dp_min_ar: dp ? num(settings.dp_min_ar, d.dp_min_ar, 0) : null,
    dp_max_ar: dp ? num(settings.dp_max_ar, d.dp_max_ar, 0) : null,
    dp_num_ar_buckets: dp ? int(settings.dp_num_ar_buckets, d.dp_num_ar_buckets, 1, U32_MAX) : null,
    batch_size: int(settings.batch_size, d.batch_size, 1, U32_MAX),
    drop_last: dp ? bool(settings.drop_last, d.drop_last) : false,
  };
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
  start_image_cluster: ClusterOptions;
  start_image_dedup: DedupOptions;
  scan_dedup_rename: DedupRenameOptions;
  filter_by_resolution: FilterOptions;
  preview_rename: RenameOptions;
  execute_rename: RenameOptions;
  keep_specified_files: FileKeeperOptions;
  analyze_resolutions: ResolutionAnalyzeOptions;
  export_resolution_aggregation: ResolutionAggregateOptions;
  export_sd_tags: ExportTagsOptions;
  start_tagging: TaggerOptions;
  prepare_hybrid_tags: PrepareHybridTagsOptions;
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

/** 标签管理批量保存失败的一项；path 为发送时条目的图片路径 */
export interface SaveFailure {
  path: string;
  error: string;
}

/** save_all_tag_files / save_all_caption_files / save_all_json_files 的返回（tag_manager.rs SaveAllResult） */
export interface SaveAllResult {
  saved: number;
  failed: SaveFailure[];
}
