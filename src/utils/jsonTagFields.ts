import { sameTags } from './tagText';

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

export function moveJsonTag(data: JsonTagData, source: JsonTagPosition & { value: string }, target: JsonTagPosition, simplified: boolean) {
  const from = JSON_FIELDS.find(field => field.key === source.field);
  const to = JSON_FIELDS.find(field => field.key === target.field);
  if (!from || !to || simplified && (!from.simplified || !to.simplified)) return data;
  const sourceValues = from.get(data), targetValues = to.get(data);
  if (!Number.isInteger(source.index) || source.index < 0 || sourceValues[source.index] !== source.value
    || !Number.isInteger(target.index) || target.index < 0 || target.index > targetValues.length) return data;
  const remaining = sourceValues.filter((_, index) => index !== source.index);
  if (from === to) {
    const index = target.index > source.index ? target.index - 1 : target.index;
    if (index === source.index) return data;
    remaining.splice(index, 0, source.value);
    return from.set(data, remaining);
  }
  const next = from.set(data, remaining);
  if (targetValues.some(value => value.toLowerCase() === source.value.toLowerCase())) return next;
  const inserted = [...targetValues];
  inserted.splice(target.index, 0, source.value);
  return to.set(next, inserted);
}

// Mirrors tag_manager::to_simplified; full JSON retains extension fields and empty schema values.
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
