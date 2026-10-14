export type MetadataField = {
  id: string;
  label: string;
  value: string;
  kind: 'text' | 'number' | 'boolean';
  multiline?: boolean;
  group?: string;
};

type Span = { start: number; end: number };
type EditableField = MetadataField & Span & { encoding?: 'json' | 'lora'; insert?: string; suffix?: string };
type JsonNode = Span & {
  kind: 'object' | 'array' | 'string' | 'number' | 'boolean' | 'null';
  value?: string;
  properties?: { key: string; node: JsonNode }[];
  items?: JsonNode[];
};

const JSON_NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;
const invalidJson = () => new Error('sdMetadata.invalidJson');
const invalidField = () => new Error('sdMetadata.invalidField');

// Keep numeric tokens and source ranges intact; parsing a seed as a JS number can lose digits.
function parseJson(text: string): JsonNode {
  let offset = 0;
  const whitespace = () => { while (/[\t\n\r ]/.test(text[offset] ?? '\0')) offset++; };
  const string = (): string => {
    const start = offset++;
    while (offset < text.length) {
      const char = text[offset++];
      if (char === '"') {
        try { return JSON.parse(text.slice(start, offset)) as string; } catch { throw invalidJson(); }
      }
      if (char === '\\') offset++;
    }
    throw invalidJson();
  };
  const value = (depth: number): JsonNode => {
    if (depth > 512) throw invalidJson();
    whitespace();
    const start = offset;
    const char = text[offset];
    if (char === '"') return { kind: 'string', start, value: string(), end: offset };
    if (char === '{' || char === '[') {
      const object = char === '{';
      const endChar = object ? '}' : ']';
      offset++;
      whitespace();
      const properties: NonNullable<JsonNode['properties']> = [];
      const items: JsonNode[] = [];
      if (text[offset] !== endChar) {
        while (true) {
          whitespace();
          if (object) {
            if (text[offset] !== '"') throw invalidJson();
            const key = string();
            whitespace();
            if (text[offset++] !== ':') throw invalidJson();
            properties.push({ key, node: value(depth + 1) });
          } else {
            items.push(value(depth + 1));
          }
          whitespace();
          if (text[offset] === endChar) break;
          if (text[offset++] !== ',') throw invalidJson();
        }
      }
      offset++;
      return { kind: object ? 'object' : 'array', start, end: offset, properties, items };
    }
    for (const literal of ['true', 'false', 'null'] as const) {
      if (text.startsWith(literal, offset)) {
        offset += literal.length;
        return { kind: literal === 'null' ? 'null' : 'boolean', start, end: offset, value: literal };
      }
    }
    const number = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(text.slice(offset));
    if (!number) throw invalidJson();
    offset += number[0].length;
    return { kind: 'number', start, end: offset, value: number[0] };
  };
  const root = value(0);
  whitespace();
  if (offset !== text.length) throw invalidJson();
  return root;
}

function property(node: JsonNode, key: string): JsonNode | undefined {
  return node.properties?.find(item => item.key === key)?.node;
}

function jsonFields(keyword: string, text: string): EditableField[] {
  const root = parseJson(text);
  const result: EditableField[] = [];
  const visit = (node: JsonNode, address: number[], names: string[], group?: string, skipArrays = false) => {
    if (node.kind === 'object') {
      node.properties!.forEach(({ key, node: child }, index) => visit(child, [...address, index], [...names, key], group, skipArrays));
    } else if (node.kind === 'array') {
      if (!skipArrays) node.items!.forEach((child, index) => visit(child, [...address, index], [...names, `[${index}]`], group));
    } else if (node.kind !== 'null') {
      const label = skipArrays ? names.join('.') : names[names.length - 1];
      const value = node.value!;
      result.push({
        id: `json:${JSON.stringify(address)}`, label, value,
        kind: node.kind === 'string' ? 'text' : node.kind,
        multiline: node.kind === 'string' && (/prompt|caption|text|^uc$/i.test(label) || value.includes('\n') || value.length > 100),
        group: group ?? names.slice(0, -1).join('.'),
        start: node.start, end: node.end, encoding: 'json',
      });
    }
  };
  if (keyword.toLowerCase() === 'prompt') {
    root.properties?.forEach(({ key, node }, nodeIndex) => {
      const classType = property(node, 'class_type');
      const title = classType?.kind === 'string' ? classType.value : '';
      node.properties?.forEach(({ key: inputKey, node: inputs }, inputsIndex) => {
        if (inputKey === 'inputs' && inputs.kind === 'object') {
          // ComfyUI arrays are commonly [node_id, output_slot] graph connections.
          visit(inputs, [nodeIndex, inputsIndex], [], title ? `${key} · ${title}` : key, true);
        }
      });
    });
  } else {
    visit(root, [], []);
  }
  return result;
}

function trimmedSpan(text: string, start: number, end: number): Span {
  const originalStart = start;
  while (start < end && /\s/.test(text[start])) start++;
  if (start === end) return { start: originalStart, end: originalStart };
  while (end > start && /\s/.test(text[end - 1])) end--;
  return { start, end };
}

