import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-tag-editor-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'tag-editor.cjs');
buildSync({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import i18n from 'i18next';
    import { initReactI18next } from 'react-i18next';
    import TagChipList from './src/components/TagChipList';
    import ScopeToggle from './src/components/ScopeToggle';
    import LlmSamplingFields, { LLM_SAMPLING_DEFAULTS } from './src/components/LlmSamplingFields';
    import { ImagePreviewPane, ResizeHandle, TagStatsActions } from './src/components/TagEditorLayout';
    import TagStatsPanel from './src/components/TagStatsPanel';
    export * as tagText from './src/utils/tagText';
    export * as tagSave from './src/utils/tagSave';
    export * as hybrid from './src/utils/hybridSettings';
    export * as prompts from './src/utils/llmPrompts';
    export * as taggerOptions from './src/utils/taggerOptions';
    export { JSON_APPEND_FIELD_KEYS } from './src/api/commandOptions';
    export { flexRatioAfterDrag } from './src/hooks/useDragResize';
    export { default as zh } from './src/i18n/locales/zh-CN';
    export { default as en } from './src/i18n/locales/en';
    export { default as ja } from './src/i18n/locales/ja';
    i18n.use(initReactI18next).init({ lng: 'en', resources: {}, initImmediate: false });
    const noop = () => {};
    export const chips = props => renderToStaticMarkup(<TagChipList onChange={noop} {...props} />);
    export const scope = props => renderToStaticMarkup(<ScopeToggle value="all" onChange={noop} {...props} />);
    export const sampling = props => renderToStaticMarkup(<LlmSamplingFields value={LLM_SAMPLING_DEFAULTS} onChange={noop} {...props} />);
    export const preview = props => renderToStaticMarkup(<ImagePreviewPane index={-1} total={0} onPrev={noop} onNext={noop} {...props} />);
    export const handle = axis => renderToStaticMarkup(<ResizeHandle axis={axis} onMouseDown={noop} />);
    export const statsActions = props => renderToStaticMarkup(<TagStatsActions disabled={false} onError={noop}
      stats={{ tagStats: [], tagListMode: 'all', setTagListMode: noop }}
      translation={{ translations: {}, translating: false, translate: async () => {} }} {...props} />);
    export const statsPanel = (rows, props) => renderToStaticMarkup(<TagStatsPanel translations={{}} currentTags={new Set()} total={2}
      filteredCount={2} filterActive={false} onClearFilter={noop} progress={null} {...props} stats={{
        globalSearch: '', setGlobalSearch: noop, tagSortBy: 'freq', setTagSortBy: noop, tagSortDir: 'desc', setTagSortDir: noop,
        tagListMode: 'all', setTagListMode: noop, selectedTags: new Set(), setSelectedTags: noop, statsLimit: 300, setStatsLimit: noop,
        taggedCount: 2, tagStats: rows, filteredStats: rows, toggleTagSelect: noop,
      }} />);
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', loader: { '.css': 'empty' }, outfile,
});
const m = createRequire(import.meta.url)(outfile);
const { tagText, tagSave, hybrid, prompts } = m;

test('tag keys ignore case, underscores and extra spaces', () => {
  assert.equal(tagText.tagKey(' Long_Hair  '), 'long hair');
  assert.equal(tagText.tagKey('long  hair'), tagText.tagKey('LONG_HAIR'));
  assert.deepEqual(tagText.dedupeTags(['Smile', 'smile', ' solo ', '']).tags, ['Smile', 'solo']);
});

test('moving a tag within a list uses the insertion point before the move', () => {
  const tags = ['a', 'b', 'c', 'd'];
  assert.deepEqual(tagText.moveTagWithin(tags, 0, 4), ['b', 'c', 'd', 'a']);
  assert.deepEqual(tagText.moveTagWithin(tags, 3, 0), ['d', 'a', 'b', 'c']);
  assert.deepEqual(tagText.moveTagWithin(tags, 0, 2), ['b', 'a', 'c', 'd']);
  for (const [from, to] of [[1, 1], [1, 2], [-1, 0], [4, 0], [0, 5], [0.5, 1]]) assert.equal(tagText.moveTagWithin(tags, from, to), tags);
});

test('renaming a chip keeps the selected tag selected under its new name', () => {
  const selected = new Set(['long hair', 'smile']);
  assert.deepEqual([...tagText.renameSelected(selected, 'long hair', 'long_hair')], ['smile', 'long_hair']);
  assert.equal(tagText.renameSelected(selected, 'solo', '1girl'), selected);
});

