import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-tag-controls-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'render.cjs');
buildSync({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import i18n from 'i18next';
    import { initReactI18next } from 'react-i18next';
    import TagChipList from './src/components/TagChipList';
    import { TaggerCategoryGrid, ThresholdSliders } from './src/components/TaggerControls';
    i18n.use(initReactI18next).init({ lng: 'en', resources: {}, initImmediate: false });
    export const chips = props => renderToStaticMarkup(<TagChipList onChange={() => {}} {...props} />);
    export const categories = () => renderToStaticMarkup(<TaggerCategoryGrid enabled={new Set(['general'])} supported={['general', 'rating']} onChange={() => {}} />);
    export const thresholds = () => renderToStaticMarkup(<ThresholdSliders general={0.55} character={0.85} onGeneral={() => {}} onCharacter={() => {}} />);
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', loader: { '.css': 'empty' }, outfile,
});
const render = createRequire(import.meta.url)(outfile);

test('JSON chips use their field color for text, border, and background', () => {
  const html = render.chips({ values: ['long hair', 'blue eyes'], color: '#c084fc' });
  assert.equal((html.match(/background:#c084fc1a/g) || []).length, 2);
  assert.equal((html.match(/border-color:#c084fc40/g) || []).length, 2);
  assert.equal((html.match(/;color:#c084fc;/g) || []).length, 2);
  assert.match(html, /tag-chip-list-field/);
  assert.match(html, /class="tag-chip-remove"/);
  assert.match(html, /class="tag-chip-add"/);
});

test('TXT chips keep their tag palette and empty fields keep their add prompt', () => {
  const html = render.chips({ values: ['long hair', 'blue eyes'] });
  assert.doesNotMatch(html, /tag-chip-list-field/);
  assert.equal(new Set([...html.matchAll(/background:([^;]+)/g)].map(match => match[1])).size, 2);
  assert.match(render.chips({ values: [], color: '#f59e0b' }), /class="tag-chip-empty"[^>]*>jsonTag.noTagData/);
  assert.match(render.chips({ values: [] }), /class="tag-chip-empty"[^>]*>tagManager.noTagsClick/);
});

test('category controls expose selected and disabled styles without losing native inputs', () => {
  const html = render.categories();
  assert.equal((html.match(/class="tagger-category"/g) || []).length, 9);
  assert.match(html, /data-checked="true" data-disabled="false"/);
  assert.equal((html.match(/data-disabled="true"/g) || []).length, 7);
  assert.equal((html.match(/type="checkbox"/g) || []).length, 9);
});

test('threshold readouts keep the original size, weight, and monospace font', () => {
  const html = render.thresholds();
  assert.equal((html.match(/font-family:monospace;font-size:12px;font-weight:700/g) || []).length, 2);
});
