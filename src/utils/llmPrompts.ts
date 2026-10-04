const defaultSystemPrompt_txt = `You are a professional image captioning assistant. Provide a detailed, natural language description of the image suitable for training image generation models.`;
const defaultUserPrompt_txt = `Please describe this image in detail.`;

// JSON 完整格式 (Full) — 嵌套 ai_output 结构
const defaultSystemPrompt_json_full = `You are an anime image tagging expert. Output ONLY valid JSON.

Output a JSON object with an "ai_output" wrapper. Fields (tag fields are arrays of lowercase strings):
1. count: string - Character count ("1girl", "2boys", "1girl, 1boy", "no humans")
2. appearance: string[] - Visual features (hair color, eye color, hairstyle, clothing, accessories)
3. tags: string[] - Actions, expressions, poses, composition, objects
4. environment: string[] - Background, location, lighting, atmosphere
5. nl: string - One sentence natural language description

Rules:
- Use lowercase English booru-style tags
- Each tag is a separate array element
- Only describe what is clearly visible
- Be detailed but don't repeat tags
- Output ONLY the JSON object, no markdown or explanation

Example:
{"ai_output": {"count": "1girl", "appearance": ["long hair", "blue eyes", "school uniform"], "tags": ["smile", "standing", "looking at viewer"], "environment": ["classroom", "window", "sunlight"], "nl": "A cheerful girl stands by the window in a sunny classroom."}}`;

const defaultUserPrompt_json_full = `Analyze this image and output structured tags as JSON with the "ai_output" wrapper.`;

// JSON 简化格式 (Simplified) — 扁平结构
const defaultSystemPrompt_json_simplified = `You are an anime image tagging expert. Output ONLY valid JSON.

JSON fields (tag fields are arrays of lowercase strings):
1. count: string - Character count ("1girl", "2boys", "1girl, 1boy", "no humans")
2. appearance: string[] - Visual features (hair color, eye color, hairstyle, clothing, accessories)
3. tags: string[] - Actions, expressions, poses, composition, objects
4. environment: string[] - Background, location, lighting, atmosphere
5. nl: string - One sentence natural language description

Rules:
- Use lowercase English booru-style tags
- Each tag is a separate array element
- Only describe what is clearly visible
- Be detailed but don't repeat tags
- Output ONLY the JSON object, no markdown or explanation

Example:
{"count": "1girl", "appearance": ["long hair", "blue eyes", "school uniform"], "tags": ["smile", "standing", "looking at viewer"], "environment": ["classroom", "window", "sunlight"], "nl": "A cheerful girl stands by the window in a sunny classroom."}`;

const defaultUserPrompt_json_simplified = `Analyze this image and output structured tags as a flat JSON object.`;

export function getDefaultPrompts(format: 'txt' | 'json', simplified: boolean) {
  if (format === 'json') {
    return simplified
      ? { sys: defaultSystemPrompt_json_simplified, user: defaultUserPrompt_json_simplified }
      : { sys: defaultSystemPrompt_json_full, user: defaultUserPrompt_json_full };
  }
  return { sys: defaultSystemPrompt_txt, user: defaultUserPrompt_txt };
}

const ALL_DEFAULTS = [getDefaultPrompts('txt', false), getDefaultPrompts('json', false), getDefaultPrompts('json', true)];

/**
 * 切换输出格式后的提示词：为空或仍是某个格式的默认提示词的那一项换成新格式的默认值，
 * 用户改过的保持不变。系统提示词和用户提示词分别判断。
 *
 * 例：`const next = switchDefaultPrompts({ sys, user }, 'json', simplified); setSys(next.sys); setUser(next.user);`
 */
export function switchDefaultPrompts(
  current: { sys: string; user: string },
  format: 'txt' | 'json',
  simplified: boolean,
): { sys: string; user: string } {
  const next = getDefaultPrompts(format, simplified);
  const isDefault = (value: string, field: 'sys' | 'user') =>
    !value.trim() || ALL_DEFAULTS.some(prompts => prompts[field] === value);
  return {
    sys: isDefault(current.sys, 'sys') ? next.sys : current.sys,
    user: isDefault(current.user, 'user') ? next.user : current.user,
  };
}

// ── 标签调优（标签细化、辅助打标），{tags} 由后端替换为该图现有标签 ──

interface RefinePromptParts {
  /** 现有标签的来源，接在 "its existing tags" 之后 */
  source: string;
  /** 第 2 条改正错误标签的举例 */
  examples: string;
  /** 第 5 条要求保持的标签写法 */
  format: string;
}

const refineIntro = ({ source }: RefinePromptParts) =>
  `You are an expert anime image tagger. You will receive an image and its existing tags${source}.`;

const refineTasks = ({ examples, format }: RefinePromptParts) => `1. Compare the image content with the existing tags
2. Fix incorrect tags (e.g. ${examples})
3. Add important missing tags that are clearly visible in the image
4. Remove tags that do not match the image at all
5. Keep the tag format consistent (${format})`;

function tagListRefinePrompt(parts: RefinePromptParts) {
  return `${refineIntro(parts)}

Your task:
${refineTasks(parts)}

Rules:
- Only make changes you are confident about
- Preserve tags that are correct
- Return ONLY the refined tags, comma-separated
- Do NOT add explanations

Existing tags: {tags}

Refined tags:`;
}

/** 标签细化：文件里的现有标签，沿用下划线写法 */
export const TAG_REFINE_PROMPT = tagListRefinePrompt({
  source: '',
  examples: 'wrong hair color, wrong clothing',
  format: 'lowercase, underscores',
});

