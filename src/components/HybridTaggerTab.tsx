import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import {
  Key, Bot, RefreshCw, Loader2, Eye, EyeOff, Save, Thermometer, Image as ImageIcon,
  Check, Cpu, Gpu, Trash2, Focus, Hash,
} from 'lucide-react';
import { Modal } from './Modal';
import ProgressLog, { getTimeStr, useLogState } from './ProgressLog';
import ProcessButton from './ProcessButton';
import InputPathPickerButton from './InputPathPickerButton';
import CustomSelect from './CustomSelect';
import Checkbox from './Checkbox';
import { useTaskQueue } from './TaskContext';
import { useUnifiedTaskLogs } from '../hooks/useUnifiedTaskLogs';
import { useTranslation } from 'react-i18next';
import { IMAGE_DETAIL_OPTIONS } from '../utils/imageDetail';

interface ModelInfo { id: string; name: string; description: string; input_size: number; is_builtin: boolean; is_downloaded: boolean; repo_id: string; input_format: string; supported_categories: string[]; }
interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; i18n_key?: string; i18n_params?: Record<string, string>; }

type Phase = '' | 'converting' | 'tagging' | 'refining';

/** 默认调优提示词（TXT 模式）：补充缺失 / 删除错误 / 修复不准确（{tags} 由后端替换为该图现有标签） */
const defaultPromptTxt = `You are an expert anime image tagger. You will receive an image and its existing tags produced by a local tagger model.

Your task:
1. Compare the image content with the existing tags
2. Fix incorrect tags (e.g. wrong hair color, wrong clothing, wrong subject count)
3. Add important missing tags that are clearly visible in the image
4. Remove tags that do not match the image at all
5. Keep the tag format consistent (lowercase danbooru-style tags)

Rules:
- Only make changes you are confident about
- Preserve tags that are correct
- Return ONLY the refined tags, comma-separated
- Do NOT add explanations

Existing tags: {tags}

Refined tags:`;

/** 默认调优提示词（JSON 模式）：在 TXT 模式基础上，要求 LLM 把标签分配到
 *  count/appearance/tags/environment 四个字段，并补写 nl 自然语言描述。
 *  本地打标器这四类是靠关键词表猜的（"simple background" 之类常落错格），
 *  由 LLM 按画面重新归类；后端解析 COUNT:/APPEARANCE:/TAGS:/ENVIRONMENT:/NL: 标记段。
 *  四类语义按 AnimaLoraStudio 打标文档：表情/姿势/构图属 tags 而非 appearance。
 *  quality/series/artist/character 来自 tagger 的模型分类，不在重排范围内。 */
const defaultPromptJson = `You are an expert anime image tagger. You will receive an image and its existing tags produced by a local tagger model.

Your task:
1. Compare the image content with the existing tags
2. Fix incorrect tags (e.g. wrong hair color, wrong clothing, wrong subject count)
3. Add important missing tags that are clearly visible in the image
4. Remove tags that do not match the image at all
5. Keep the tag format consistent (lowercase danbooru-style tags)
6. Sort every remaining tag into the correct category (the local tagger often puts tags in the wrong one)
7. Write a natural language description (1-2 sentences) of the image, consistent with the image content and the final tags

Categories:
- COUNT: character count only, e.g. 1girl, 2boys, 1girl 1boy, no humans
- APPEARANCE: the character's visual features - hair color, hairstyle, eye color, clothing, accessories
- TAGS: actions, expressions, poses, composition, objects held or used, e.g. smile, standing, looking at viewer, upper body
- ENVIRONMENT: background, location, lighting, atmosphere, e.g. simple background, white background, outdoors, classroom, night, sunlight

Rules:
- Only make changes you are confident about
- Preserve tags that are correct
- Every tag must appear in exactly one category
- Leave a category line empty if it has no tags
- Character names, series names, artist names and quality tags are handled separately - do not list them
- Do NOT add explanations

Existing tags: {tags}

Reply in exactly this format (five lines, nothing else):
COUNT: <comma-separated>
APPEARANCE: <comma-separated>
TAGS: <comma-separated>
ENVIRONMENT: <comma-separated>
NL: <natural language description>`;

/** 仅补 nl 描述：不动任何标签，只根据画面和现有标签写自然语言描述。
 *  本地打标器不产生 nl 字段，这个预设专门补它；后端见到只有 NL: 一段的回复会保留原标签。 */
const promptNlOnly = `You are an expert anime image tagger. You will receive an image and its existing tags produced by a local tagger model.

Your task:
Write a natural language description (1-2 sentences) of the image, consistent with the image content and the existing tags.

Rules:
- Do NOT modify, add or remove any tags
- Describe only what is clearly visible
- Do NOT add explanations

Existing tags: {tags}

Reply in exactly this format (one line, nothing else):
NL: <natural language description>`;

/** 归类字段 + 补描述（JSON 专用）：给已经打好标的数据集用——
 *  LLM 只决定每个标签属于哪个字段并补 nl，标签集合一个不增不减。
 *  后端走 preserve_tags 保证集合恒定，不指望模型遵守"不要增删"的嘱咐。 */
