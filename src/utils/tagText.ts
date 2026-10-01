export const normalizeEditableTag = (tag: string) => tag.trim().replace(/_/g, ' ').replace(/\s+/g, ' ').trim();

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

const colors = ['124,92,252', '96,165,250', '74,222,128', '251,191,36', '248,113,113', '192,132,252', '45,212,191', '251,146,60', '236,72,153', '132,204,22'];
export function getTagChipColor(tag: string) {
  let hash = 0;
  for (let i = 0; i < tag.length; i++) hash = ((hash << 5) - hash + tag.charCodeAt(i)) | 0;
  const color = colors[Math.abs(hash) % colors.length];
  return { bg: `rgba(${color},0.10)`, bd: `rgba(${color},0.25)`, tx: `rgb(${color})` };
}