/** 辅助打标：标签来自本地打标模型 */
const LOCAL_TAGGER: RefinePromptParts = {
  source: ' produced by a local tagger model',
  examples: 'wrong hair color, wrong clothing, wrong subject count',
  format: 'lowercase danbooru-style tags',
};

/** 辅助打标 TXT：补充缺失、删除错误、修复不准确 */
export const HYBRID_PROMPT_TXT = tagListRefinePrompt(LOCAL_TAGGER);

// 四类语义按 AnimaLoraStudio 打标文档：表情/姿势/构图属 tags 而非 appearance
const JSON_CATEGORIES = `Categories:
- COUNT: character count only, e.g. 1girl, 2boys, 1girl 1boy, no humans
- APPEARANCE: the character's visual features - hair color, hairstyle, eye color, clothing, accessories
- TAGS: actions, expressions, poses, composition, objects held or used, e.g. smile, standing, looking at viewer, upper body
- ENVIRONMENT: background, location, lighting, atmosphere, e.g. simple background, white background, outdoors, classroom, night, sunlight`;

const JSON_REPLY_FORMAT = `Reply in exactly this format (five lines, nothing else):
COUNT: <comma-separated>
APPEARANCE: <comma-separated>
TAGS: <comma-separated>
ENVIRONMENT: <comma-separated>
NL: <natural language description>`;

/**
 * 辅助打标 JSON：在 TXT 的基础上把标签重新分到 count/appearance/tags/environment 并补写 nl。
 * 本地打标器这四类是靠关键词表猜的（"simple background" 之类常落错格），由 LLM 按画面重新归类；
 * 后端解析 COUNT:/APPEARANCE:/TAGS:/ENVIRONMENT:/NL: 标记段。
 * quality/series/artist/character 来自打标模型的分类，不在重排范围内。
 */
export const HYBRID_PROMPT_JSON = `${refineIntro(LOCAL_TAGGER)}

Your task:
${refineTasks(LOCAL_TAGGER)}
6. Sort every remaining tag into the correct category (the local tagger often puts tags in the wrong one)
7. Write a natural language description (1-2 sentences) of the image, consistent with the image content and the final tags

${JSON_CATEGORIES}

Rules:
- Only make changes you are confident about
- Preserve tags that are correct
- Every tag must appear in exactly one category
- Leave a category line empty if it has no tags
- Character names, series names, artist names and quality tags are handled separately - do not list them
- Do NOT add explanations

Existing tags: {tags}

${JSON_REPLY_FORMAT}`;

/** 仅补 nl：本地打标器不产生 nl 字段；后端见到只有 NL: 一段的回复会保留原标签 */
export const HYBRID_PROMPT_NL_ONLY = `${refineIntro(LOCAL_TAGGER)}

Your task:
Write a natural language description (1-2 sentences) of the image, consistent with the image content and the existing tags.

Rules:
- Do NOT modify, add or remove any tags
- Describe only what is clearly visible
- Do NOT add explanations

Existing tags: {tags}

Reply in exactly this format (one line, nothing else):
NL: <natural language description>`;

/**
 * 归类字段 + 补描述（JSON 专用）：LLM 只决定每个标签属于哪个字段并补 nl。
 * 标签集合由后端 preserve_tags 保证恒定，不指望模型遵守"不要增删"的嘱咐。
 */
export const HYBRID_PROMPT_SORT_ONLY = `You are an expert anime image tagger. You will receive an image and its existing tags, already produced by a local tagger.

Your task is ONLY to put each given tag into the correct category, and to write a natural language description. You must NOT change the tag set itself.

Rules:
- Use every tag you are given, exactly as written — do not reword, merge or split them
- Do NOT add any tag that is not in the list, even if you can see something untagged in the image
- Do NOT remove any tag, even if you believe it does not match the image
- Every tag must appear in exactly one category
- Leave a category line empty if nothing belongs to it
- Character names, series names, artist names and quality tags are handled separately, they are not in your list

${JSON_CATEGORIES}

Then write a natural language description (1-2 sentences) of the image, consistent with the image content and the tags.

Existing tags: {tags}

${JSON_REPLY_FORMAT}`;

/**
 * 详细自然语言打标（txt 专用）：整段回复就是 .txt 的全部内容。本地标签只作校准参考
 * （视觉模型容易认错发色/人数/服装这类离散属性），不写回文件；后端走 caption_mode，不解析标签。
 * {trigger} 由 applyTriggerWord 换成触发词。
 */
export const HYBRID_PROMPT_DETAILED_CAPTION = `You are a professional image captioning assistant producing detailed anime-style captions for AI art training. You will receive an image and the existing tags produced by a local anime tagger model. Describe ONLY what is actually visible. Never speculate, never critique the artwork, never add unrelated remarks.

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

/**
 * 把提示词里的 {trigger} 换成触发词。
 * 触发词为空时整行删掉——否则会给模型留下 `beginning with ","` 这种残句。
 */
export function applyTriggerWord(prompt: string, trigger: string): string {
  const word = trigger.trim();
  if (word) return prompt.split('{trigger}').join(word);
  return prompt.split('\n').filter(line => !line.includes('{trigger}')).join('\n');
}

// ── 标签排序 ──

export const TAG_SORT_PROMPT = `Please sort the following tags in this order: character count (e.g. 1girl) → character name → series/source → artist → features → clothing → expression details → clothing details → camera angle/perspective → actions → scene/environment → others.

Important rules:
1. Only rearrange the order of existing tags
2. Do NOT add any new tags, do NOT remove any original tags
3. Return ONLY the sorted tags, comma-separated

Tags to sort: {tags}

Sorted tags:`;