test('new tags follow the dataset style and escaping is idempotent', () => {
  const escaped = tagText.toDanbooruEscaped(' Cat Ears (Animal) ');
  assert.equal(escaped, 'cat_ears_\\(animal\\)');
  assert.equal(tagText.toDanbooruEscaped(escaped), escaped);
  assert.equal(tagText.normalizeLikeTags('Long Hair', ['blue_eyes']), 'long_hair');
  assert.equal(tagText.normalizeLikeTags('Long_Hair', ['blue eyes']), 'long hair');
});

test('tag chips use the original 16 colours', () => {
  const colors = new Set(Array.from({ length: 2000 }, (_, i) => tagText.getTagChipColor(`tag ${i}`).tx));
  assert.equal(colors.size, 16);
  assert.deepEqual(tagText.getTagChipColor('long hair'), { bg: 'rgba(45,212,191,0.10)', bd: 'rgba(45,212,191,0.25)', tx: '#2dd4bf' });
  assert.equal(tagText.getTagChipColor('blue eyes').tx, '#ec4899');
  assert.equal(tagText.getTagChipColor('1girl').tx, '#14b8a6');
  assert.equal(tagText.getTagChipColor('solo').tx, '#22c55e');
});

test('the JSON stats list keeps its own 10 colours while TXT uses all 16', () => {
  const colors = new Set(Array.from({ length: 2000 }, (_, i) => tagText.getJsonStatColor(`tag ${i}`).tx));
  assert.equal(colors.size, 10);
  assert.equal(tagText.getJsonStatColor('long hair').tx, '#ec4899');
  assert.equal(tagText.getJsonStatColor('1girl').tx, '#84cc16');
  assert.equal(tagText.getJsonStatColor('solo').tx, '#c084fc');
  const rows = [['long hair', 2]];
  assert.match(m.statsPanel(rows), /title="long hair" style="[^"]*color:#2dd4bf/);
  assert.match(m.statsPanel(rows, { colorOf: tagText.getJsonStatColor }), /title="long hair" style="[^"]*color:#ec4899/);
});

test('save-all results only clear the items that were saved unchanged', () => {
  const tagsA = ['a'], tagsB = ['b'], tagsC = ['c'];
  const items = [
    { path: '/x/a.png', tags: tagsA, dirty: true },
    { path: '/x/b.png', tags: tagsB, dirty: true },
    { path: '/x/c.png', tags: ['c', 'edited'], dirty: true },
    { path: '/x/d.png', tags: ['d'], dirty: true },
  ];
  const sent = new Map([['/x/a.png', tagsA], ['/x/b.png', tagsB], ['/x/c.png', tagsC]]);
  const failures = tagSave.saveFailures({ saved: 2, failed: [{ path: '/x/b.png', error: '写入失败 /x/b.txt: denied' }] });
  const next = tagSave.settleSaved(items, sent, failures, item => item.tags);
  assert.deepEqual(next.map(item => item.dirty), [false, true, true, true]);
  assert.equal(tagSave.settleSaved(next, sent, failures, item => item.tags), next);
});

test('save failures are read defensively and listed by file name', () => {
  assert.deepEqual(tagSave.saveFailures(3), []);
  assert.deepEqual(tagSave.saveFailures(null), []);
  assert.deepEqual(tagSave.saveFailures({ failed: [{ path: 'C:\\\\data\\\\a.png', error: 'x' }, { error: 'no path' }] }), [{ path: 'C:\\\\data\\\\a.png', error: 'x' }]);
  const many = Array.from({ length: 12 }, (_, i) => ({ path: `/data/set/${i}.png`, error: 'denied' }));
  const t = (key, { n }) => `${key} ${n}`;
  const text = tagSave.saveFailureAlert(many, t);
  assert.equal(text.split('\n').length, 12);
  assert.match(text, /^tagEditor\.saveFailedCount 12\n0\.png: denied\n/);
  assert.match(text, /…$/);
  assert.equal(tagSave.saveFailureAlert([{ path: 'C:\\data\\a.png', error: 'e' }], t), 'tagEditor.saveFailedCount 1\na.png: e');
});

