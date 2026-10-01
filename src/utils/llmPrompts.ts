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
