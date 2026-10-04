import { moveTagWithin, sameTags } from './tagText';

export interface JsonTagData {
  fixed: { quality?: string; series?: string; artist?: string; [key: string]: unknown };
  character: { name: string; variant: string; [key: string]: unknown };
  from_path: { appearance: string[]; [key: string]: unknown };
  ai_output: { count?: string; appearance: string[]; tags: string[]; environment: string[]; nl?: string; [key: string]: unknown };
  [key: string]: unknown;
}

const csv = (value: unknown) => typeof value === 'string' ? value.split(',').map(v => v.trim()).filter(Boolean) : [];
function field(section: 'fixed' | 'character' | 'from_path' | 'ai_output', name: string, kind: 'csv' | 'list', labelKey: string, color: string, simplified = true) {
  return {
    key: `${section}.${name}`, section, name, kind, labelKey: `jsonTag.${labelKey}`, color, simplified,
    get: (data: JsonTagData): string[] => kind === 'csv' ? csv(data[section][name]) : data[section][name] as string[],
    set: (data: JsonTagData, values: string[]): JsonTagData => {
      const previous = kind === 'csv' ? csv(data[section][name]) : data[section][name] as string[];
      const value = kind === 'csv' ? values.join(', ') : values;
      if (sameTags(previous, values) && (kind === 'list' || value === data[section][name])) return data;
      return { ...data, [section]: { ...data[section], [name]: value } };
    },
  };
}

export const JSON_FIELDS = [
  field('fixed', 'quality', 'csv', 'fieldQuality', '#f59e0b'),
  field('fixed', 'series', 'csv', 'fieldSeries', '#f59e0b'),
  field('fixed', 'artist', 'csv', 'fieldArtist', '#f59e0b'),
  field('character', 'name', 'csv', 'fieldCharacter', '#f472b6'),
  field('character', 'variant', 'csv', 'fieldVariant', '#f472b6', false),
  field('from_path', 'appearance', 'list', 'fieldFromPath', '#22d3ee', false),
  field('ai_output', 'count', 'csv', 'fieldCount', '#818cf8'),
  field('ai_output', 'appearance', 'list', 'fieldAppearance', '#c084fc'),
  field('ai_output', 'tags', 'list', 'fieldTags', '#60a5fa'),
  field('ai_output', 'environment', 'list', 'fieldEnvironment', '#34d399'),
];

export const collectAllTags = (data: JsonTagData) => JSON_FIELDS.flatMap(field => field.get(data));

export interface JsonTagPosition { field: string; index: number; }

const ARTIST_FIELD = 'fixed.artist';

/** 画师按约定每位都带 @：拖进画师字段时补上，拖出时去掉 */
function valueForField(value: string, from: string, to: string) {
  if (to === ARTIST_FIELD && from !== ARTIST_FIELD) return value.startsWith('@') ? value : `@${value}`;
  if (from === ARTIST_FIELD && to !== ARTIST_FIELD) return value.replace(/^@+/, '');
  return value;
}

export function moveJsonTag(data: JsonTagData, source: JsonTagPosition & { value: string }, target: JsonTagPosition, simplified: boolean) {
  const from = JSON_FIELDS.find(field => field.key === source.field);
  const to = JSON_FIELDS.find(field => field.key === target.field);
  if (!from || !to || simplified && (!from.simplified || !to.simplified)) return data;
  const sourceValues = from.get(data), targetValues = to.get(data);
  if (!Number.isInteger(source.index) || source.index < 0 || sourceValues[source.index] !== source.value
    || !Number.isInteger(target.index) || target.index < 0 || target.index > targetValues.length) return data;
  if (from === to) {
    const moved = moveTagWithin(sourceValues, source.index, target.index);
    return moved === sourceValues ? data : from.set(data, moved);
  }
  const value = valueForField(source.value, from.key, to.key);
  if (!value) return data;
  const next = from.set(data, sourceValues.filter((_, index) => index !== source.index));
  if (targetValues.some(existing => existing.toLowerCase() === value.toLowerCase())) return next;
  const inserted = [...targetValues];
  inserted.splice(target.index, 0, value);
  return to.set(next, inserted);
}

// 与 tag_manager::to_simplified 一致；完整格式保留扩展字段，缺省的固定字段补空值
export function jsonTagPreview(data: JsonTagData, simplified: boolean) {
  if (!simplified) return { ...data,
    fixed: { ...data.fixed, quality: data.fixed.quality ?? '', series: data.fixed.series ?? '', artist: data.fixed.artist ?? '' },
    ai_output: { ...data.ai_output, count: data.ai_output.count ?? '', nl: data.ai_output.nl ?? '' },
  };
  const appearance = [...data.from_path.appearance];
  for (const tag of data.ai_output.appearance) {
    if (!appearance.includes(tag)) appearance.push(tag);
  }
  return {
    quality: data.fixed.quality ?? '', series: data.fixed.series ?? '', artist: data.fixed.artist ?? '',
    character: data.character.name, count: data.ai_output.count ?? '',
    appearance,
    tags: data.ai_output.tags, environment: data.ai_output.environment, nl: data.ai_output.nl ?? '',
  };
}