test('both editors share the translate and common/all buttons of the stats header', () => {
  const settings = { translate_enabled: 'true' };
  Object.defineProperty(globalThis, 'localStorage', {
    value: { getItem: key => settings[key] ?? null, setItem: () => {} }, configurable: true, writable: true,
  });
  try {
    const html = m.statsActions({});
    assert.match(html, /^<button class="btn btn-ghost btn-sm tag-icon-btn" title="tagEditor.translateTags">/);
    assert.match(html, /<button class="btn btn-ghost btn-sm tag-text-btn">tagEditor.commonLabel<\/button>$/);
    assert.match(m.statsActions({ stats: { tagStats: [], tagListMode: 'common', setTagListMode: () => {} } }), />tagEditor.allLabel</);
    // 没有图片或设置里没开翻译时禁用
    assert.equal((m.statsActions({ disabled: true }).match(/disabled=""/g) || []).length, 2);
    settings.translate_enabled = 'false';
    assert.equal((m.statsActions({}).match(/disabled=""/g) || []).length, 1);
    // 翻译中：两种模式都禁用翻译按钮，只有 JSON 模式把图标换成转圈
    const translating = { translations: { a: 'A' }, translating: true, translate: async () => {} };
    assert.match(m.statsActions({ translation: translating, spinner: true }), /animation:spin 1s linear infinite/);
    assert.doesNotMatch(m.statsActions({ translation: translating }), /animation:spin/);
    assert.match(m.statsActions({ translation: translating }), /style="color:#60a5fa"/);
  } finally {
    delete globalThis.localStorage;
  }
});

test('the preview divider follows the pointer pixel for pixel', () => {
  // 300:100 按下，向上拖 50 像素后上块应正好 250 像素
  const ratio = m.flexRatioAfterDrag(300, 100, -50, 0.5, 6);
  assert.equal(Math.round(400 * ratio / (ratio + 1)), 250);
  assert.equal(m.flexRatioAfterDrag(300, 100, 0, 0.5, 6), 3);
  assert.equal(m.flexRatioAfterDrag(300, 100, 60, 0.5, 6), 6);
  assert.equal(m.flexRatioAfterDrag(300, 100, 100, 0.5, 6), 6);
  assert.equal(m.flexRatioAfterDrag(100, 300, -90, 0.5, 6), 0.5);
});

test('skip-existing defaults on only for a fresh install', () => {
  assert.equal(hybrid.initialSkipExisting(null), true);
  assert.equal(hybrid.initialSkipExisting({}), true);
  assert.equal(hybrid.initialSkipExisting({ modelId: 'wd', preferExisting: true }), false);
  assert.equal(hybrid.initialSkipExisting({ preferExisting: true, skipExisting: true }), true);
  assert.equal(hybrid.initialSkipExisting({ preferExisting: false, skipExisting: true }), false);
  assert.equal(hybrid.storedNumber('0.3', 1), 0.3);
  assert.equal(hybrid.storedNumber(5, 1), 5);
  for (const bad of ['', 'abc', undefined, NaN, Infinity, null]) assert.equal(hybrid.storedNumber(bad, 7), 7);
});

test('saved concurrency and image size come back as whole numbers in range', () => {
  // 旧版存的是输入框原文
  assert.equal(hybrid.storedCount('2.5', 1, 16), 2);
  assert.equal(hybrid.storedCount('-1', 1, 16), 1);
  assert.equal(hybrid.storedCount('0', 1, 16), 1);
  assert.equal(hybrid.storedCount('20', 1, 16), 16);
  assert.equal(hybrid.storedCount(3.9, 1, 16), 3);
  assert.equal(hybrid.storedCount('1536', 1024, 0xffffffff), 1536);
  for (const bad of ['', 'abc', undefined, null, NaN, Infinity]) assert.equal(hybrid.storedCount(bad, 1024, 0xffffffff), 1024);
});

test('refine prompts share one template and keep their own wording', () => {
  for (const prompt of [prompts.TAG_REFINE_PROMPT, prompts.HYBRID_PROMPT_TXT, prompts.HYBRID_PROMPT_JSON]) {
    assert.equal(prompt.split('{tags}').length, 2);
    assert.match(prompt, /^You are an expert anime image tagger\./);
  }
  assert.match(prompts.TAG_REFINE_PROMPT, /\(lowercase, underscores\)/);
  assert.doesNotMatch(prompts.TAG_REFINE_PROMPT, /local tagger/);
  assert.match(prompts.HYBRID_PROMPT_TXT, /produced by a local tagger model\./);
  assert.match(prompts.HYBRID_PROMPT_TXT, /\(lowercase danbooru-style tags\)\n/);
  assert.match(prompts.HYBRID_PROMPT_JSON, /wrong subject count[\s\S]*6\. Sort every remaining tag[\s\S]*NL: <natural language description>$/);
  assert.match(prompts.TAG_SORT_PROMPT, /Tags to sort: \{tags\}/);
});

