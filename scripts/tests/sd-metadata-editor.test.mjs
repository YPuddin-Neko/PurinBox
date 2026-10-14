import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-sd-metadata-editor-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'editor.cjs');
buildSync({ entryPoints: [join(root, 'src/utils/sdMetadataEditor.ts')], bundle: true, platform: 'node', format: 'cjs', outfile });
const { getMetadataFields, updateMetadataField, validateMetadataText } = createRequire(import.meta.url)(outfile);

const find = (keyword, source, label, group) => {
  const field = getMetadataFields(keyword, source).find(field => field.label === label && (group === undefined || field.group === group));
  assert.ok(field, `field ${label} in ${group ?? 'any group'}`);
  return field;
};
const edit = (keyword, source, label, value, group) => updateMetadataField(keyword, source, find(keyword, source, label, group).id, value);

test('A1111 edits prompt, negative prompt and complete parameters without changing surrounding bytes', () => {
  const source = '  girl <lora:style:0.8>, moon\r\nsecond line\r\nNegative prompt:  bad hands\r\nSteps: 28, Sampler: Euler a, Seed: 18446744073709551615, Extension: "a, b: c"  \r\n';
  assert.equal(find('parameters', source, 'sdMetadata.positive').value, 'girl <lora:style:0.8>, moon\r\nsecond line');
  assert.equal(edit('parameters', source, 'sdMetadata.positive', 'new prompt'), source.replace('girl <lora:style:0.8>, moon\r\nsecond line', 'new prompt'));
  assert.equal(edit('parameters', source, 'sdMetadata.negative', 'low quality'), source.replace('bad hands', 'low quality'));
  assert.equal(edit('parameters', source, 'sdMetadata.params', 'Steps: 30, Seed: 18446744073709551614'), source.replace('Steps: 28, Sampler: Euler a, Seed: 18446744073709551615, Extension: "a, b: c"', 'Steps: 30, Seed: 18446744073709551614'));
  assert.equal(edit('parameters', source, 'Seed', '18446744073709551614'), source.replace('18446744073709551615', '18446744073709551614'));
  assert.equal(find('parameters', source, 'Extension').value, '"a, b: c"');
});

test('A1111 LoRA fields replace or remove exactly their original occurrence', () => {
  const source = 'before <lora:style:0.8>, middle <lora:style:0.8> after\nNegative prompt: <lyco:bad:1> blur\nSteps: 20';
  const loras = getMetadataFields('parameters', source).filter(field => field.label === 'sdMetadata.lora');
  assert.equal(loras.length, 3);
  assert.equal(updateMetadataField('parameters', source, loras[1].id, '<lora:other:0.6>'), source.replace('middle <lora:style:0.8>', 'middle <lora:other:0.6>'));
  assert.equal(updateMetadataField('parameters', source, loras[2].id, ''), source.replace('<lyco:bad:1>', ''));
  assert.throws(() => updateMetadataField('parameters', source, loras[0].id, 'not a tag'), /sdMetadata.invalidField/);
});

test('A1111 missing negative and sampler sections can be added without consuming the prompt', () => {
  const source = 'portrait\r\nSteps: 20, Seed: 42';
  assert.equal(edit('parameters', source, 'sdMetadata.negative', 'blur'), 'portrait\r\nNegative prompt: blur\r\nSteps: 20, Seed: 42');
  assert.equal(edit('parameters', 'portrait', 'sdMetadata.negative', 'blur'), 'portrait\nNegative prompt: blur');
  assert.equal(edit('parameters', 'portrait', 'sdMetadata.params', 'Steps: 20, Seed: 42'), 'portrait\nSteps: 20, Seed: 42');
  assert.equal(edit('parameters', 'portrait', 'sdMetadata.negative', ''), 'portrait');
  assert.equal(edit('parameters', 'Steps: 20', 'sdMetadata.positive', 'portrait'), 'portrait\nSteps: 20');
  assert.equal(edit('parameters', '\r\nSteps: 20', 'sdMetadata.positive', 'portrait'), 'portrait\r\nSteps: 20');
  assert.equal(edit('parameters', 'Negative prompt: \nSteps: 20', 'sdMetadata.positive', 'portrait'), 'portrait\nNegative prompt: \nSteps: 20');
  assert.equal(edit('parameters', 'portrait\nNegative prompt: \nSteps: 20', 'sdMetadata.negative', 'blur'), 'portrait\nNegative prompt: blur\nSteps: 20');
});

