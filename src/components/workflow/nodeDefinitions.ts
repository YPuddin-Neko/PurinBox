// ═══════════════ 节点定义 ═══════════════
import type { NodeTypeDef } from './workflowTypes';

// 分类颜色
export const CATEGORY_COLORS: Record<string, string> = {
  input: '#7c5cfc',
  process: '#00d4ff',
  ai: '#f59e0b',
  tag: '#4ade80',
  analysis: '#f472b6',
  file: '#a78bfa',
  condition: '#fb923c',
  output: '#7c5cfc',
};

export const NODE_DEFS: NodeTypeDef[] = [
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
    tauriCommand: 'scale_images',
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
    tauriCommand: 'crop_images',
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
    tauriCommand: 'flip_images',
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
    tauriCommand: 'convert_format',
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
    tauriCommand: 'convert_alpha',
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
    tauriCommand: 'blur_noise_images',
  },
  {
    type: 'perspective',
    nameKey: 'workflow.nodePerspective',
    category: 'process',
    icon: 'Move3D',
    color: CATEGORY_COLORS.process,
    params: [
      { key: 'intensity', labelKey: 'perspective.intensity', type: 'number', default: 10, min: 1, max: 50 },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    tauriCommand: 'perspective_transform',
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
    tauriCommand: 'start_upscale',
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
    tauriCommand: 'start_person_crop',
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
    tauriCommand: 'start_aesthetic_scoring',
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
        { value: 'json_simplified', labelKey: 'JSON (简化)' },
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
      { key: 'append_position', labelKey: 'aiTagger.appendTags', type: 'select', default: 'append', options: [
        { value: 'prepend', labelKey: 'aiTagger.prepend' },
        { value: 'append', labelKey: 'aiTagger.append' },
      ]},
      { key: 'batch_size', labelKey: 'aiTagger.batchSize', type: 'number', default: 1, min: 1, max: 32, step: 1 },
      { key: 'exclude_tags', labelKey: 'aiTagger.excludeTags', type: 'string', default: '' },
      { key: 'append_tags', labelKey: 'aiTagger.appendTags', type: 'string', default: '' },
      { key: 'cat_general', labelKey: 'aiTagger.catGeneral', type: 'boolean', default: true },
      { key: 'cat_character', labelKey: 'aiTagger.catCharacter', type: 'boolean', default: true },
      { key: 'cat_rating', labelKey: 'aiTagger.catRating', type: 'boolean', default: false },
      { key: 'cat_artist', labelKey: 'aiTagger.catArtist', type: 'boolean', default: false },
      { key: 'cat_copyright', labelKey: 'aiTagger.catCopyright', type: 'boolean', default: false },
      { key: 'cat_meta', labelKey: 'aiTagger.catMeta', type: 'boolean', default: false },
      { key: 'replace_underscore', labelKey: 'aiTagger.replaceUnderscore', type: 'boolean', default: true },
      { key: 'escape_parentheses', labelKey: 'aiTagger.escapeParentheses', type: 'boolean', default: false },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    tauriCommand: 'start_tagging',
  },
  {
    type: 'llm-tagger',
    nameKey: 'workflow.nodeLlmTagger',
    category: 'tag',
    icon: 'Tags',
    color: CATEGORY_COLORS.tag,
    params: [
      { key: 'api_endpoint', labelKey: 'llmTagger.apiEndpoint', type: 'string', default: '' },
      { key: 'api_key', labelKey: 'llmTagger.apiKey', type: 'string', default: '' },
      { key: 'model_name', labelKey: 'llmTagger.modelLabel', type: 'string', default: '' },
      { key: 'system_prompt', labelKey: 'llmTagger.systemPrompt', type: 'string', default: '' },
      { key: 'user_prompt', labelKey: 'llmTagger.userPrompt', type: 'string', default: '' },
      { key: 'temperature', labelKey: 'llmTagger.temperature', type: 'number', default: 0.7, min: 0, max: 2, step: 0.1 },
      { key: 'max_tokens', labelKey: 'llmTagger.maxTokens', type: 'number', default: -1, min: -1, max: 4096, step: 1 },
      { key: 'image_size', labelKey: 'llmTagger.imageSize', type: 'number', default: 1024, min: 256, max: 4096, step: 64 },
      { key: 'concurrency', labelKey: 'llmTagger.concurrency', type: 'number', default: 1, min: 1, max: 16, step: 1 },
      { key: 'output_format', labelKey: 'llmTagger.outputFormat', type: 'select', default: 'txt', options: [
        { value: 'txt', labelKey: 'TXT' },
        { value: 'json', labelKey: 'JSON' },
        { value: 'json_simplified', labelKey: 'JSON (简化)' },
      ]},
      { key: 'skip_existing', labelKey: 'llmTagger.skipExisting', type: 'boolean', default: false },
    ],
    hasInput: true,
    hasOutput: true,
    inputLabelKey: 'workflow.slotImage',
    outputLabelKey: 'workflow.slotImage',
    tauriCommand: 'start_llm_tagging',
  },

  // ── 分析/条件 ──
  {
    type: 'bucket-assign',
    nameKey: 'workflow.nodeBucketAssign',
    category: 'condition',
    icon: 'Grid3X3',
    color: CATEGORY_COLORS.condition,
    params: [
      { key: 'res_width', labelKey: 'bucketPreview.resolution', type: 'number', default: 1024, min: 64, step: 64 },
      { key: 'res_height', labelKey: 'bucketPreview.resolution', type: 'number', default: 1024, min: 64, step: 64 },
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
    tauriCommand: 'analyze_buckets',
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
    tauriCommand: 'filter_by_resolution',
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
    tauriCommand: 'execute_rename',
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