function a1111Fields(text: string): EditableField[] {
  const result: EditableField[] = [];
  const newline = text.includes('\r\n') ? '\r\n' : '\n';
  const paramsMatches = [...text.matchAll(/^Steps:[ \t]*/gm)];
  const paramsStart = paramsMatches[paramsMatches.length - 1]?.index ?? text.length;
  const negativeMatch = /^Negative prompt:[ \t]*/m.exec(text.slice(0, paramsStart));
  const negativeStart = negativeMatch?.index ?? paramsStart;
  const positive = trimmedSpan(text, 0, negativeStart);
  const negative = negativeMatch
    ? trimmedSpan(text, negativeStart + negativeMatch[0].length, paramsStart)
    : { start: paramsStart, end: paramsStart };
  const params = trimmedSpan(text, paramsStart, text.length);
  result.push({
    id: 'a1111:positive', label: 'sdMetadata.positive', value: text.slice(positive.start, positive.end), kind: 'text', multiline: true, ...positive,
    suffix: positive.start === negativeStart && negativeStart < text.length ? newline : undefined,
  });
  result.push({
    id: 'a1111:negative', label: 'sdMetadata.negative', value: text.slice(negative.start, negative.end), kind: 'text', multiline: true,
    ...negative, insert: negativeMatch ? undefined : `${paramsStart > 0 && !text.slice(0, paramsStart).endsWith('\n') ? newline : ''}Negative prompt: `,
  });
  result.push({
    id: 'a1111:params', label: 'sdMetadata.params', value: text.slice(params.start, params.end), kind: 'text', multiline: true,
    ...params, insert: paramsStart === text.length && text.length > 0 && !text.endsWith('\n') ? newline : undefined,
  });
  for (const [name, span] of [['positive', positive], ['negative', negative]] as const) {
    const prompt = text.slice(span.start, span.end);
    for (const [index, match] of [...prompt.matchAll(/<(?:lora|lyco):[^<>\r\n]+>/gi)].entries()) {
      const start = span.start + match.index!;
      result.push({ id: `a1111:lora:${name}:${index}`, label: 'sdMetadata.lora', value: match[0], kind: 'text', group: name, start, end: start + match[0].length, encoding: 'lora' });
    }
  }
  const rawParams = text.slice(params.start, params.end);
  const boundaries = [0];
  let quote = false;
  let escaped = false;
  const brackets: string[] = [];
  for (let i = 0; i < rawParams.length; i++) {
    const char = rawParams[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (char === '\\') escaped = true;
      else if (char === '"') quote = false;
    } else if (char === '"') quote = true;
    else if ('([{'.includes(char)) brackets.push(char);
    else if (')]}'.includes(char)) brackets.pop();
    else if (char === ',' && brackets.length === 0 && /^[ \t]*[^:,\r\n]+:[ \t]*/.test(rawParams.slice(i + 1))) boundaries.push(i + 1);
  }
  boundaries.forEach((boundary, index) => {
    const key = /^[ \t]*([^:,\r\n]+):[ \t]*/.exec(rawParams.slice(boundary));
    if (!key) return;
    const span = trimmedSpan(text, params.start + boundary + key[0].length, params.start + (boundaries[index + 1] !== undefined ? boundaries[index + 1] - 1 : rawParams.length));
    const value = text.slice(span.start, span.end);
    result.push({ id: `a1111:param:${index}`, label: key[1].trim(), value, kind: JSON_NUMBER.test(value) ? 'number' : 'text', group: 'sdMetadata.params', ...span });
  });
  return result;
}

function fields(keyword: string, text: string): EditableField[] {
  if (keyword.toLowerCase() === 'parameters') return a1111Fields(text);
  if (keyword.toLowerCase() === 'prompt' || keyword.toLowerCase() === 'comment' && /^\s*[{[]/.test(text)) return jsonFields(keyword, text);
  return [];
}

/** Unrecognized or malformed metadata remains available in the raw editor. */
export function getMetadataFields(keyword: string, text: string): MetadataField[] {
  try {
    return fields(keyword, text).map(({ id, label, value, kind, multiline, group }) => ({ id, label, value, kind, multiline, group }));
  } catch { return []; }
}

export function updateMetadataField(keyword: string, text: string, id: string, value: string): string {
  const field = fields(keyword, text).find(candidate => candidate.id === id);
  if (!field) throw invalidField();
  if (value === field.value) return text;
  let replacement = value;
  if (field.kind === 'number') {
    replacement = value.trim();
    if (!JSON_NUMBER.test(replacement)) throw new Error('sdMetadata.invalidNumber');
  } else if (field.kind === 'boolean') {
    if (value !== 'true' && value !== 'false') throw invalidField();
  } else if (field.encoding === 'json') {
    replacement = JSON.stringify(value);
  } else if (field.encoding === 'lora' && value !== '' && !/^<(?:lora|lyco):[^<>\r\n]+>$/i.test(value)) {
    throw invalidField();
  }
  if (id === 'a1111:params' && value && !/^Steps:[ \t]*/.test(value)) throw invalidField();
  if (field.insert && value) replacement = field.insert + replacement + (id === 'a1111:negative' && field.start < text.length ? (text.includes('\r\n') ? '\r\n' : '\n') : '');
  if (field.suffix && value) replacement += field.suffix;
  return text.slice(0, field.start) + replacement + text.slice(field.end);
}

/** Original text distinguishes a NovelAI JSON Comment from an ordinary text Comment. */
export function validateMetadataText(keyword: string, text: string, originalText?: string): void {
  const key = keyword.toLowerCase();
  let originalJson = false;
  if (originalText !== undefined && key === 'comment') {
    try { parseJson(originalText); originalJson = true; } catch { /* Plain text comments are valid. */ }
  }
  if (key === 'prompt' || key === 'workflow' || key === 'comment' && (originalJson || /^\s*[{[]/.test(text))) parseJson(text);
}
