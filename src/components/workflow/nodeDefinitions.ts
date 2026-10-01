// ═══════════════ 节点定义 ═══════════════
import type { NodeTypeDef } from './workflowTypes';
import type { OptionsCommandCall } from '../../api/commandOptions';
import { loadLlmApiConfig, resolveLlmApi } from '../../utils/llmPresets';
import { TAGGER_CATEGORIES, JSON_APPEND_FIELDS, categoriesFromFlags, splitOutputFormat, toIntervalMs, toThreads, toImageSize } from '../../utils/taggerOptions';
import { getDefaultPrompts } from '../../utils/llmPrompts';

// 分类颜色
export const CATEGORY_COLORS: Record<string, string> = {
  input: '#7c5cfc',
  process: '#00d4ff',
  ai: '#f59e0b',
  tag: '#4ade80',
  file: '#a78bfa',
  condition: '#fb923c',
  output: '#7c5cfc',
};

const NODE_DEFS: NodeTypeDef[] = [
  // ── 输入 ──
  {
    type: 'image-folder',
    nameKey: 'workflow.nodeImageFolder',
    category: 'input',
    icon: 'FolderOpen',
    color: CATEGORY_COLORS.input,
    params: [
      { key: 'path', labelKey: 'workflow.inputPath', type: 'path', default: '' },
      { key: 'recursive', labelKey: 'pages.recursiveScan', type: 'boolean', default: false },
    ],
    hasInput: false,
    hasOutput: true,
    outputLabelKey: 'workflow.slotImage',
  },

  // ── 图像处理 ──
  {
    type: 'scale',
    nameKey: 'workflow.nodeScale',
    category: 'process',
    icon: 'Scaling',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'width', labelKey: 'scale.width', type: 'number', default: 1024, min: 1 },
      { key: 'height', labelKey: 'scale.height', type: 'number', default: 1024, min: 1 },
      { key: 'mode', labelKey: 'scale.scaleOptions', type: 'select', default: 'upscale', options: [
        { value: 'upscale', labelKey: 'scale.upscale' },
        { value: 'downscale', labelKey: 'scale.downscale' },
        { value: 'both', labelKey: 'scale.startBoth' },
      ]},
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'scale_images', options: {
      ...io, mode: p.mode, target_width: p.width, target_height: p.height,
      down_target_width: 0, down_target_height: 0,
    }}),
    cancelCommand: 'cancel_scale',
    progressEvent: 'scale-progress',
  },
  {
    type: 'crop',
    nameKey: 'workflow.nodeCrop',
    category: 'process',
    icon: 'Crop',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'mode', labelKey: 'crop.cropMode', type: 'select', default: 'center', options: [
        { value: 'center', labelKey: 'crop.center' },
        { value: 'cover', labelKey: 'crop.cover' },
      ]},
      { key: 'width', labelKey: 'crop.targetWidth', type: 'number', default: 1024, min: 1 },
      { key: 'height', labelKey: 'crop.targetHeight', type: 'number', default: 1024, min: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'crop_images', options: {
      ...io, mode: p.mode, crop_anchor: 'center', target_width: p.width, target_height: p.height,
      aspect_ratio: 1, crop_top: 0, crop_bottom: 0, crop_left: 0, crop_right: 0,
    }}),
    cancelCommand: 'cancel_crop',
    progressEvent: 'crop-progress',
  },
  {
    type: 'flip',
    nameKey: 'workflow.nodeFlip',
    category: 'process',
    icon: 'FlipHorizontal2',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'direction', labelKey: 'flip.flipDirection', type: 'select', default: 'horizontal', options: [
        { value: 'horizontal', labelKey: 'flip.horizontal' },
        { value: 'vertical', labelKey: 'flip.vertical' },
      ]},
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'flip_images', options: { ...io, direction: p.direction }}),
    cancelCommand: 'cancel_flip',
    progressEvent: 'flip-progress',
  },
  {
    type: 'format-convert',
    nameKey: 'workflow.nodeFormatConvert',
    category: 'process',
    icon: 'FileType',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'target_format', labelKey: 'formatConvert.targetFormat', type: 'select', default: 'png', options: [
        { value: 'png', labelKey: 'PNG' },
        { value: 'jpg', labelKey: 'JPG' },
        { value: 'webp', labelKey: 'WebP' },
        { value: 'bmp', labelKey: 'BMP' },
      ]},
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'convert_format', options: { ...io, target_format: p.target_format }}),
    cancelCommand: 'cancel_convert',
    progressEvent: 'convert-progress',
  },
  {
    type: 'alpha-convert',
    nameKey: 'workflow.nodeAlphaConvert',
    category: 'process',
    icon: 'Layers',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'background', labelKey: 'alphaConvert.fillArea', type: 'select', default: 'white', options: [
        { value: 'white', labelKey: 'alphaConvert.bgWhite' },
        { value: 'black', labelKey: 'alphaConvert.bgBlack' },
      ]},
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'convert_alpha', options: { ...io, background: p.background }}),
    cancelCommand: 'cancel_alpha',
    progressEvent: 'alpha-progress',
  },
  {
    type: 'blur-noise',
    nameKey: 'workflow.nodeBlurNoise',
    category: 'process',
    icon: 'Sparkles',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'blur_radius', labelKey: 'blurNoise.blurRadius', type: 'number', default: 0, min: 0, max: 50, step: 0.1 },
      { key: 'noise_strength', labelKey: 'blurNoise.noiseStrength', type: 'number', default: 0, min: 0, max: 100, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'blur_noise_images', options: {
      ...io, blur_radius: p.blur_radius, noise_strength: p.noise_strength,
    }}),
    cancelCommand: 'cancel_blur_noise',
    progressEvent: 'blur-noise-progress',
  },
  {
    type: 'perspective',
    nameKey: 'workflow.nodePerspective',
    category: 'process',
    icon: 'Move3D',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'intensity', labelKey: 'perspective.intensity', type: 'number', default: 0.1, min: 0.02, max: 0.3, step: 0.01 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'perspective_transform', options: { ...io, intensity: p.intensity }}),
    cancelCommand: 'cancel_perspective',
    progressEvent: 'perspective-progress',
  },

  // ── AI 处理 ──
  {
    type: 'upscale',
    nameKey: 'workflow.nodeUpscale',
    category: 'ai',
    icon: 'ZoomIn',
    color: CATEGORY_COLORS.ai,
    params: [
      { key: 'engine_id', labelKey: 'upscale.engine', type: 'select', default: 'realcugan', options: [
        { value: 'realcugan', labelKey: 'RealCUGAN' },
        { value: 'realesrgan', labelKey: 'RealESRGAN' },
        { value: 'waifu2x', labelKey: 'Waifu2x' },
      ]},
      { key: 'scale', labelKey: 'upscale.scaleRatio', type: 'select', default: '2', options: [
        { value: '2', labelKey: '2x' },
        { value: '4', labelKey: '4x' },
      ]},
      { key: 'denoise_level', labelKey: 'upscale.denoiseLevel', type: 'number', default: -1, min: -1, max: 3, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_upscale', options: {
      ...io, engine_id: p.engine_id,
      model_id: p.engine_id === 'realesrgan' ? 'realesrgan-x4plus' : p.engine_id === 'waifu2x' ? 'models-cunet' : 'models-se',
      scale: Number(p.scale), denoise_level: p.denoise_level, tta: false, gpu_id: 0, tile_size: 0,
    }}),
    cancelCommand: 'force_cancel_upscale',
    progressEvent: 'upscale-progress',
  },
  {
    type: 'person-crop',
    nameKey: 'workflow.nodePersonCrop',
    category: 'ai',
    icon: 'ScanFace',
    color: CATEGORY_COLORS.ai,
    params: [
      { key: 'person_enabled', labelKey: 'personCrop.fullBody', type: 'boolean', default: true },
      { key: 'upper_enabled', labelKey: 'personCrop.halfBody', type: 'boolean', default: false },
      { key: 'head_enabled', labelKey: 'personCrop.headDet', type: 'boolean', default: false },
      { key: 'person_conf', labelKey: 'personCrop.confThreshold', type: 'number', default: 0.3, min: 0, max: 1, step: 0.05 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_person_crop', options: {
      ...io, use_gpu: true, person_enabled: p.person_enabled, person_conf: p.person_conf,
      upper_enabled: p.upper_enabled, upper_conf: 0.3, upper_tag: '',
      head_enabled: p.head_enabled, head_conf: 0.3, head_tag: '', head_scale: 1.5,
      eyes_enabled: false, eyes_conf: 0.3, eyes_tag: '', eyes_scale: 2, keep_original_tags: true,
    }}),
    cancelCommand: 'force_cancel_person_crop',
    progressEvent: 'person-crop-progress',
  },
  {
    type: 'aesthetic',
    nameKey: 'workflow.nodeAesthetic',
    category: 'ai',
    icon: 'Star',
    color: CATEGORY_COLORS.ai,
    params: [
      { key: 'batch_size', labelKey: 'aesthetic.batchSize', type: 'number', default: 1, min: 1, max: 32, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    nestedOutput: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_aesthetic_scoring', options: {
      // Temporary outputs must never become the only copy of the source images.
      ...io, use_gpu: true, copy_files: true, batch_size: p.batch_size,
    }}),
    cancelCommand: 'force_cancel_aesthetic_scoring',
    progressEvent: 'aesthetic-progress',
  },

  // ── 标签 ──
  {
    type: 'tagger',
    nameKey: 'workflow.nodeTagger',
    category: 'tag',
    icon: 'Tags',
    color: CATEGORY_COLORS.tag,
    params: [
      { key: 'model_id', labelKey: 'aiTagger.taggerModel', type: 'dynamic-select', default: 'wd-swinv2-tagger-v3',
        tauriListCommand: 'get_tagger_models', optionValueKey: 'id', optionLabelKey: 'name',
        optionFilter: { key: 'is_downloaded', value: true } },
      { key: 'general_threshold', labelKey: 'aiTagger.generalTh', type: 'number', default: 0.35, min: 0, max: 1, step: 0.01 },
      { key: 'character_threshold', labelKey: 'aiTagger.charTh', type: 'number', default: 0.85, min: 0, max: 1, step: 0.01 },
      { key: 'output_format', labelKey: 'aiTagger.outputFormat', type: 'select', default: 'txt', options: [
        { value: 'txt', labelKey: 'TXT' },
        { value: 'json', labelKey: 'JSON' },
        { value: 'json_simplified', labelKey: 'workflow.jsonSimplified' },
      ]},
      { key: 'existing_tags_action', labelKey: 'aiTagger.existingTagsAction', type: 'select', default: 'overwrite', options: [
        { value: 'overwrite', labelKey: 'aiTagger.existingAction_overwrite' },
        { value: 'skip', labelKey: 'aiTagger.existingAction_skip' },
        { value: 'prepend', labelKey: 'aiTagger.existingAction_prepend' },
        { value: 'append', labelKey: 'aiTagger.existingAction_append' },
      ]},
      { key: 'sort_by', labelKey: 'aiTagger.sortBy', type: 'select', default: 'confidence', options: [
        { value: 'confidence', labelKey: 'aiTagger.sortBy_confidence' },
        { value: 'frequency', labelKey: 'aiTagger.sortBy_frequency' },
      ]},
      { key: 'append_position', labelKey: 'workflow.appendPosition', type: 'select', default: 'append', options: [
        { value: 'prepend', labelKey: 'aiTagger.prepend' },
        { value: 'append', labelKey: 'aiTagger.append' },
      ]},
      { key: 'json_append_field', labelKey: 'aiTagger.appendField', type: 'select', default: 'tags', options: JSON_APPEND_FIELDS },
      { key: 'batch_size', labelKey: 'aiTagger.batchSize', type: 'number', default: 1, min: 1, max: 32, step: 1 },
      { key: 'exclude_tags', labelKey: 'aiTagger.excludeTags', type: 'string', default: '' },
      { key: 'append_tags', labelKey: 'aiTagger.appendTags', type: 'string', default: '' },
      ...TAGGER_CATEGORIES.map(c => ({ key: `cat_${c.key}`, labelKey: c.labelKey, type: 'boolean' as const, default: c.defaultOn })),
      { key: 'replace_underscore', labelKey: 'aiTagger.replaceUnderscore', type: 'boolean', default: true },
      { key: 'escape_parentheses', labelKey: 'aiTagger.escapeParentheses', type: 'boolean', default: false },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_tagging', options: {
      input_path: io.input_path, recursive: io.recursive, model_id: p.model_id,
      general_threshold: p.general_threshold, character_threshold: p.character_threshold,
      enabled_categories: categoriesFromFlags(p), use_gpu: true,
      exclude_tags: p.exclude_tags, append_tags: p.append_tags, append_position: p.append_position,
      json_append_field: p.json_append_field, replace_underscore: p.replace_underscore,
      ...splitOutputFormat(p.output_format), escape_parentheses: p.escape_parentheses,
      sort_by: p.sort_by, existing_tags_action: p.existing_tags_action, batch_size: p.batch_size,
    }}),
    cancelCommand: 'force_cancel_tagging',
    progressEvent: 'tagger-progress',
  },
  {
    type: 'llm-tagger',
    nameKey: 'workflow.nodeLlmTagger',
    category: 'tag',
    icon: 'Tags',
    color: CATEGORY_COLORS.tag,
    params: [
      { key: 'model_name', labelKey: 'llmApi.modelLabel', type: 'string', default: '' },
      { key: 'system_prompt', labelKey: 'llmTagger.systemPrompt', type: 'string', default: getDefaultPrompts('txt', false).sys },
      { key: 'user_prompt', labelKey: 'llmTagger.userPrompt', type: 'string', default: getDefaultPrompts('txt', false).user },
      { key: 'temperature', labelKey: 'llmTagger.temperature', type: 'number', default: 0.7, min: 0, max: 2, step: 0.1 },
      { key: 'max_tokens', labelKey: 'llmTagger.maxTokens', type: 'number', default: -1, min: -1, max: 4096, step: 1 },
      { key: 'image_size', labelKey: 'llmTagger.imageSize', type: 'number', default: 1024, min: 256, max: 4096, step: 64 },
      { key: 'image_detail', labelKey: 'tagRefine.imageDetail', type: 'select', default: '', options: [
        { value: '', labelKey: 'tagRefine.imageDetailDefault' },
        { value: 'auto', labelKey: 'tagRefine.imageDetailAuto' },
        { value: 'low', labelKey: 'tagRefine.imageDetailLow' },
        { value: 'high', labelKey: 'tagRefine.imageDetailHigh' },
        { value: 'original', labelKey: 'tagRefine.imageDetailOriginal' },
      ]},
      { key: 'request_interval', labelKey: 'workflow.requestInterval', type: 'number', default: -1, min: -1, step: 0.1 },
      { key: 'concurrency', labelKey: 'llmTagger.concurrency', type: 'number', default: 1, min: 1, max: 16, step: 1 },
      { key: 'output_format', labelKey: 'llmTagger.outputFormat', type: 'select', default: 'txt', options: [
        { value: 'txt', labelKey: 'TXT' },
        { value: 'json', labelKey: 'JSON' },
        { value: 'json_simplified', labelKey: 'workflow.jsonSimplified' },
      ]},
      { key: 'skip_existing', labelKey: 'llmTagger.skipExisting', type: 'boolean', default: false },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: true,
    buildOptions: async (p, io): Promise<OptionsCommandCall> => {
      const { endpoint, apiKey } = resolveLlmApi(await loadLlmApiConfig());
      return { command: 'start_llm_tagging', options: {
        input_path: io.input_path, recursive: io.recursive, api_endpoint: endpoint, api_key: apiKey,
        model_name: p.model_name, system_prompt: p.system_prompt, user_prompt: p.user_prompt,
        temperature: p.temperature, max_tokens: p.max_tokens, image_size: toImageSize(p.image_size),
        image_detail: p.image_detail, skip_existing: p.skip_existing, ...splitOutputFormat(p.output_format),
        request_interval_ms: toIntervalMs(p.request_interval), concurrency: toThreads(p.concurrency, 16),
      }};
    },
    cancelCommand: 'cancel_llm_tagging',
    progressEvent: 'llm-tagger-progress',
  },

  // ── 条件 ──
  {
    type: 'bucket-assign',
    nameKey: 'workflow.nodeBucketAssign',
    category: 'condition',
    icon: 'Grid3X3',
    color: CATEGORY_COLORS.condition,
    params: [
      { key: 'res_width', labelKey: 'workflow.bucketWidth', type: 'number', default: 1024, min: 64, step: 64 },
      { key: 'res_height', labelKey: 'workflow.bucketHeight', type: 'number', default: 1024, min: 64, step: 64 },
      { key: 'steps', labelKey: 'bucketPreview.stepsLabel', type: 'select', default: '64', options: [
        { value: '32', labelKey: '32' },
        { value: '64', labelKey: '64' },
        { value: '128', labelKey: '128' },
      ]},
      { key: 'no_upscale', labelKey: 'bucketPreview.noUpscale', type: 'boolean', default: false },
      { key: 'uniform_threshold', labelKey: 'workflow.bucketUniformThreshold', type: 'number', default: 70, min: 10, max: 100, step: 5 },
      { key: 'max_outlier_buckets', labelKey: 'workflow.bucketMaxOutliers', type: 'number', default: 2, min: 0, max: 20, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    hasOutputB: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.branchUniform',
    outputBLabelKey: 'workflow.branchScattered',
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'analyze_buckets', options: {
      input_path: io.input_path, recursive: io.recursive, res_width: p.res_width,
      res_height: p.res_height, steps: Number(p.steps), no_upscale: p.no_upscale,
    }}),
    cancelCommand: 'cancel_bucket_analysis',
    progressEvent: 'bucket-progress',
  },

  // ── 文件操作 ──
  {
    type: 'filter',
    nameKey: 'workflow.nodeFilter',
    category: 'file',
    icon: 'ScanSearch',
    color: CATEGORY_COLORS.file,
    params: [
      { key: 'action', labelKey: 'filter.actionMode', type: 'select', default: 'copy', options: [
        { value: 'copy', labelKey: 'filter.actionCopy' },
        { value: 'delete', labelKey: 'filter.actionDelete' },
      ]},
      { key: 'condition', labelKey: 'filter.filterCondition', type: 'select', default: 'below_resolution', options: [
        { value: 'min_width', labelKey: 'filter.condMinWidth' },
        { value: 'min_height', labelKey: 'filter.condMinHeight' },
        { value: 'below_resolution', labelKey: 'filter.condBelowRes' },
        { value: 'above_resolution', labelKey: 'filter.condAboveRes' },
      ]},
      { key: 'width', labelKey: 'filter.widthPx', type: 'number', default: 512, min: 1 },
      { key: 'height', labelKey: 'filter.heightPx', type: 'number', default: 512, min: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: p => p.action === 'delete',
    carrySidecars: true,
    allowEmptyResult: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'filter_by_resolution', options: {
      ...io, action: p.action, condition: p.condition, width: p.width, height: p.height,
    }}),
    cancelCommand: 'cancel_filter',
    progressEvent: 'filter-progress',
  },
  {
    type: 'rename',
    nameKey: 'workflow.nodeRename',
    category: 'file',
    icon: 'TextCursorInput',
    color: CATEGORY_COLORS.file,
    params: [
      { key: 'prefix', labelKey: 'batchRename.prefix', type: 'string', default: 'img_' },
      { key: 'start_number', labelKey: 'batchRename.startNum', type: 'number', default: 1, min: 0, step: 1 },
      { key: 'digit_count', labelKey: 'batchRename.digitCount', type: 'number', default: 4, min: 1, max: 8, step: 1 },
      { key: 'shuffle', labelKey: 'batchRename.shuffle', type: 'boolean', default: false },
      { key: 'rename_tags', labelKey: 'batchRename.renameTags', type: 'boolean', default: true },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: true,
    flatInputOnly: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'execute_rename', options: {
      input_path: io.input_path, prefix: p.prefix, start_number: p.start_number,
      digit_count: p.digit_count, shuffle: p.shuffle, rename_tags: p.rename_tags,
    }}),
    progressEvent: 'rename-progress',
  },

  // ── 输出 ──
  {
    type: 'output-folder',
    nameKey: 'workflow.nodeOutput',
    category: 'output',
    icon: 'FolderOutput',
    color: CATEGORY_COLORS.output,
    params: [
      { key: 'path', labelKey: 'workflow.outputPath', type: 'path', default: '' },
    ],
    hasInput: true,
    hasOutput: false,
    inputLabelKey: 'workflow.slotImage',
  },
];

