/**
 * OpenAI Vision 的 image_url.detail 取值（DeepSeek 视觉模型等兼容端点同样支持）。
 * low 在推理前把图缩到 512×512，更快更省 token；high / original 保留原图；auto 由服务端决定。
 * 空字符串表示整个字段不发送——Gemini 兼容层等不认识该字段的端点会因未知参数报错。
 *
 * label 本地化、value 保持协议原值：显示怎么翻译都不影响发给后端的字段。
 */
export const IMAGE_DETAIL_OPTIONS = (t: (key: string) => string) => [
  { value: '', label: t('llmApi.imageDetailDefault') },
  { value: 'auto', label: `${t('llmApi.imageDetailAuto')} (auto)` },
  { value: 'low', label: `${t('llmApi.imageDetailLow')} (low)` },
  { value: 'high', label: `${t('llmApi.imageDetailHigh')} (high)` },
  { value: 'original', label: `${t('llmApi.imageDetailOriginal')} (original)` },
];
