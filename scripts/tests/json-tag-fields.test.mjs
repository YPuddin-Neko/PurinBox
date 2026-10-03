import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-json-tags-'));
after(() => rmSync(output, { recursive: true, force: true }));
execFileSync(process.execPath, [join(root, 'node_modules/typescript/bin/tsc'),
  'src/utils/jsonTagFields.ts', '--outDir', output, '--target', 'ES2020',
  '--module', 'commonjs', '--moduleResolution', 'node', '--skipLibCheck'], { cwd: root });
const { JSON_FIELDS, moveJsonTag, jsonTagPreview } = createRequire(import.meta.url)(join(output, 'jsonTagFields.js'));
const field = key => JSON_FIELDS.find(item => item.key === key);
const fixture = () => ({
  fixed: { quality: 'masterpiece, best quality', series: '', artist: 'artist name', extension: 'keep' },
  character: { name: 'alice', variant: '' },
  from_path: { appearance: ['long hair'] },
  ai_output: { count: '1girl', appearance: ['blue eyes'], tags: ['smile', 'outdoors'], environment: [], nl: 'Keep this caption.' },
  extension: { unchanged: true },
});
const move = (data, from, index, to, insertion, simplified = false) => moveJsonTag(data,
  { field: from, index, value: field(from).get(data)[index] }, { field: to, index: insertion }, simplified);

test('moves tags between every pair of fields without mutating the source', () => {
  for (const from of JSON_FIELDS) for (const to of JSON_FIELDS) {
    if (from === to) continue;
    let data = from.set(fixture(), ['move me', 'keep me']);
    data = to.set(data, ['first', 'last']);
    const original = structuredClone(data);
    const next = move(data, from.key, 0, to.key, 1);
    assert.deepEqual(from.get(next), ['keep me'], `${from.key} -> ${to.key}`);
    assert.deepEqual(to.get(next), ['first', 'move me', 'last']);
    assert.deepEqual(data, original);
    assert.equal(next.ai_output.nl, original.ai_output.nl);
    assert.deepEqual(next.extension, original.extension);
    assert.equal(next.fixed.extension, 'keep');
    assert.equal(typeof next[to.section][to.name], to.kind === 'csv' ? 'string' : 'object');
  }
});

test('supports empty destinations and leaves an empty source in its original format', () => {
  for (const from of JSON_FIELDS) for (const to of JSON_FIELDS) {
    if (from === to) continue;
    const data = to.set(from.set(fixture(), ['move me']), []);
    const next = move(data, from.key, 0, to.key, 0);
    assert.deepEqual(from.get(next), []);
    assert.deepEqual(to.get(next), ['move me']);
    assert.deepEqual(next[from.section][from.name], from.kind === 'csv' ? '' : []);
  }
});

test('reorders within a field at the beginning, middle, and end', () => {
  const data = field('ai_output.tags').set(fixture(), ['a', 'b', 'c', 'd']);
  assert.deepEqual(move(data, 'ai_output.tags', 0, 'ai_output.tags', 4).ai_output.tags, ['b', 'c', 'd', 'a']);
  assert.deepEqual(move(data, 'ai_output.tags', 3, 'ai_output.tags', 0).ai_output.tags, ['d', 'a', 'b', 'c']);
  assert.deepEqual(move(data, 'ai_output.tags', 0, 'ai_output.tags', 2).ai_output.tags, ['b', 'a', 'c', 'd']);
  assert.equal(move(data, 'ai_output.tags', 1, 'ai_output.tags', 1), data);
  assert.equal(move(data, 'ai_output.tags', 1, 'ai_output.tags', 2), data);
});

test('does not add duplicates to the destination or normalize the moved text', () => {
  const data = field('ai_output.tags').set(fixture(), ['BLUE_EYES', 'other']);
  const duplicate = field('fixed.quality').set(data, ['blue_eyes', 'first']);
  const next = move(duplicate, 'ai_output.tags', 0, 'fixed.quality', 0);
  assert.deepEqual(next.ai_output.tags, ['other']);
  assert.equal(next.fixed.quality, 'blue_eyes, first');
  assert.equal(move(data, 'ai_output.tags', 0, 'fixed.series', 0).fixed.series, 'BLUE_EYES');
});

test('only removes the dragged occurrence from a source containing duplicates', () => {
  const data = field('ai_output.tags').set(fixture(), ['smile', 'smile', 'outdoors']);
  assert.deepEqual(move(data, 'ai_output.tags', 1, 'fixed.series', 0).ai_output.tags, ['smile', 'outdoors']);
});

test('rejects stale, invalid, and hidden-field drops', () => {
  const data = fixture();
  const source = { field: 'ai_output.tags', index: 0, value: 'smile' };
  const target = { field: 'fixed.quality', index: 0 };
  for (const invalid of [{ ...source, value: 'changed' }, { ...source, index: -1 }, { ...source, index: 20 }, { ...source, field: 'unknown' }]) {
    assert.equal(moveJsonTag(data, invalid, target, false), data);
  }
  for (const invalid of [{ ...target, field: 'ai_output.nl' }, { ...target, index: -1 }, { ...target, index: 3 }, { ...target, index: 0.5 }]) {
    assert.equal(moveJsonTag(data, source, invalid, false), data);
  }
  assert.equal(move(data, 'ai_output.tags', 0, 'character.variant', 0, true), data);
  assert.equal(move(data, 'from_path.appearance', 0, 'ai_output.tags', 0, true), data);
});

test('simplified and full serialization retain the move', () => {
  const data = move(fixture(), 'fixed.quality', 0, 'ai_output.environment', 0, true);
  const simple = jsonTagPreview(data, true);
  assert.equal(simple.quality, 'best quality');
  assert.deepEqual(simple.environment, ['masterpiece']);
  assert.equal(simple.nl, 'Keep this caption.');
  assert.deepEqual(jsonTagPreview(data, false).ai_output.environment, ['masterpiece']);
});