/** 按分类分组的节点定义 */
export function getNodeDefsByCategory(): Record<string, NodeTypeDef[]> {
  const groups: Record<string, NodeTypeDef[]> = {};
  for (const def of NODE_DEFS) {
    if (!groups[def.category]) groups[def.category] = [];
    groups[def.category].push(def);
  }
  return groups;
}

/** 根据 type 获取节点定义 */
export function getNodeDef(type: string): NodeTypeDef | undefined {
  return NODE_DEFS.find(d => d.type === type);
}

/** Normalize old files and keep credentials out of node state and serialization. */
export function withDefaults(def: NodeTypeDef | undefined, params: Record<string, any> = {}): Record<string, any> {
  const values = Object.fromEntries(Object.entries(params).filter(([key, value]) => key !== 'api_key' && value != null));
  const result = { ...Object.fromEntries((def?.params ?? []).map(p => [p.key, p.default])), ...values };
  for (const param of def?.params ?? []) {
    const value = result[param.key];
    if (param.type === 'number') {
      let number = Number(value);
      if (!Number.isFinite(number) || value === '') number = Number(param.default);
      if (!param.step || Number.isInteger(param.step)) number = Math.round(number);
      result[param.key] = Math.min(param.max ?? Infinity, Math.max(param.min ?? -Infinity, number));
    } else if (param.type === 'select') {
      result[param.key] = param.options?.some(option => option.value === String(value)) ? String(value) : param.default;
    } else if (param.type === 'boolean') {
      result[param.key] = typeof value === 'boolean' ? value : param.default;
    }
  }
  if (def?.type === 'llm-tagger') {
    delete result.api_endpoint;
    const defaults = getDefaultPrompts(result.output_format === 'txt' ? 'txt' : 'json', result.output_format === 'json_simplified');
    if (!values.system_prompt) result.system_prompt = defaults.sys;
    if (!values.user_prompt) result.user_prompt = defaults.user;
  }
  return result;
}
