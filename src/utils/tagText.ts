export const normalizeEditableTag = (tag: string) => tag.trim().replace(/_/g, ' ').replace(/\s+/g, ' ').trim();

/** 比较两个标签是否同一个：忽略大小写、下划线与空格写法的差别（long_hair 与 Long Hair 相同） */
export const tagKey = (tag: string) => normalizeEditableTag(tag).toLowerCase();

/** 按小写去重并去掉空白项；seen 可跨多次调用共享（JSON 各字段之间去重） */
export function dedupeTags(values: readonly string[], seen = new Set<string>()) {
  const tags = values.map(v => v.trim()).filter(tag => {
    const key = tag.toLowerCase();
    if (!tag || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return { tags, changed: !sameTags(values, tags) };
}

export const splitTagInput = (raw: string, normalize = normalizeEditableTag) => dedupeTags(raw.split(/[,，]/).map(normalize)).tags;
export const sameTags = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((v, i) => v === b[i]);

export function replaceTagAtIndex(values: string[], index: number, raw: string, normalize = normalizeEditableTag) {
  const tag = normalize(raw);
  if (!tag || index < 0 || index >= values.length || values[index] === tag) return values;
  const next = [...values];
  if (next.some((v, i) => i !== index && v.toLowerCase() === tag.toLowerCase())) next.splice(index, 1);
  else next[index] = tag;
  return next;
}

/** 把 from 位置的标签挪到插入点 to（0..length，按挪动前的位置计）；位置无效或不动时返回原数组 */
export function moveTagWithin<T>(values: T[], from: number, to: number): T[] {
  if (!Number.isInteger(from) || from < 0 || from >= values.length
    || !Number.isInteger(to) || to < 0 || to > values.length) return values;
  const index = to > from ? to - 1 : to;
  if (index === from) return values;
  const next = [...values];
  const [moved] = next.splice(from, 1);
  next.splice(index, 0, moved);
  return next;
}

/** 已选标签里的 from 改名为 to（没选中 from 时原样返回） */
export function renameSelected(selected: Set<string>, from: string, to: string): Set<string> {
  if (!selected.has(from) || from === to) return selected;
  const next = new Set(selected);
  next.delete(from);
  next.add(to);
  return next;
}

/**
 * danbooru 原形：空格→下划线、括号转义为 \( \)。
 * 先把已有转义剥掉再统一重转，保证幂等（已转义的不会被转成 \\(）
 */
export const toDanbooruEscaped = (tag: string) =>
  tag.trim().toLowerCase()
    .replace(/\\([()])/g, '$1')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/ /g, '_')
    .replace(/\(/g, '\\(')
    .replace(/\)/g, '\\)');

/**
 * 数据集里已有 danbooru 原形标签（带下划线或转义括号）→ 新标签沿用同种形式。
 * 不然一份 txt 里 "long hair" 和 "long_hair" 两种格式混着，训练时是两个 token
 */
const hasEscapedTags = (tags: readonly string[]) => tags.some(t => t.includes('_') || t.includes('\\('));

/** 新标签按 pool（当前图片或整个数据集的标签）的写法规范化：原形或小写空格形式 */
export const normalizeLikeTags = (raw: string, pool: readonly string[]) =>
  hasEscapedTags(pool) ? toDanbooruEscaped(raw) : tagKey(raw);

// 同一标签恒定同色：按名称哈希取色
const CHIP_COLORS = [
  { bg: 'rgba(124,92,252,0.10)', bd: 'rgba(124,92,252,0.25)', tx: '#a78bfa' },
  { bg: 'rgba(96,165,250,0.10)', bd: 'rgba(96,165,250,0.25)', tx: '#60a5fa' },
  { bg: 'rgba(74,222,128,0.10)', bd: 'rgba(74,222,128,0.25)', tx: '#4ade80' },
  { bg: 'rgba(251,191,36,0.10)', bd: 'rgba(251,191,36,0.25)', tx: '#fbbf24' },
  { bg: 'rgba(248,113,113,0.10)', bd: 'rgba(248,113,113,0.25)', tx: '#f87171' },
  { bg: 'rgba(192,132,252,0.10)', bd: 'rgba(192,132,252,0.25)', tx: '#c084fc' },
  { bg: 'rgba(45,212,191,0.10)', bd: 'rgba(45,212,191,0.25)', tx: '#2dd4bf' },
  { bg: 'rgba(251,146,60,0.10)', bd: 'rgba(251,146,60,0.25)', tx: '#fb923c' },
  { bg: 'rgba(236,72,153,0.10)', bd: 'rgba(236,72,153,0.25)', tx: '#ec4899' },
  { bg: 'rgba(132,204,22,0.10)', bd: 'rgba(132,204,22,0.25)', tx: '#84cc16' },
  { bg: 'rgba(14,165,233,0.10)', bd: 'rgba(14,165,233,0.25)', tx: '#0ea5e9' },
  { bg: 'rgba(234,179,8,0.10)', bd: 'rgba(234,179,8,0.25)', tx: '#eab308' },
  { bg: 'rgba(168,85,247,0.10)', bd: 'rgba(168,85,247,0.25)', tx: '#a855f7' },
  { bg: 'rgba(20,184,166,0.10)', bd: 'rgba(20,184,166,0.25)', tx: '#14b8a6' },
  { bg: 'rgba(239,68,68,0.10)', bd: 'rgba(239,68,68,0.25)', tx: '#ef4444' },
  { bg: 'rgba(34,197,94,0.10)', bd: 'rgba(34,197,94,0.25)', tx: '#22c55e' },
] as const;

export type TagChipColor = (typeof CHIP_COLORS)[number];

function tagHash(tag: string) {
  let hash = 0;
  for (let i = 0; i < tag.length; i++) hash = ((hash << 5) - hash + tag.charCodeAt(i)) | 0;
  return Math.abs(hash);
}

/** TXT 标签芯片与标签统计：16 色 */
export function getTagChipColor(tag: string): TagChipColor {
  return CHIP_COLORS[tagHash(tag) % CHIP_COLORS.length];
}

/** JSON 模式的标签统计列表：取 16 色里的前 10 种 */
export function getJsonStatColor(tag: string): TagChipColor {
  return CHIP_COLORS[tagHash(tag) % 10];
}