const promptSortOnly = `You are an expert anime image tagger. You will receive an image and its existing tags, already produced by a local tagger.

Your task is ONLY to put each given tag into the correct category, and to write a natural language description. You must NOT change the tag set itself.

Rules:
- Use every tag you are given, exactly as written — do not reword, merge or split them
- Do NOT add any tag that is not in the list, even if you can see something untagged in the image
- Do NOT remove any tag, even if you believe it does not match the image
- Every tag must appear in exactly one category
- Leave a category line empty if nothing belongs to it
- Character names, series names, artist names and quality tags are handled separately, they are not in your list

Categories:
- COUNT: character count only, e.g. 1girl, 2boys, 1girl 1boy, no humans
- APPEARANCE: the character's visual features - hair color, hairstyle, eye color, clothing, accessories
- TAGS: actions, expressions, poses, composition, objects held or used, e.g. smile, standing, looking at viewer, upper body
- ENVIRONMENT: background, location, lighting, atmosphere, e.g. simple background, white background, outdoors, classroom, night, sunlight

Then write a natural language description (1-2 sentences) of the image, consistent with the image content and the tags.

Existing tags: {tags}

Reply in exactly this format (five lines, nothing else):
COUNT: <comma-separated>
APPEARANCE: <comma-separated>
TAGS: <comma-separated>
ENVIRONMENT: <comma-separated>
NL: <natural language description>`;

/** 详细自然语言打标（txt 专用）：LLM 结合本地标签与画面写一整段 500-600 词 caption，
 *  整段就是 .txt 的全部内容。本地标签只作为校准参考（视觉模型容易认错发色/人数/服装
 *  这类离散属性），不写回文件。后端走 caption_mode 独立通路，不解析标签、与 nl 无关。 */
const promptDetailedCaption = `You are a professional image captioning assistant producing detailed anime-style captions for AI art training. You will receive an image and the existing tags produced by a local anime tagger model. Describe ONLY what is actually visible. Never speculate, never critique the artwork, never add unrelated remarks.

HOW TO USE THE TAGS (important):
The tags come from a specialized anime tagger trained on the danbooru vocabulary. For anything that vocabulary covers, the tags are more reliable than your own visual guess — trust the tag over your impression when they conflict. That includes character count, hair color and style, eye color, garment types, accessories and named objects, and equally the pose and exposure vocabulary: standing, sitting, lying, on back, on stomach, all fours, kneeling, squatting, spread legs, legs up, arched back, bent over, top-down bottom-up, looking back, hand on hip, as well as nude, topless, bottomless, clothes lift, skirt lift, clothes pull, open clothes, see-through, breasts out, nipples, pussy, anus, censored, uncensored.

The tags state WHAT is present. Your own observation must supply WHAT IT LOOKS LIKE, WHERE, and TO WHAT DEGREE:
- camera and framing: shot size, camera height, angle, focal length, perspective, crop range, the subject's scale and position within the frame
- spatial relations: what occludes what, foreground versus background layering, where each element sits relative to the others
- degree and orientation, which no tag can express: a tag says "spread legs" but not how widely or whether they face the camera, "arched back" but not how deeply, "breasts" but not their size or shape, "clothes lift" but not how far or what exactly it exposes
- light and color: light direction and quality, glow, rim light, shadows, contrast, reflections, bokeh, depth of field, overall atmosphere

So treat every tag as an established fact, then quantify and situate it with what you see. Never just restate the tag list as sentences.

MULTIPLE FIGURES (read this before writing):
When more than one figure is present, describe the picture as ONE interaction, never one figure after another. Do not finish everything about the first figure and only then start on the second — that produces two separate portraits instead of a scene. Anchor the description to what the figures are doing to each other and where their bodies meet, and weave each figure's own hair, face, clothing and exposure into that shared action at the moment it becomes relevant. Every figure present must get real detail; none may be reduced to a passing mention at the end.

OUTPUT FORMAT (strict):
- English only, segments separated by English commas
- Length strictly 500-600 English words — this is critical, anything beyond gets truncated during training
- Must begin with: {trigger},
- Natural language description, NOT a tag list: every comma-separated segment must be a phrase or clause carrying a verb, a spatial relation or a state — never a bare noun sitting on its own
  Wrong: "long sleeves, white gloves, a frilled headdress, a red ribbon, a bow, a collar"
  Right: "long sleeves reach past her wrists into white gloves, a frilled headdress sits over her bangs, a red ribbon is knotted at her throat above a narrow collar"
- Useful training keywords are welcome but must be embedded inside natural language
- Spaces between words, never underscores: write "long hair", not "long_hair"
- No Chinese punctuation, no periods joining content, everything joined by commas, with exactly ONE period at the very end
- Output the caption text only: no title, no explanation, no line breaks

DESCRIBE IN THIS ORDER:
1. Camera and framing: shot size (close-up, medium shot, full body, wide shot), camera height and angle (eye level, high angle, low angle, bird's eye, worm's eye, over the shoulder, POV), focal length and perspective (wide angle, telephoto, fisheye, background compression, perspective distortion), crop range (bust crop, above the knees, above the waist), and the subject's scale and position offset within the frame — all worded as natural language, never as bare tags
2. Subject: number of figures, gender presentation, placement, body orientation, head orientation, gaze direction, standing / sitting / lying / floating pose, tilt angle, hand gestures, leg posture, facial expression, mouth shape, blush, eye color, quality of the gaze, hair color, hairstyle, bangs, twintails, length, curls, the direction the strands flow, hair ornaments
3. Visible anatomy: any clearly exposed body parts (chest and its size, nipples, belly, inner thighs, genitals, labia, clitoris, buttocks, anus). If a part is fully covered by clothing, say nothing about it; if partially or fully exposed, describe its form, color, wetness and degree of openness exactly as visible
4. Pose relative to the viewer: whether the buttocks face the camera, whether the genitals face the viewer, how widely the legs are spread, whether the hips are raised, whether the pose is kneeling prone or presented from behind, whether the waist dips or arches — describe sexually suggestive posture in explicit detail when present
5. Clothing: skirt color, garment construction, frills, lace, bows, ribbons, sheer tulle, layered hems, sleeves, gloves, stockings, thighhighs, pantyhose, shoes, hats, headdresses, collars, jewelry, straps, belts, crosses, small bells, floral accents, and explicitly which garments are lifted, rolled up, pulled aside, torn or missing, and what skin or anatomy that exposes
6. Objects and companions: cats, rabbits, small animals, plushies, bouquets, teacups, books, cakes, candy, microphones, umbrellas, weapons, crystals, bubbles, butterflies, petals, ribbons, stars, moons, musical notes, vines, branches, glass shards, light streaks, glowing particles, every visible ornament
7. Foreground occluders and environment: gardens, forests, night sky, balconies, castles, interior rooms, windows, curtains, mirrors, candelabra, ornate chairs, tea tables, birdcages, water surfaces, snow, fountains, architecture, furniture, distant scenery, blurred background, spatial layering
8. Light and color: light direction, soft light, glow, rim light, blown highlights, pink light, blue light, purple shadows, warm-cool contrast, transparent reflections, glassy quality, crystal refraction, bokeh, foreground blur, depth of field, dreamlike atmosphere, pictorial depth

FORBIDDEN:
- Tag lists or stacked isolated keywords
- Image quality defects, unless they are a deliberate style feature
- Copyright, ethics, authorship, dataset, watermark or any remark unrelated to the picture
- First person or conversational tone
- Generic filler unrelated to the image
- Titles, bullets, explanations, summaries or extra notes
- Anything not actually visible: never speculate or invent

Existing tags: {tags}

Output the caption text now, beginning with "{trigger}," and ending with a single period. Nothing else.`;

