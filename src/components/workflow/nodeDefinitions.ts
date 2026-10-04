// ═══════════════ 节点定义 ═══════════════
import type { NodeTypeDef } from './workflowTypes';
import {
  AESTHETIC_DEFAULTS, ALPHA_CONVERT_DEFAULTS, BLUR_NOISE_DEFAULTS, BUCKET_DEFAULTS, CROP_DEFAULTS,
  FILTER_DEFAULTS, FLIP_DEFAULTS, FORMAT_CONVERT_DEFAULTS, LLM_TAGGER_DEFAULTS, PERSON_CROP_DEFAULTS,
  PERSPECTIVE_DEFAULTS, RENAME_DEFAULTS, SCALE_DEFAULTS, TAGGER_DEFAULTS, UPSCALE_DEFAULTS,
  buildAestheticOptions, buildAlphaConvertOptions, buildBlurNoiseOptions, buildBucketOptions, buildCropOptions,
  buildFilterOptions, buildFlipOptions, buildFormatConvertOptions, buildLlmTaggerOptions, buildPersonCropOptions,
  buildPerspectiveOptions, buildRenameOptions, buildScaleOptions, buildTaggerOptions, buildUpscaleOptions,
  type OptionsCommandCall,
} from '../../api/commandOptions';
import { loadLlmApiConfig, resolveLlmApi } from '../../utils/llmPresets';
import { TAGGER_CATEGORIES, JSON_APPEND_FIELDS, categoriesFromFlags, splitOutputFormat, toIntervalMs } from '../../utils/taggerOptions';
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
      { key: 'width', labelKey: 'scale.width', type: 'number', default: SCALE_DEFAULTS.upscale_width, min: 1 },
      { key: 'height', labelKey: 'scale.height', type: 'number', default: SCALE_DEFAULTS.upscale_height, min: 1 },
      { key: 'mode', labelKey: 'scale.scaleOptions', type: 'select', default: SCALE_DEFAULTS.mode, options: [
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
    // 节点只有一组宽高，上采样与下采样目标都用它
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'scale_images', options: buildScaleOptions(io, {
      mode: p.mode, upscale_width: p.width, upscale_height: p.height, downscale_width: p.width, downscale_height: p.height,
    })}),
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
      { key: 'mode', labelKey: 'crop.cropMode', type: 'select', default: CROP_DEFAULTS.mode, options: [
        { value: 'center', labelKey: 'crop.center' },
        { value: 'cover', labelKey: 'crop.cover' },
      ]},
      { key: 'width', labelKey: 'crop.targetWidth', type: 'number', default: CROP_DEFAULTS.target_width, min: 1 },
      { key: 'height', labelKey: 'crop.targetHeight', type: 'number', default: CROP_DEFAULTS.target_height, min: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'crop_images', options: buildCropOptions(io, {
      mode: p.mode, target_width: p.width, target_height: p.height,
    })}),
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
      { key: 'direction', labelKey: 'flip.flipDirection', type: 'select', default: FLIP_DEFAULTS.direction, options: [
        { value: 'horizontal', labelKey: 'flip.horizontal' },
        { value: 'vertical', labelKey: 'flip.vertical' },
      ]},
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'flip_images', options: buildFlipOptions(io, { direction: p.direction }) }),
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
      { key: 'target_format', labelKey: 'formatConvert.targetFormat', type: 'select', default: FORMAT_CONVERT_DEFAULTS.target_format, options: [
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
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'convert_format', options: buildFormatConvertOptions(io, { target_format: p.target_format }) }),
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
      { key: 'background', labelKey: 'alphaConvert.fillArea', type: 'select', default: ALPHA_CONVERT_DEFAULTS.background, options: [
        { value: 'white', labelKey: 'alphaConvert.bgWhite' },
        { value: 'black', labelKey: 'alphaConvert.bgBlack' },
      ]},
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'convert_alpha', options: buildAlphaConvertOptions(io, { background: p.background }) }),
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
      { key: 'blur_radius', labelKey: 'blurNoise.blurRadius', type: 'number', default: BLUR_NOISE_DEFAULTS.blur_radius, min: 0, max: 50, step: 0.1 },
      { key: 'noise_strength', labelKey: 'blurNoise.noiseStrength', type: 'number', default: BLUR_NOISE_DEFAULTS.noise_strength, min: 0, max: 100, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'blur_noise_images', options: buildBlurNoiseOptions(io, {
      blur_radius: p.blur_radius, noise_strength: p.noise_strength,
    })}),
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
      { key: 'intensity', labelKey: 'perspective.intensity', type: 'number', default: PERSPECTIVE_DEFAULTS.intensity, min: 0.02, max: 0.3, step: 0.01 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'perspective_transform', options: buildPerspectiveOptions(io, { intensity: p.intensity }) }),
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
      { key: 'engine_id', labelKey: 'upscale.engine', type: 'select', default: UPSCALE_DEFAULTS.engine_id, options: [
        { value: 'realcugan', labelKey: 'RealCUGAN' },
        { value: 'realesrgan', labelKey: 'RealESRGAN' },
        { value: 'waifu2x', labelKey: 'Waifu2x' },
      ]},
      { key: 'scale', labelKey: 'upscale.scaleRatio', type: 'select', default: String(UPSCALE_DEFAULTS.scale), options: [
        { value: '2', labelKey: '2x' },
        { value: '4', labelKey: '4x' },
      ]},
      { key: 'denoise_level', labelKey: 'upscale.denoiseLevel', type: 'number', default: UPSCALE_DEFAULTS.denoise_level, min: -1, max: 3, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    carrySidecars: true,
    // 节点不选模型，用引擎的默认模型
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_upscale', options: buildUpscaleOptions(io, {
      engine_id: p.engine_id, scale: p.scale, denoise_level: p.denoise_level, use_gpu: true,
    })}),
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
      { key: 'person_enabled', labelKey: 'personCrop.fullBody', type: 'boolean', default: PERSON_CROP_DEFAULTS.person_enabled },
      { key: 'upper_enabled', labelKey: 'personCrop.halfBody', type: 'boolean', default: false },
      { key: 'head_enabled', labelKey: 'personCrop.headDet', type: 'boolean', default: false },
      { key: 'person_conf', labelKey: 'personCrop.confThreshold', type: 'number', default: PERSON_CROP_DEFAULTS.person_conf, min: 0, max: 1, step: 0.05 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    // 节点不裁眼部、不写裁切标签，只带原标签；没暴露的阈值和放大倍数取页面默认值
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_person_crop', options: buildPersonCropOptions(io, {
      use_gpu: true, person_enabled: p.person_enabled, person_conf: p.person_conf,
      upper_enabled: p.upper_enabled, upper_tag: '', head_enabled: p.head_enabled, head_tag: '',
      eyes_enabled: false, eyes_tag: '', keep_original_tags: true,
    })}),
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
      { key: 'batch_size', labelKey: 'aesthetic.batchSize', type: 'number', default: AESTHETIC_DEFAULTS.batch_size, min: 1, max: 32, step: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    nestedOutput: true,
    // 输出多半在 .workflow_temp 里：必须复制，移动会让原图的唯一副本随临时目录一起被清理
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_aesthetic_scoring', options: buildAestheticOptions(io, {
      use_gpu: true, copy_files: true, batch_size: p.batch_size,
    })}),
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
      { key: 'model_id', labelKey: 'aiTagger.taggerModel', type: 'dynamic-select', default: TAGGER_DEFAULTS.model_id,
        tauriListCommand: 'get_tagger_models', optionValueKey: 'id', optionLabelKey: 'name',
        optionFilter: { key: 'is_downloaded', value: true } },
      { key: 'general_threshold', labelKey: 'aiTagger.generalTh', type: 'number', default: TAGGER_DEFAULTS.general_threshold, min: 0, max: 1, step: 0.01 },
      { key: 'character_threshold', labelKey: 'aiTagger.charTh', type: 'number', default: TAGGER_DEFAULTS.character_threshold, min: 0, max: 1, step: 0.01 },
      { key: 'output_format', labelKey: 'aiTagger.outputFormat', type: 'select', default: 'txt', options: [
        { value: 'txt', labelKey: 'TXT' },
        { value: 'json', labelKey: 'JSON' },
        { value: 'json_simplified', labelKey: 'workflow.jsonSimplified' },
      ]},
      { key: 'existing_tags_action', labelKey: 'aiTagger.existingTagsAction', type: 'select', default: TAGGER_DEFAULTS.existing_tags_action, options: [
        { value: 'overwrite', labelKey: 'aiTagger.existingAction_overwrite' },
        { value: 'skip', labelKey: 'aiTagger.existingAction_skip' },
        { value: 'prepend', labelKey: 'aiTagger.existingAction_prepend' },
        { value: 'append', labelKey: 'aiTagger.existingAction_append' },
      ]},
      { key: 'sort_by', labelKey: 'aiTagger.sortBy', type: 'select', default: TAGGER_DEFAULTS.sort_by, options: [
        { value: 'confidence', labelKey: 'aiTagger.sortBy_confidence' },
        { value: 'frequency', labelKey: 'aiTagger.sortBy_frequency' },
      ]},
      { key: 'append_position', labelKey: 'workflow.appendPosition', type: 'select', default: TAGGER_DEFAULTS.append_position, options: [
        { value: 'prepend', labelKey: 'aiTagger.prepend' },
        { value: 'append', labelKey: 'aiTagger.append' },
      ]},
      { key: 'json_append_field', labelKey: 'aiTagger.appendField', type: 'select', default: TAGGER_DEFAULTS.json_append_field, options: JSON_APPEND_FIELDS },
      { key: 'batch_size', labelKey: 'aiTagger.batchSize', type: 'number', default: TAGGER_DEFAULTS.batch_size, min: 1, max: 32, step: 1 },
      { key: 'exclude_tags', labelKey: 'aiTagger.excludeTags', type: 'string', default: TAGGER_DEFAULTS.exclude_tags },
      { key: 'append_tags', labelKey: 'aiTagger.appendTags', type: 'string', default: TAGGER_DEFAULTS.append_tags },
      ...TAGGER_CATEGORIES.map(c => ({ key: `cat_${c.key}`, labelKey: c.labelKey, type: 'boolean' as const, default: c.defaultOn })),
      { key: 'replace_underscore', labelKey: 'aiTagger.replaceUnderscore', type: 'boolean', default: TAGGER_DEFAULTS.replace_underscore },
      { key: 'escape_parentheses', labelKey: 'aiTagger.escapeParentheses', type: 'boolean', default: TAGGER_DEFAULTS.escape_parentheses },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'start_tagging', options: buildTaggerOptions(io, {
      model_id: p.model_id, general_threshold: p.general_threshold, character_threshold: p.character_threshold,
      enabled_categories: categoriesFromFlags(p), use_gpu: true,
      exclude_tags: p.exclude_tags, append_tags: p.append_tags, append_position: p.append_position,
      json_append_field: p.json_append_field, replace_underscore: p.replace_underscore,
      ...splitOutputFormat(p.output_format), escape_parentheses: p.escape_parentheses,
      sort_by: p.sort_by, existing_tags_action: p.existing_tags_action, batch_size: p.batch_size,
    })}),
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
      { key: 'model_name', labelKey: 'llmApi.modelLabel', type: 'string', default: LLM_TAGGER_DEFAULTS.model_name },
      { key: 'system_prompt', labelKey: 'llmTagger.systemPrompt', type: 'string', multiline: true, default: getDefaultPrompts('txt', false).sys },
      { key: 'user_prompt', labelKey: 'llmTagger.userPrompt', type: 'string', multiline: true, default: getDefaultPrompts('txt', false).user },
      { key: 'temperature', labelKey: 'llmApi.temperature', type: 'number', default: LLM_TAGGER_DEFAULTS.temperature, min: 0, max: 2, step: 0.1 },
      { key: 'max_tokens', labelKey: 'llmTagger.maxTokens', type: 'number', default: LLM_TAGGER_DEFAULTS.max_tokens, min: -1, max: 4096, step: 1 },
      { key: 'image_size', labelKey: 'llmApi.imageSize', type: 'number', default: LLM_TAGGER_DEFAULTS.image_size, min: 256, max: 4096, step: 64 },
      { key: 'image_detail', labelKey: 'llmApi.imageDetail', type: 'select', default: LLM_TAGGER_DEFAULTS.image_detail, options: [
        { value: '', labelKey: 'llmApi.imageDetailDefault' },
        { value: 'auto', labelKey: 'llmApi.imageDetailAuto' },
        { value: 'low', labelKey: 'llmApi.imageDetailLow' },
        { value: 'high', labelKey: 'llmApi.imageDetailHigh' },
        { value: 'original', labelKey: 'llmApi.imageDetailOriginal' },
      ]},
      // 单位是秒，构造参数时换算成毫秒；-1 表示不间隔
      { key: 'request_interval', labelKey: 'workflow.requestInterval', type: 'number', default: -1, min: -1, step: 0.1 },
      { key: 'concurrency', labelKey: 'llmApi.concurrency', type: 'number', default: LLM_TAGGER_DEFAULTS.concurrency, min: 1, max: 16, step: 1 },
      { key: 'output_format', labelKey: 'llmTagger.outputFormat', type: 'select', default: 'txt', options: [
        { value: 'txt', labelKey: 'TXT' },
        { value: 'json', labelKey: 'JSON' },
        { value: 'json_simplified', labelKey: 'workflow.jsonSimplified' },
      ]},
      { key: 'skip_existing', labelKey: 'llmTagger.skipExisting', type: 'boolean', default: LLM_TAGGER_DEFAULTS.skip_existing },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: true,
    // 端点和 Key 取 VLM 打标页保存的全局配置，不随工作流保存
    buildOptions: async (p, io): Promise<OptionsCommandCall> => {
      const { endpoint, apiKey } = resolveLlmApi(await loadLlmApiConfig());
      const prompts = fillBlankPrompts('llm-tagger', p);
      return { command: 'start_llm_tagging', options: buildLlmTaggerOptions(io, { api_endpoint: endpoint, api_key: apiKey }, {
        model_name: p.model_name, system_prompt: prompts.system_prompt, user_prompt: prompts.user_prompt,
        temperature: p.temperature, max_tokens: p.max_tokens, image_size: p.image_size, image_detail: p.image_detail,
        skip_existing: p.skip_existing, ...splitOutputFormat(p.output_format),
        request_interval_ms: toIntervalMs(p.request_interval), concurrency: p.concurrency,
      })};
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
      { key: 'res_width', labelKey: 'workflow.bucketWidth', type: 'number', default: BUCKET_DEFAULTS.res_width, min: 64, step: 64 },
      { key: 'res_height', labelKey: 'workflow.bucketHeight', type: 'number', default: BUCKET_DEFAULTS.res_height, min: 64, step: 64 },
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
    // 桶边长上限沿用后端的缺省值（宽高中较大的一个），页面默认的 2048 不适用于节点
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'analyze_buckets', options: buildBucketOptions(io, {
      res_width: p.res_width, res_height: p.res_height, steps: p.steps, no_upscale: p.no_upscale,
      max_bucket_reso: Math.max(p.res_width, p.res_height),
    })}),
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
      { key: 'action', labelKey: 'filter.actionMode', type: 'select', default: FILTER_DEFAULTS.action, options: [
        { value: 'copy', labelKey: 'filter.actionCopy' },
        { value: 'delete', labelKey: 'filter.actionDelete' },
      ]},
      { key: 'condition', labelKey: 'filter.filterCondition', type: 'select', default: FILTER_DEFAULTS.condition, options: [
        { value: 'min_width', labelKey: 'filter.condMinWidth' },
        { value: 'min_height', labelKey: 'filter.condMinHeight' },
        { value: 'below_resolution', labelKey: 'filter.condBelowRes' },
        { value: 'above_resolution', labelKey: 'filter.condAboveRes' },
      ]},
      { key: 'width', labelKey: 'filter.widthPx', type: 'number', default: FILTER_DEFAULTS.width, min: 1 },
      { key: 'height', labelKey: 'filter.heightPx', type: 'number', default: FILTER_DEFAULTS.height, min: 1 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: p => p.action === 'delete',
    carrySidecars: true,
    allowEmptyResult: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'filter_by_resolution', options: buildFilterOptions(io, {
      action: p.action, condition: p.condition, width: p.width, height: p.height,
    })}),
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
      { key: 'prefix', labelKey: 'batchRename.prefix', type: 'string', default: RENAME_DEFAULTS.prefix },
      { key: 'start_number', labelKey: 'batchRename.startNum', type: 'number', default: RENAME_DEFAULTS.start_number, min: 0, step: 1 },
      { key: 'digit_count', labelKey: 'batchRename.digitCount', type: 'number', default: RENAME_DEFAULTS.digit_count, min: 1, max: 8, step: 1 },
      { key: 'shuffle', labelKey: 'batchRename.shuffle', type: 'boolean', default: RENAME_DEFAULTS.shuffle },
      { key: 'rename_tags', labelKey: 'batchRename.renameTags', type: 'boolean', default: RENAME_DEFAULTS.rename_tags },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    inPlace: true,
    flatInputOnly: true,
    buildOptions: (p, io): OptionsCommandCall => ({ command: 'execute_rename', options: buildRenameOptions(io, {
      prefix: p.prefix, start_number: p.start_number, digit_count: p.digit_count, shuffle: p.shuffle, rename_tags: p.rename_tags,
    })}),
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