test('ComfyUI exposes literal inputs and preserves graph links, unknown metadata and 64-bit seeds byte for byte', () => {
  const source = '{\n "3": {"class_type":"KSampler", "inputs": {"seed":18446744073709551615,"steps":20,"cfg":7.00,"sampler_name":"euler", "positive":["6",0],"negative":["7",0],"model":["4",0],"custom_flag":true}},\n "6":{"inputs":{"text":"a \\u732b","clip":["4",1]},"class_type":"CLIPTextEncode","unknown":{"number":9007199254740993}}, "7":{"inputs":{"text":"bad"},"class_type":"CLIPTextEncode"}, "8":{"inputs":{"lora_name":"style.safetensors","strength_model":0.8,"strength_clip":1},"class_type":"LoraLoader"}\n}';
  const fields = getMetadataFields('prompt', source);
  assert.equal(fields.filter(field => field.label === 'text').length, 2);
  assert.ok(fields.some(field => field.label === 'lora_name'));
  assert.ok(fields.some(field => field.label === 'strength_model'));
  assert.ok(fields.every(field => !['positive', 'negative', 'model', 'clip', 'number'].includes(field.label)));
  assert.equal(edit('prompt', source, 'steps', '30'), source.replace('"steps":20', '"steps":30'));
  assert.equal(edit('prompt', source, 'seed', '18446744073709551614'), source.replace('18446744073709551615', '18446744073709551614'));
  assert.equal(edit('prompt', source, 'text', 'new "cat"\nline', '6 · CLIPTextEncode'), source.replace('"a \\u732b"', JSON.stringify('new "cat"\nline')));
  assert.equal(edit('prompt', source, 'custom_flag', 'false'), source.replace('"custom_flag":true', '"custom_flag":false'));
  const textField = find('prompt', source, 'text', '6 · CLIPTextEncode');
  assert.equal(updateMetadataField('prompt', source, textField.id, textField.value), source);
  const longer = edit('prompt', source, 'text', 'a considerably longer prompt', '6 · CLIPTextEncode');
  assert.equal(find('prompt', longer, 'text', '7 · CLIPTextEncode').id, find('prompt', source, 'text', '7 · CLIPTextEncode').id);
});

test('NovelAI V4 nested captions, characters and scalar settings preserve unknown fields and original numeric spelling', () => {
  const source = '{ "v4_prompt":{"caption":{"base_caption":"scenery","char_captions":[{"char_caption":"1girl","centers":[{"x":0.5,"y":0.50}]}]},"use_coords":false},"v4_negative_prompt":{"caption":{"base_caption":"bad"}},"uc":"old negative","seed":18446744073709551615,"sampler":"k_euler", "scale":6.500, "unknown":{"future":[null,true,9007199254740993]},"escaped":"\\u732b" }';
  assert.equal(edit('Comment', source, 'base_caption', 'mountain', 'v4_prompt.caption'), source.replace('"scenery"', '"mountain"'));
  assert.equal(edit('Comment', source, 'char_caption', '1boy', 'v4_prompt.caption.char_captions.[0]'), source.replace('"1girl"', '"1boy"'));
  assert.equal(edit('Comment', source, 'uc', ''), source.replace('"old negative"', '""'));
  assert.equal(edit('Comment', source, 'seed', '18446744073709551614'), source.replace('18446744073709551615', '18446744073709551614'));
  assert.equal(edit('Comment', source, 'use_coords', 'true'), source.replace('"use_coords":false', '"use_coords":true'));
  assert.equal(edit('Comment', source, 'escaped', '猫'), source);
});

test('duplicate JSON keys remain distinct and unchanged noncanonical number tokens retain all digits', () => {
  const source = '{"seed":9007199254740993,"seed":18446744073709551615,"scale":1.00e+2}';
  const seeds = getMetadataFields('Comment', source).filter(field => field.label === 'seed');
  assert.equal(seeds.length, 2);
  assert.notEqual(seeds[0].id, seeds[1].id);
  assert.equal(updateMetadataField('Comment', source, seeds[1].id, '42'), '{"seed":9007199254740993,"seed":42,"scale":1.00e+2}');
});

test('JSON raw validation rejects malformed JSON without coercing plain metadata into JSON', () => {
  for (const source of ['{"a":01}', '{"a":NaN}', '{"a":1,}', '{"a":true false}', '{"a":"\\q"}', '[1,]', '{"a":1} trailing', '{"a":"line\nline"}']) {
    assert.throws(() => validateMetadataText('prompt', source), /sdMetadata.invalidJson/, source);
    assert.deepEqual(getMetadataFields('prompt', source), []);
  }
  assert.doesNotThrow(() => validateMetadataText('parameters', 'prompt {not JSON}\nSteps: 20'));
  assert.doesNotThrow(() => validateMetadataText('Comment', 'ordinary comment'));
  assert.doesNotThrow(() => validateMetadataText('Comment', 'edited comment', 'ordinary comment'));
  assert.throws(() => validateMetadataText('Comment', 'broken', '{"seed":1}'), /sdMetadata.invalidJson/);
  assert.doesNotThrow(() => validateMetadataText('prompt', '{"seed":18446744073709551615,"float":1e9999}'));
});

test('field edits reject invalid numeric and boolean input, missing fields and malformed JSON', () => {
  const source = '{"1":{"class_type":"Sampler","inputs":{"seed":18446744073709551615,"enabled":true,"text":"hello"}}}';
  for (const value of ['', 'Infinity', 'NaN', '1 trailing', '01', '0x10', '1,2']) assert.throws(() => edit('prompt', source, 'seed', value), /sdMetadata.invalidNumber/);
  assert.throws(() => edit('prompt', source, 'enabled', '1'), /sdMetadata.invalidField/);
  assert.throws(() => updateMetadataField('prompt', source, 'json:[999]', 'bad'), /sdMetadata.invalidField/);
  assert.throws(() => updateMetadataField('prompt', '{broken', 'json:[]', 'bad'), /sdMetadata.invalidJson/);
  assert.throws(() => edit('parameters', 'prompt\nSteps: 20', 'Steps', 'many'), /sdMetadata.invalidNumber/);
  assert.throws(() => edit('parameters', 'prompt\nSteps: 20', 'sdMetadata.params', 'Sampler: Euler'), /sdMetadata.invalidField/);
});