const defaultPromptFor = (fmt: 'txt' | 'json' | 'json_simplified') =>
  fmt === 'txt' ? defaultPromptTxt : defaultPromptJson;

/** captionMode: 该预设产出的是整段自然语言描述（直接落盘为 txt 内容），不是标签 */
interface PromptPreset {
  id: string;
  name: string;
  prompt: string;
  /** 产出整段自然语言描述（直接落盘为 txt 内容），不是标签 */
  captionMode?: boolean;
  /** 只归类不增删：标签集合由后端保证恒定，LLM 的回复只当归属映射 */
  preserveTags?: boolean;
}

const TRIGGER_WORD_KEY = 'hybrid_trigger_word';

/** 把提示词里的 {trigger} 换成实际触发词。
 *  触发词为空时整行删掉——否则会给模型留下 `beginning with ","` 这种残句。 */
const applyTriggerWord = (prompt: string, trigger: string): string => {
  const t = trigger.trim();
  if (t) return prompt.split('{trigger}').join(t);
  return prompt.split('\n').filter(l => !l.includes('{trigger}')).join('\n');
};
const CUSTOM_PRESETS_KEY = 'hybrid_prompt_presets';

const loadCustomPresets = (): PromptPreset[] => {
  try {
    const raw = localStorage.getItem(CUSTOM_PRESETS_KEY);
    const list = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter((p: PromptPreset) => p?.id && p?.name) : [];
  } catch { return []; }
};