/** 就地修改输入目录、不产出新目录的节点：它的输出就是输入 */
export function isInPlaceNode(def: NodeTypeDef | undefined, params: Record<string, any>): boolean {
  return typeof def?.inPlace === 'function' ? def.inPlace(params) : !!def?.inPlace;
}

const vlmDefaultPrompts = (outputFormat: unknown) =>
  getDefaultPrompts(outputFormat === 'txt' ? 'txt' : 'json', outputFormat === 'json_simplified');

const isBlank = (value: unknown) => typeof value !== 'string' || value.trim() === '';

/**
 * 补齐缺少的参数并规范类型：保存过的值照常生效，旧文件缺的项取默认值。
 * API Key 不进节点状态，也不写进工作流文件。
 */
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
    } else if (typeof value !== 'string') {
      result[param.key] = String(param.default);
    }
  }
  if (def?.type === 'llm-tagger') {
    delete result.api_endpoint;
    // 缺提示词时取当前输出格式的默认提示词；空串是编辑时清空的，留着让文本框保持为空
    const prompts = vlmDefaultPrompts(result.output_format);
    if (typeof values.system_prompt !== 'string') result.system_prompt = prompts.sys;
    if (typeof values.user_prompt !== 'string') result.user_prompt = prompts.user;
  }
  return result;
}

/**
 * 工作流里 VLM 节点的提示词留空 = 用当前输出格式的默认提示词。
 * 读取工作流文件和运行节点时经这里换成默认值；编辑时不经过这里，文本框可以清空。其他节点原样返回。
 */
export function fillBlankPrompts(type: string, params: Record<string, any>): Record<string, any> {
  if (type !== 'llm-tagger' || (!isBlank(params.system_prompt) && !isBlank(params.user_prompt))) return params;
  const prompts = vlmDefaultPrompts(params.output_format);
  return {
    ...params,
    system_prompt: isBlank(params.system_prompt) ? prompts.sys : params.system_prompt,
    user_prompt: isBlank(params.user_prompt) ? prompts.user : params.user_prompt,
  };
}