test('the trigger word fills its placeholder or drops the line', () => {
  assert.equal(prompts.applyTriggerWord('Start with: {trigger},\nend', ' mychar '), 'Start with: mychar,\nend');
  assert.equal(prompts.applyTriggerWord('Start with: {trigger},\nend', '  '), 'end');
  assert.match(prompts.applyTriggerWord(prompts.HYBRID_PROMPT_DETAILED_CAPTION, ''), /^(?![\s\S]*\{trigger\})/);
});

test('JSON append targets come from the command keys and use the JSON editor field names', () => {
  const fields = m.taggerOptions.JSON_APPEND_FIELDS;
  assert.deepEqual(fields.map(field => field.value), [...m.JSON_APPEND_FIELD_KEYS]);
  for (const locale of [m.zh, m.en, m.ja]) {
    for (const { labelKey } of fields) {
      const [ns, key] = labelKey.split('.');
      assert.equal(ns, 'jsonTag');
      assert.equal(typeof locale[ns][key], 'string', labelKey);
    }
  }
  assert.equal(m.zh.jsonTag.fieldTags, '动作/表情/构图');
});

test('the shared editor namespace has the same keys in every language', () => {
  const keys = locale => Object.keys(locale.tagEditor).sort();
  assert.deepEqual(keys(m.en), keys(m.zh));
  assert.deepEqual(keys(m.ja), keys(m.zh));
});

test('chip buttons are named for what they do and carry no misleading tooltip', () => {
  const html = m.chips({ values: ['long hair'] });
  assert.match(html, /class="tag-chip-remove" aria-label="tagEditor.removeTag: long hair"/);
  assert.match(html, /class="tag-chip-add" aria-label="tagEditor.addTag"/);
  assert.doesNotMatch(html, /title=/);
  assert.doesNotMatch(html, /outline:/);
});

test('scope chips look the same in both batch dialogs and disable a missing current image', () => {
  const html = m.scope({ hasCurrent: false, color: '#f87171' });
  assert.match(html, /<legend>tagEditor.applyScope<\/legend>/);
  assert.match(html, /--option-color:#f87171/);
  assert.equal((html.match(/class="tag-option"/g) || []).length, 2);
  assert.match(html, /<input type="radio"[^>]*disabled=""[^>]*\/>tagEditor.scopeCurrentImage/);
});

test('sampling fields keep each page layout', () => {
  const order = html => [...html.matchAll(/(llmApi\.(?:interval|concurrency|temperature|imageSize|imageDetail)|Top P|EXTRA)(?=[<\s])/g)].map(x => x[1]);
  assert.deepEqual(order(m.sampling({})), ['llmApi.interval', 'llmApi.concurrency', 'llmApi.temperature', 'Top P']);
  assert.deepEqual(order(m.sampling({ image: true, samplingFirst: true, extra: 'EXTRA' })),
    ['llmApi.temperature', 'Top P', 'llmApi.imageSize', 'EXTRA', 'llmApi.imageDetail', 'llmApi.interval', 'llmApi.concurrency']);
  assert.deepEqual(order(m.sampling({ layout: 'compact', image: true, extra: 'EXTRA' })),
    ['llmApi.temperature', 'Top P', 'llmApi.imageSize', 'llmApi.concurrency', 'llmApi.interval', 'llmApi.imageDetail', 'EXTRA']);
  const html = m.sampling({ maxConcurrency: 16 });
  assert.match(html, /type="number"[^>]*min="1" max="16"/);
  assert.match(html, /type="number"[^>]*min="-1" max="120"/);
});

test('sampling fields keep the interval tooltip and only use existing keys', () => {
  for (const layout of ['panel', 'compact']) {
    const html = m.sampling({ layout, image: true });
    assert.match(html, /title="llmApi\.intervalTip"/);
    assert.doesNotMatch(html, /placeholder=/);
    assert.match(m.sampling({ layout, image: true, imageSizePlaceholder: 'PH' }), /placeholder="PH"[^>]*min="256"|min="256"[^>]*placeholder="PH"/);
    const used = new Set([...html.matchAll(/llmApi\.([A-Za-z]+)/g)].map(x => x[1]));
    for (const locale of [m.zh, m.en, m.ja]) {
      for (const key of used) assert.equal(typeof locale.llmApi[key], 'string', `llmApi.${key}`);
    }
  }
});

test('the preview pane hides navigation until images are loaded', () => {
  assert.doesNotMatch(m.preview({}), /tag-preview-nav/);
  const html = m.preview({ total: 3, index: 0, path: '/x/a.png', filename: 'a.png' });
  assert.match(html, /tag-preview-nav/);
  assert.match(html, />1\/3</);
  assert.match(html, /class="tag-panel-note">a.png/);
  assert.match(m.handle('y'), /class="tag-resize-handle is-y"/);
});