export default function HybridTaggerTab() {
  const { t } = useTranslation();
  const cats = [
    { key: 'general', label: t('aiTagger.catGeneral'), default: true },
    { key: 'character', label: t('aiTagger.catCharacter'), default: true },
    { key: 'rating', label: t('aiTagger.catRating'), default: false },
    { key: 'artist', label: t('aiTagger.catArtist'), default: false },
    { key: 'copyright', label: t('aiTagger.catCopyright'), default: false },
    { key: 'meta', label: t('aiTagger.catMeta'), default: false },
    { key: 'quality', label: t('aiTagger.catQuality'), default: false },
    { key: 'model', label: t('aiTagger.catModel'), default: false },
  ];

  // ── 路径 ──
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);

  // ── 本地打标 ──
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('');
  const [genTh, setGenTh] = useState(0.35);
  const [charTh, setCharTh] = useState(0.85);
  const [useGpu, setUseGpu] = useState(true);
  const [replaceUnderscore, setReplaceUnderscore] = useState(true);
  const [escapeParentheses, setEscapeParentheses] = useState(false);
  /** 图片已有同格式标签文件时跳过本地打标（保留现成标签，直接进入 LLM 调优） */
  const [preferExisting, setPreferExisting] = useState(true);
  const [enabledCats, setEnabledCats] = useState<Set<string>>(new Set(cats.filter(c => c.default).map(c => c.key)));

  // ── LLM 调优 ──
  const [preset, setPreset] = useState('openai');
  const [customEndpoint, setCustomEndpoint] = useState('');
  // 每个预设各存一份 key，切换预设时输入框跟着换，不能只存单个值
  const [apiKeys, setApiKeys] = useState<Record<string, string>>({});
  const [showKey, setShowKey] = useState(false);
  const [modelName, setModelName] = useState('');
  const [modelList, setModelList] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchMsg, setFetchMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [saveMsg, setSaveMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [prompt, setPrompt] = useState(defaultPromptTxt);
  const [presetId, setPresetId] = useState('builtin_full');
  const [customPresets, setCustomPresets] = useState<PromptPreset[]>(loadCustomPresets);
  const [showSaveModal, setShowSaveModal] = useState(false);
  const [newPresetName, setNewPresetName] = useState('');
  // 触发词按项目变，不写死在预设里；提示词用 {trigger} 占位
  const [triggerWord, setTriggerWord] = useState(() => {
    try { return localStorage.getItem(TRIGGER_WORD_KEY) || ''; } catch { return ''; }
  });
  const [temperature, setTemperature] = useState('0.3');
  const [topP, setTopP] = useState('0');
  const [imageSize, setImageSize] = useState('1024');
  const [imageDetail, setImageDetail] = useState('');
  const [concurrency, setConcurrency] = useState('1');
  const [intervalSec, setIntervalSec] = useState('-1');

  // ── 输出 ──
  const [outputFormat, setOutputFormat] = useState<'txt' | 'json' | 'json_simplified'>('txt');

  // ── 执行状态 ──
  const [processing, setProcessing] = useState(false);
  const [phase, setPhase] = useState<Phase>('');
  const phaseRef = useRef<Phase>('');
  // 用户是否点过取消：start_tagging 被取消后仍返回 Ok，靠它拦住阶段二
  const cancelRequestedRef = useRef(false);
  phaseRef.current = phase;
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useLogState();
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const taskLogs = useUnifiedTaskLogs(setLogs);
  const { addTask, updateTask } = useTaskQueue();

  const PRESETS: Record<string, { label: string; url: string }> = {
    openai: { label: 'OpenAI', url: 'https://api.openai.com/v1/' },
    gemini: { label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai/' },
    deepseek: { label: 'DeepSeek', url: 'https://api.deepseek.com/v1/' },
    custom: { label: t('llmTagger.customLabel'), url: '' },
  };
  const endpoint = preset === 'custom' ? customEndpoint : (PRESETS[preset]?.url || '');
  const apiKey = apiKeys[preset] || '';
  const setApiKey = (v: string) => setApiKeys(prev => ({ ...prev, [preset]: v }));

  // 模型列表 + 已保存的 API 配置
  useEffect(() => {
    invoke<ModelInfo[]>('get_tagger_models').then(l => {
      setModels(l);
      const firstDownloaded = l.find(m => m.is_downloaded);
      if (!selectedModel) setSelectedModel((firstDownloaded || l[0])?.id || '');
    }).catch(() => {});
    invoke<{ preset: string; custom_endpoint: string; api_keys: Record<string, string> }>('load_api_config').then((cfg) => {
      const known = ['openai', 'gemini', 'deepseek', 'custom'];
      if (cfg.preset) setPreset(known.includes(cfg.preset) ? cfg.preset : 'custom');
      if (cfg.custom_endpoint) setCustomEndpoint(cfg.custom_endpoint);
      if (cfg.api_keys) setApiKeys(cfg.api_keys);
    }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 模型切换时移除其不支持的类别选择（WD 系列只支持部分类别）
  useEffect(() => {
    const curModel = models.find(m => m.id === selectedModel);
    if (!curModel) return;
    const supported = new Set(curModel.supported_categories);
    setEnabledCats(prev => {
      const next = new Set([...prev].filter(k => supported.has(k)));
      if (next.size === 0) cats.filter(c => c.default && supported.has(c.key)).forEach(c => next.add(c.key));
      if (next.size === 0 && curModel.supported_categories.length > 0) next.add(curModel.supported_categories[0]);
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedModel, models]);

  // 本地打标/转换走 tagger-progress，LLM 调优走 tag-refine-progress，统一进日志与进度条
  useEffect(() => {
    let cancelled = false;
    const handler = (expectPhases: Phase[]) => (e: { payload: ProgressPayload }) => {
      if (cancelled || !expectPhases.includes(phaseRef.current)) return;
      const p = e.payload;
      if (p.total > 0) {
        setPCur(p.current); setPTot(p.total);
        setProgress((p.current / p.total) * 100);
      }
      if (p.status === 'error') setHasErr(true);
      if (p.status !== 'processing') taskLogs.appendProgressLog(p);
    };
    const l1 = listen<ProgressPayload>('tagger-progress', handler(['tagging']));
    const l2 = listen<ProgressPayload>('tag-refine-progress', handler(['refining']));
    return () => { cancelled = true; l1.then(fn => fn()); l2.then(fn => fn()); };
  }, [taskLogs]);

  const handleFetchModels = async () => {
    if (!endpoint) return;
    setFetchingModels(true);
    try {
      const list = await invoke<string[]>('fetch_llm_models', { apiEndpoint: endpoint, apiKey });
      setModelList(list);
      if (list.length > 0 && !list.includes(modelName)) setModelName(list[0]);
      setFetchMsg({ text: t('llmTagger.fetchOk', { n: list.length }), ok: true });
    } catch (e: any) {
      setFetchMsg({ text: `${t('llmTagger.fetchFail')}: ${String(e)}`, ok: false });
    } finally {
      setFetchingModels(false);
      setTimeout(() => setFetchMsg(null), 3000);
    }
  };

  const handleSaveConfig = async () => {
    try {
      await invoke('save_api_config', { preset, customEndpoint, apiKeys });
      setSaveMsg({ text: t('llmTagger.configSaved'), ok: true });
    } catch (e: any) {
      setSaveMsg({ text: `${t('llmTagger.saveFailed')}: ${String(e)}`, ok: false });
    }
    setTimeout(() => setSaveMsg(null), 2000);
  };

  const toggleCat = (key: string) => {
    setEnabledCats(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };

  const isJson = outputFormat !== 'txt';
  const canStart = !!inputPath && !!selectedModel && !!endpoint && !!modelName && enabledCats.size > 0;

  // 内置预设 + 用户预设。「仅补 NL」只在 JSON 模式下有意义（txt 没有 nl 字段）
  const builtinPresets: PromptPreset[] = [
    // txt 只做标签调优（没有 nl 字段可写），JSON 才是"调标签 + 补自然语言描述"
    {
      id: 'builtin_full',
      name: isJson ? t('hybridTagger.presetFull') : t('hybridTagger.presetTagsOnly'),
      prompt: defaultPromptFor(outputFormat),
    },
    // 「归类字段 + 补描述」「仅补自然语言描述」都依赖 JSON 的字段结构与 nl
    ...(isJson ? [
      { id: 'builtin_sort', name: t('hybridTagger.presetSortOnly'), prompt: promptSortOnly, preserveTags: true },
      { id: 'builtin_nl', name: t('hybridTagger.presetNlOnly'), prompt: promptNlOnly },
    ] : []),
    // 「详细自然语言打标」整段 caption 就是 txt 的全部内容，标签只作为 LLM 的校准参考
    ...(!isJson ? [{ id: 'builtin_caption', name: t('hybridTagger.presetDetailedCaption'), prompt: promptDetailedCaption, captionMode: true }] : []),
  ];
  const allPresets = [...builtinPresets, ...customPresets];
  const isCustomPreset = customPresets.some(p => p.id === presetId);
  const activePreset = allPresets.find(p => p.id === presetId);
  const captionMode = !!activePreset?.captionMode;
  const preserveTags = !!activePreset?.preserveTags;

  const applyPreset = (id: string) => {
    setPresetId(id);
    const p = allPresets.find(x => x.id === id);
    if (p) setPrompt(p.prompt);
  };

  const persistPresets = (list: PromptPreset[]) => {
    setCustomPresets(list);
    try { localStorage.setItem(CUSTOM_PRESETS_KEY, JSON.stringify(list)); } catch { /* 配额满等，忽略 */ }
  };

  const handleSavePreset = () => {
    const name = newPresetName.trim();
    if (!name) return;
    const existing = customPresets.find(p => p.name === name);
    const id = existing ? existing.id : `u_${Date.now()}`;
    // captionMode 随当前预设继承：基于「详细自然语言打标」改的提示词，产出的仍是整段描述
    persistPresets(existing
      ? customPresets.map(p => (p.id === existing.id ? { ...p, prompt, captionMode, preserveTags } : p))
      : [...customPresets, { id, name, prompt, captionMode, preserveTags }]);
    setPresetId(id);
    setShowSaveModal(false);
    setNewPresetName('');
  };

  const handleDeletePreset = () => {
    persistPresets(customPresets.filter(p => p.id !== presetId));
    setPresetId('builtin_full');
    // 编辑框内容保留不动：删错了立刻再存一次就回来了，省一个确认弹窗
  };

  // 切换输出格式时联动默认提示词（JSON 模式要求标记格式以补写 nl 字段）；
  // 用户改过提示词或选了自定义预设则不动
  const handleFormatChange = (v: string) => {
    const next = v as typeof outputFormat;
    setOutputFormat(next);
    // 两个内置预设各自只适用一种格式，切到另一种就退回完整调优
    if (((presetId === 'builtin_nl' || presetId === 'builtin_sort') && next === 'txt')
      || (presetId === 'builtin_caption' && next !== 'txt')) {
      setPresetId('builtin_full');
      setPrompt(defaultPromptFor(next));
      return;
    }
    if (presetId === 'builtin_full') {
      setPrompt(prev =>
        prev === defaultPromptTxt || prev === defaultPromptJson ? defaultPromptFor(next) : prev
      );
    }
  };

  const handleStart = async () => {
    if (!canStart) return;
    cancelRequestedRef.current = false;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    addTask('tagger', t('hybridTagger.taskName'));
    const sec = parseFloat(intervalSec);
    const intervalMs = sec < 0 ? -1 : Math.round(sec * 1000);
    const threads = Math.max(1, parseInt(concurrency) || 1);
    taskLogs.setInitialLog(t('hybridTagger.phaseTagging'));

    try {
      // JSON 输出 + 优先使用已有标签：先把只有 .txt 的图按模型词表转成 JSON，
      // 这样下一步"跳过已有标签"的判定才能命中，不会丢掉手里现成的 txt 标签去重跑模型。
      // 已经有 .json 的图会被跳过（不拿扁平 txt 盖掉带 nl 的成果）
      if (isJson && preferExisting) {
        setPhase('converting');
        taskLogs.appendLog(t('hybridTagger.phaseConverting'), 'info');
        updateTask('tagger', { status: 'running', message: t('hybridTagger.phaseConverting') });
        await invoke<ProcessResult>('convert_tags_to_json', {
          options: {
            input_path: inputPath,
            model_id: selectedModel,
            json_simplified: outputFormat === 'json_simplified',
            remove_txt: false,
            recursive,
            overwrite_existing: false,
          },
        });
        if (cancelRequestedRef.current) throw '已取消';
      }

      // txt 输出 + 优先使用已有标签：只有 .json 的图先摊平成 txt，
      // 打标阶段的 skip 才能命中——不然手里现成的 JSON 标签会被无视、重跑模型
      if (!isJson && preferExisting) {
        setPhase('converting');
        taskLogs.appendLog(t('hybridTagger.phaseConvertingToTxt'), 'info');
        updateTask('tagger', { status: 'running', message: t('hybridTagger.phaseConvertingToTxt') });
        await invoke<ProcessResult>('convert_json_to_txt', {
          options: { input_path: inputPath, recursive, overwrite_existing: false },
        });
        if (cancelRequestedRef.current) throw '已取消';
      }

      // 本地打标（直接按所选格式输出）
      setPhase('tagging');
      await invoke<ProcessResult>('start_tagging', {
        options: {
          input_path: inputPath,
          model_id: selectedModel,
          general_threshold: genTh,
          character_threshold: charTh,
          enabled_categories: [...enabledCats],
          use_gpu: useGpu,
          exclude_tags: '',
          append_tags: '',
          append_position: 'append',
          replace_underscore: replaceUnderscore,
          output_format: isJson ? 'json' : 'txt',
          json_simplified: outputFormat === 'json_simplified',
          escape_parentheses: escapeParentheses,
          sort_by: 'confidence',
          existing_tags_action: preferExisting ? 'skip' : 'overwrite',
          batch_size: 1,
          recursive,
        },
      });

      // 阶段一期间点了取消：start_tagging 被取消后仍返回 Ok，
      // 绝不能继续进入 LLM 精修——那会照样烧 API 并就地改写数据集标签
      if (cancelRequestedRef.current) throw '已取消';

      // LLM 二次确认与调优（就地更新标签文件）
      setPhase('refining');
      setProgress(0); setPCur(0); setPTot(0);
      updateTask('tagger', { status: 'running', message: t('hybridTagger.phaseRefining') });
      taskLogs.appendLog(t('hybridTagger.phaseRefining'), 'info');
      // 二次确认：上面那次检查到这里之间点的取消仍会指向已结束的阶段一，尽量收窄窗口
      if (cancelRequestedRef.current) throw '已取消';
      await invoke<ProcessResult>('start_tag_refining', {
        options: {
          input_path: inputPath,
          output_path: inputPath,
          api_endpoint: endpoint,
          api_key: apiKey,
          model_name: modelName,
          prompt: applyTriggerWord(prompt, triggerWord),
          temperature: Number.isFinite(parseFloat(temperature)) ? parseFloat(temperature) : 0.3,
          max_tokens: -1,
          image_size: parseInt(imageSize) || 1024,
          image_detail: imageDetail,
          top_p: parseFloat(topP) || 0,
          request_interval_ms: intervalMs,
          concurrency: threads,
          recursive,
          // JSON 模式差量写回：保留本地打标的字段归属，仅应用 LLM 的增删
          file_format: isJson ? 'json' : 'txt',
          // 自然语言打标：LLM 回复整段写入 txt，不做标签解析（仅 txt 有意义）
          caption_mode: !isJson && captionMode,
          // 触发词：txt 强制置于开头（标签/自然语言都是），JSON 追加进 artist 字段
          trigger_word: triggerWord,
          // 只归类不增删：标签集合由后端保证恒定（仅 JSON 有字段结构）
          preserve_tags: isJson && preserveTags,
        },
      });

      setIsDone(true);
      taskLogs.appendLog(t('hybridTagger.allDone'), 'success');
      updateTask('tagger', { status: 'done', message: t('hybridTagger.allDone') });
    } catch (e: any) {
      const errStr = typeof e === 'string' ? e : e?.message || String(e);
      taskLogs.appendCatchError(errStr, t('pages.errorPrefix'));
      setHasErr(true); setIsDone(true);
      updateTask('tagger', {
        status: /已取消|cancel/i.test(errStr) ? 'cancelled' : 'error',
        message: errStr,
      });
    } finally {
      setProcessing(false);
      setPhase('');
    }
  };

  const clearLogs = () => { setLogs([]); setIsDone(false); setHasErr(false); };

  const numInput = (v: string, set: (n: number) => void, fallback: number, min: number, max: number) => {
    const n = parseFloat(v);
    set(Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback);
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
      {/* 数据集路径 */}
      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('llmTagger.datasetPath')}</span>
          <label style={{ display: 'flex', alignItems: 'center', gap: 4, cursor: 'pointer', fontSize: 11, color: 'var(--color-text-secondary)' }}>
            <Checkbox checked={recursive} onChange={setRecursive} size={14} />
            {t('llmTagger.recursiveScan')}
          </label>
        </div>
        <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
          <input className="form-input" placeholder={t('llmTagger.selectFolder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
          <InputPathPickerButton onSelect={setInputPath} />
        </div>
      </div>

      {/* 本地模型 | LLM 模型 —— 一行两块 */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)', alignItems: 'stretch' }}>
        {/* 本地打标 */}
        <div className="tool-panel" style={{ marginBottom: 0 }}>
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('hybridTagger.localPhase')}</span>
            <div style={{ display: 'flex', gap: 4 }}>
              {(['cpu', 'gpu'] as const).map(hw => {
                const isGpu = hw === 'gpu';
                const active = isGpu === useGpu;
                const Icon = isGpu ? Gpu : Cpu;
                const color = isGpu ? '#4ade80' : '#fbbf24';
                return (
                  <button key={hw} onClick={() => setUseGpu(isGpu)} style={{
                    display: 'flex', alignItems: 'center', gap: 4, padding: '3px 10px',
                    borderRadius: 'var(--radius-sm)',
                    border: `1.5px solid ${active ? color : 'var(--color-border)'}`,
                    background: active ? (isGpu ? 'rgba(74,222,128,0.07)' : 'rgba(251,191,36,0.07)') : 'transparent',
                    cursor: 'pointer', fontSize: 11, fontWeight: 600,
                    color: active ? color : 'var(--color-text-tertiary)',
                  }}>
                    <Icon style={{ width: 13, height: 13 }} /> {hw.toUpperCase()}
                  </button>
                );
              })}
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <CustomSelect
              value={selectedModel}
              onChange={setSelectedModel}
              options={models.map(m => ({ value: m.id, label: m.is_downloaded ? m.name : `${m.name} (${t('hybridTagger.notDownloaded')})` }))}
            />
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6 }}>
              {(() => {
                const curModel = models.find(m => m.id === selectedModel);
                const supported = new Set(curModel?.supported_categories || cats.map(c => c.key));
                return cats.map(c => {
                  const on = enabledCats.has(c.key);
                  const avail = supported.has(c.key);
                  return (
                    <div key={c.key} onClick={() => { if (avail) toggleCat(c.key); }}
                      title={avail ? undefined : t('hybridTagger.catUnsupported')}
                      style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 8px', borderRadius: 'var(--radius-sm)', border: `1px solid ${on && avail ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: !avail ? 'rgba(0,0,0,0.04)' : on ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)', cursor: avail ? 'pointer' : 'not-allowed', transition: 'all 0.15s', opacity: avail ? 1 : 0.35, minWidth: 0 }}>
                      <div style={{ width: 14, height: 14, borderRadius: 3, minWidth: 14, border: `2px solid ${on && avail ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, background: on && avail ? 'var(--color-accent-primary)' : 'transparent', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{on && avail && <Check style={{ width: 9, height: 9, color: '#fff' }} />}</div>
                      <span style={{ fontSize: 12, fontWeight: 600, color: avail ? 'var(--color-text-primary)' : 'var(--color-text-tertiary)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.label}</span>
                    </div>
                  );
                });
              })()}
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-4)' }}>
              <div style={{ flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                  <span style={{ fontSize: 12 }}>{t('aiTagger.generalTh')}</span>
                  <span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{genTh.toFixed(2)}</span>
                </label>
                <input type="range" min="0.05" max="1" step="0.01" value={genTh} onChange={e => numInput(e.target.value, setGenTh, 0.35, 0.05, 1)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
              </div>
              <div style={{ flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                  <span style={{ fontSize: 12 }}>{t('aiTagger.charTh')}</span>
                  <span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{charTh.toFixed(2)}</span>
                </label>
                <input type="range" min="0.05" max="1" step="0.01" value={charTh} onChange={e => numInput(e.target.value, setCharTh, 0.85, 0.05, 1)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
              </div>
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-4)', flexWrap: 'wrap' }}>
              <Checkbox checked={replaceUnderscore} onChange={setReplaceUnderscore} label={t('aiTagger.replaceUnderscore')} size={14} />
              <span title={t('aiTagger.escapeParenthesesTip')}>
                <Checkbox checked={escapeParentheses} onChange={setEscapeParentheses} label={t('aiTagger.escapeParentheses')} size={14} />
              </span>
              <span title={t('hybridTagger.preferExistingTip')}>
                <Checkbox checked={preferExisting} onChange={setPreferExisting} label={t('hybridTagger.preferExisting')} size={14} />
              </span>
            </div>
          </div>
        </div>

        {/* API 设置 */}
        <div className="tool-panel" style={{ marginBottom: 0 }}>
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('llmTagger.apiSettings')}</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              {saveMsg && <span style={{ fontSize: 11, color: saveMsg.ok ? '#4ade80' : '#f87171' }}>{saveMsg.ok ? '✓' : '✗'} {saveMsg.text}</span>}
              <button className="btn btn-ghost btn-sm" onClick={handleSaveConfig} style={{ padding: '2px 8px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}>
                <Save style={{ width: 12, height: 12 }} /> {t('llmTagger.saveConfig')}
              </button>
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <div style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
                {Object.entries(PRESETS).map(([key, { label }]) => (
                  <button key={key} className={`btn btn-sm ${preset === key ? 'btn-primary' : 'btn-secondary'}`} onClick={() => setPreset(key)} style={{ fontSize: 11 }}>{label}</button>
                ))}
              </div>
              {preset === 'custom' ? (
                <input className="form-input" placeholder="https://api.example.com/v1/" value={customEndpoint} onChange={e => setCustomEndpoint(e.target.value)} style={{ marginTop: 6 }} />
              ) : (
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>{endpoint}</div>
              )}
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Key style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> API Key</label>
              <div style={{ position: 'relative' }}>
                <input className="form-input" type={showKey ? 'text' : 'password'} placeholder="sk-..." value={apiKey} onChange={e => setApiKey(e.target.value)} style={{ paddingRight: 32 }} />
                <button onClick={() => setShowKey(!showKey)} style={{ position: 'absolute', right: 6, top: '50%', transform: 'translateY(-50%)', background: 'none', border: 'none', cursor: 'pointer', color: 'var(--color-text-tertiary)', display: 'flex', padding: 2 }}>
                  {showKey ? <EyeOff style={{ width: 14, height: 14 }} /> : <Eye style={{ width: 14, height: 14 }} />}
                </button>
              </div>
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Bot style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.modelLabel')}</span>
                <button className="btn btn-ghost btn-sm" onClick={handleFetchModels} disabled={fetchingModels || !endpoint} style={{ padding: '2px 8px', fontSize: 11 }}>
                  {fetchingModels ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 12, height: 12 }} />} {t('llmTagger.fetchModels')}
                </button>
              </label>
              {modelList.length > 0 ? (
                <CustomSelect value={modelName} onChange={setModelName} options={modelList.map(m => ({ value: m, label: m }))} />
              ) : (
                <input className="form-input" placeholder={t('llmTagger.modelPlaceholder')} value={modelName} onChange={e => setModelName(e.target.value)} />
              )}
              {fetchMsg && <div style={{ fontSize: 11, marginTop: 4, color: fetchMsg.ok ? '#4ade80' : '#f87171' }}>{fetchMsg.ok ? '✓' : '✗'} {fetchMsg.text}</div>}
            </div>
          </div>
        </div>
      </div>

      {/* 调优设置聚合：提示词 + 参数 + 输出格式 */}
      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('hybridTagger.llmPhase')}</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <CustomSelect compact value={presetId} onChange={applyPreset}
              options={allPresets.map(p => ({ value: p.id, label: p.name }))} style={{ width: 190 }} />
            <button className="btn btn-ghost btn-sm" style={{ fontSize: 10, padding: '4px 6px' }}
              title={t('hybridTagger.savePreset')}
              onClick={() => { setNewPresetName(isCustomPreset ? (allPresets.find(p => p.id === presetId)?.name || '') : ''); setShowSaveModal(true); }}>
              <Save style={{ width: 13, height: 13 }} />
            </button>
            {/* 常驻显示，内置预设时置灰——藏起来会让人以为没有删除功能 */}
            <button className="btn btn-ghost btn-sm" disabled={!isCustomPreset}
              style={{ fontSize: 10, padding: '4px 6px', color: isCustomPreset ? '#f87171' : undefined, opacity: isCustomPreset ? 1 : 0.35 }}
              title={t('hybridTagger.deletePreset')} onClick={handleDeletePreset}>
              <Trash2 style={{ width: 13, height: 13 }} />
            </button>
            <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }}
              onClick={() => applyPreset(presetId)}>{t('tagSort.resetDefault')}</button>
          </div>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 300px', gap: 'var(--space-4)', alignItems: 'stretch' }}>
          {/* 左：提示词 */}
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            <label className="form-label">{t('hybridTagger.promptLabel')}</label>
            <textarea className="form-input" value={prompt} onChange={e => setPrompt(e.target.value)}
              style={{ fontSize: 11, fontFamily: 'monospace', lineHeight: 1.6, resize: 'vertical', flex: 1, minHeight: 200 }} />
          </div>
          {/* 右：参数 + 输出 */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.temperature')}</span>
                <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{temperature}</span>
              </label>
              <input type="range" min="0" max="2" step="0.05" value={temperature} onChange={e => setTemperature(e.target.value)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            <div>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
                <span>Top P</span>
                <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{topP || '0'}</span>
              </label>
              <input type="range" min="0" max="1" step="0.05" value={topP || '0'} onChange={e => setTopP(e.target.value)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              <div style={{ flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 4 }}><ImageIcon style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.imageSize')}</label>
                <input className="form-input" type="number" min="256" max="4096" step="128" value={imageSize} onChange={e => setImageSize(e.target.value)} />
              </div>
              <div style={{ flex: 1 }}>
                <label className="form-label">{t('hybridTagger.concurrency')}</label>
                <input className="form-input" type="number" min={1} max={16} value={concurrency} onChange={e => setConcurrency(e.target.value)} />
              </div>
              <div style={{ flex: 1 }}>
                <label className="form-label">{t('hybridTagger.interval')}</label>
                <input className="form-input" type="number" step="0.1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)} />
              </div>
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 4 }}><Focus style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.imageDetail')}</label>
                <CustomSelect value={imageDetail} onChange={setImageDetail} options={IMAGE_DETAIL_OPTIONS(t)} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <label className="form-label">{t('hybridTagger.outputFormat')}</label>
                <CustomSelect
                  value={outputFormat}
                  onChange={handleFormatChange}
                  options={[
                    { value: 'txt', label: t('hybridTagger.formatTxt') },
                    { value: 'json', label: t('hybridTagger.formatJson') },
                    { value: 'json_simplified', label: t('hybridTagger.formatJsonSimplified') },
                  ]}
                />
              </div>
            </div>
            <div>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <Hash style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /> {t('hybridTagger.triggerWord')}
              </label>
              <input className="form-input" value={triggerWord}
                onChange={e => {
                  setTriggerWord(e.target.value);
                  try { localStorage.setItem(TRIGGER_WORD_KEY, e.target.value); } catch { /* 配额满等，忽略 */ }
                }} />
            </div>
          </div>
        </div>
      </div>

      {/* 操作 + 进度 + 日志 */}
      <ProcessButton
        processing={processing}
        onStart={handleStart}
        disabled={!canStart}
        cancelCommand={phase === 'refining' ? 'cancel_tag_refining' : 'force_cancel_tagging'}
        startText={t('hybridTagger.startText')}
        processingText={
          phase === 'converting' ? t('hybridTagger.phaseShortConverting')
          : phase === 'tagging' ? t('hybridTagger.phaseShortTagging')
          : phase === 'refining' ? t('hybridTagger.phaseShortRefining')
          : t('pages.processing')
        }
        onCancelLog={(msg) => {
          cancelRequestedRef.current = true;
          setLogs(prev => [...prev, { time: getTimeStr(), message: msg, status: 'warning' }]);
        }}
      />
      <ProgressLog
        progress={progress}
        current={pCur}
        total={pTot}
        logs={logs}
        isDone={isDone}
        hasError={hasErr}
        onClearLogs={clearLogs}
      />

      <Modal open={showSaveModal} onClose={() => setShowSaveModal(false)} title={t('hybridTagger.savePreset')}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <input className="form-input" autoFocus value={newPresetName}
            placeholder={t('hybridTagger.presetNamePlaceholder')}
            onChange={e => setNewPresetName(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') handleSavePreset(); }} />
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 'var(--space-2)' }}>
            <button className="btn btn-secondary btn-sm" onClick={() => setShowSaveModal(false)}>{t('common.cancel')}</button>
            <button className="btn btn-primary btn-sm" disabled={!newPresetName.trim()} onClick={handleSavePreset}>{t('common.save')}</button>
          </div>
        </div>
      </Modal>
    </div>
  );
}
