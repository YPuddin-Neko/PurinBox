import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-hybrid-presets-'));
after(() => rmSync(output, { recursive: true, force: true }));
const require = createRequire(import.meta.url);
const runtime = require.resolve('react/jsx-runtime');
const outfile = join(output, 'presets.cjs');

await build({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import './src/i18n';
    import HybridTaggerTab from './src/components/HybridTaggerTab';
    export * from './src/utils/hybridPresets';
    export const render = () => renderToStaticMarkup(<HybridTaggerTab />);
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', outfile,
  loader: { '.css': 'empty' }, logLevel: 'error',
  plugins: [{
    name: 'hybrid-fixture',
    setup(build) {
      build.onLoad({ filter: /\/HybridTaggerTab\.tsx$/ }, ({ path }) => {
        const source = readFileSync(path, 'utf8');
        const initializer = "const [inputPath, setInputPath] = useState('');";
        assert.equal(source.split(initializer).length, 2);
        const nameInitializer = "const [newPresetName, setNewPresetName] = useState('');";
        assert.equal(source.split(nameInitializer).length, 2);
        return { contents: source
          .replace(initializer, "const [inputPath, setInputPath] = useState('/fixture/input');")
          .replace(nameInitializer, "const [newPresetName, setNewPresetName] = useState(globalThis.hybridFixture.name ?? '');"), loader: 'tsx' };
      });
      build.onResolve({ filter: /^(react\/jsx-runtime|@tauri-apps\/api\/core|\.\.\/hooks\/useLlmApiConfig)$/ }, args => {
        if (args.importer.endsWith('/HybridTaggerTab.tsx')) return { path: args.path, namespace: 'fixture' };
      });
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path }) => ({
        contents: path === 'react/jsx-runtime' ? `
          import * as runtime from ${JSON.stringify(runtime)};
          export const Fragment = runtime.Fragment;
          const capture = factory => (type, props, key) => {
            if (props?.onStart) globalThis.hybridFixture.start = props.onStart;
            if (type === 'button' && props.className === 'btn btn-primary btn-sm') globalThis.hybridFixture.save = props.onClick;
            return factory(type, props, key);
          };
          export const jsx = capture(runtime.jsx);
          export const jsxs = capture(runtime.jsxs);
        ` : path === '@tauri-apps/api/core' ? `
          export const invoke = async (command, args) => {
            globalThis.hybridFixture.calls.push({ command, args });
            return { success_count: 1, fail_count: 0, warning_count: 0 };
          };
        ` : `
          export const useLlmApiConfig = () => ({ ready: true, endpoint: 'http://fixture.invalid/v1', apiKey: '', modelName: 'fixture-vlm', preset: 'custom', customEndpoint: 'http://fixture.invalid/v1', modelList: [] });
        `,
        resolveDir: root,
      }));
    },
  }],
});
Object.defineProperty(globalThis, 'localStorage', {
  value: { getItem: () => null, setItem: () => {} }, configurable: true, writable: true,
});
const lib = require(outfile);
const base = {
  prompt: 'Original prompt', triggerWord: 'original', shortReplyThreshold: 100,
  sampling: { temperature: 0.3, topP: 0, imageSize: 1024, imageDetail: '', concurrency: 1, intervalSec: -1 },
  outputFormat: 'txt', skipExisting: false,
};
const configured = {
  prompt: 'Return NL: description', triggerWord: 'saved', shortReplyThreshold: 37,
  sampling: { temperature: 1.25, topP: 0.65, imageSize: 2048, imageDetail: 'original', concurrency: 4, intervalSec: 3 },
  outputFormat: 'json_simplified', skipExisting: true,
};

test('named presets round-trip every VLM field without local-model or API settings', () => {
  const list = lib.saveHybridPreset([], 'Complete', 'custom-1', { ...configured, apiKey: 'do-not-copy', modelId: 'local' }, { nlOnly: true });
  const restored = lib.parseHybridPresets(JSON.stringify(list));
  assert.deepEqual(lib.applyHybridPreset(restored[0], base, true), configured);
  assert.equal(restored[0].nlOnly, true);
  assert.equal(restored[0].apiKey, undefined);
  assert.equal(restored[0].modelId, undefined);
  assert.notEqual(list[0].sampling, configured.sampling);
});

test('same-name overwrite retains identity and updates all fields, including false and empty', () => {
  const list = lib.saveHybridPreset([], 'Same', 'first', configured, { nlOnly: true });
  const next = { ...base, triggerWord: '', shortReplyThreshold: 1 };
  const overwritten = lib.saveHybridPreset(list, 'Same', 'unused', next, { captionMode: true });
  assert.equal(overwritten.length, 1);
  assert.equal(overwritten[0].id, 'first');
  assert.deepEqual(lib.applyHybridPreset(overwritten[0], configured, true), next);
  assert.equal(overwritten[0].nlOnly, false);
  assert.equal(overwritten[0].captionMode, true);
});

test('old presets keep unspecified parameters and infer only their required output family', () => {
  const old = { id: 'old', name: 'Old', prompt: 'Legacy prompt', nlOnly: true };
  const applied = lib.applyHybridPreset(old, base, true);
  assert.deepEqual(applied, { ...base, prompt: old.prompt, outputFormat: 'json' });
  for (const format of ['json', 'json_simplified']) {
    assert.equal(lib.applyHybridPreset(old, { ...base, outputFormat: format }, true).outputFormat, format);
  }
  assert.equal(lib.applyHybridPreset({ ...old, nlOnly: false }, base, true).outputFormat, 'txt');
});

test('skip-existing never overrides the local reuse prerequisite', () => {
  const [preset] = lib.saveHybridPreset([], 'Skip', 'skip', configured, {});
  assert.equal(lib.applyHybridPreset(preset, base, true).skipExisting, true);
  assert.equal(lib.applyHybridPreset(preset, base, false).skipExisting, false);
});

test('all specialized modes enforce their compatible formats for built-in and custom presets', () => {
  for (const [id, mode, expected] of [
    ['builtin_nl', { nlOnly: true }, 'json'], ['builtin_sort', { preserveTags: true }, 'json'],
    ['builtin_caption', { captionMode: true }, 'txt'],
  ]) {
    for (const format of ['txt', 'json', 'json_simplified']) {
      const builtin = { ...lib.hybridBuiltinPreset(id, format), name: id };
      const [custom] = lib.saveHybridPreset([], id, 'custom', { ...base, outputFormat: format }, mode);
      const compatible = expected === 'json' && format !== 'txt' ? format : expected;
      assert.equal(lib.applyHybridPreset(builtin, { ...base, outputFormat: format }, true).outputFormat, compatible);
      assert.equal(lib.applyHybridPreset(custom, base, true).outputFormat, compatible);
    }
  }
});

test('restart restores built-in/custom selection and unsaved prompt edits, including empty text', () => {
  const [custom] = lib.saveHybridPreset([], 'NL', 'custom', configured, { nlOnly: true });
  for (const presetId of ['builtin_nl', 'builtin_sort', 'custom']) {
    for (const prompt of ['Edited but not saved as a preset', '']) {
      const saved = JSON.parse(JSON.stringify({ presetId, prompt, outputFormat: 'json_simplified' }));
      assert.deepEqual(lib.restoreHybridSelection(saved, [custom]), { ...saved });
    }
  }
  assert.equal(lib.restoreHybridSelection({ presetId: 'custom', outputFormat: 'txt' }, [custom]).outputFormat, 'json');
  assert.equal(lib.restoreHybridSelection({ presetId: 'builtin_caption', outputFormat: 'json' }, []).outputFormat, 'txt');
});

test('deleted presets and invalid storage fall back to a matching default without stale mode prompts', () => {
  for (const raw of [null, 'bad JSON', '{}', '[null, {}, {"id":"x","name":"X"}]']) {
    assert.deepEqual(lib.parseHybridPresets(raw), []);
  }
  for (const outputFormat of ['txt', 'json', 'json_simplified']) {
    assert.deepEqual(lib.restoreHybridSelection({ presetId: 'deleted', prompt: 'NL-only stale prompt', outputFormat }, []), {
      presetId: 'builtin_full', outputFormat, prompt: lib.defaultHybridPrompt(outputFormat),
    });
    assert.equal(lib.restoreHybridSelection({ outputFormat }, []).prompt, lib.defaultHybridPrompt(outputFormat));
  }
});

test('preset numeric limits follow the controls and invalid values preserve the current values', () => {
  const bad = { id: 'bad', name: 'Bad', prompt: '', shortReplyThreshold: 10000,
    sampling: { temperature: 20, topP: -1, imageSize: 99999, concurrency: 100, intervalSec: -20, imageDetail: 'invalid' } };
  const applied = lib.applyHybridPreset(bad, base, true);
  assert.equal(applied.shortReplyThreshold, 500);
  assert.deepEqual(applied.sampling, { temperature: 2, topP: 0, imageSize: 4096, concurrency: 16, intervalSec: -1, imageDetail: '' });
  assert.equal(lib.applyHybridPreset({ ...bad, shortReplyThreshold: 'bad', sampling: {} }, base, true).shortReplyThreshold, 100);
  assert.deepEqual(lib.applyHybridPreset({ ...bad, sampling: {} }, base, true).sampling, base.sampling);
});

test('the actual save handler writes all settings and preserves existing data when storage rejects the write', () => {
  const oldStorage = globalThis.localStorage;
  try {
    const presets = lib.saveHybridPreset([], 'Existing', 'existing', base, {});
    const store = new Map([
      ['hybrid_tagger_settings_v1', JSON.stringify({ presetId: 'builtin_nl', ...configured, ...configured.sampling })],
      ['hybrid_prompt_presets', JSON.stringify(presets)], ['hybrid_trigger_word', configured.triggerWord],
    ]);
    let failWrite = false;
    globalThis.localStorage = {
      getItem: key => store.get(key) ?? null,
      setItem: (key, value) => { if (failWrite) throw new Error('Storage quota exceeded'); store.set(key, value); },
    };
    globalThis.hybridFixture = { calls: [], name: 'Existing' };
    lib.render();
    globalThis.hybridFixture.save();
    const [preset] = lib.parseHybridPresets(store.get('hybrid_prompt_presets'));
    assert.equal(preset.id, 'existing');
    assert.equal(preset.nlOnly, true);
    // No backend calls or inferred values: these are the values read by the component's own handler.
    assert.deepEqual(lib.applyHybridPreset(preset, base, true), configured);
    const beforeFailure = store.get('hybrid_prompt_presets');
    failWrite = true;
    globalThis.hybridFixture = { calls: [], name: 'Must not appear' };
    lib.render();
    assert.doesNotThrow(() => globalThis.hybridFixture.save());
    assert.equal(store.get('hybrid_prompt_presets'), beforeFailure);
  } finally {
    globalThis.localStorage = oldStorage;
    delete globalThis.hybridFixture;
  }
});

test('restored presets produce the actual expected invoke options for every output and mode', async () => {
  const oldStorage = globalThis.localStorage;
  const oldWindow = globalThis.window;
  try {
    globalThis.window = { dispatchEvent() {} };
    for (const [mode, formats] of [
      [{}, ['txt', 'json', 'json_simplified']],
      [{ nlOnly: true }, ['json', 'json_simplified']],
      [{ preserveTags: true }, ['json', 'json_simplified']],
      [{ captionMode: true }, ['txt']],
    ]) {
      for (const outputFormat of formats) {
        for (const preferExisting of [false, true]) {
          const presets = lib.saveHybridPreset([], 'Saved mode', 'custom', { ...configured, outputFormat }, mode);
          const applied = lib.applyHybridPreset(presets[0], base, preferExisting);
          const saved = { modelId: 'fixture-local', presetId: 'custom', prompt: applied.prompt,
            outputFormat: applied.outputFormat, preferExisting, skipExisting: applied.skipExisting,
            shortReplyThreshold: String(applied.shortReplyThreshold), ...applied.sampling };
          const store = new Map([
            ['hybrid_tagger_settings_v1', JSON.stringify(saved)],
            ['hybrid_prompt_presets', JSON.stringify(presets)], ['hybrid_trigger_word', applied.triggerWord],
          ]);
          globalThis.localStorage = { getItem: key => store.get(key) ?? null, setItem: (k, v) => store.set(k, v) };
          globalThis.hybridFixture = { calls: [] };
          const html = lib.render();
          assert.ok(html.includes('Saved mode'));
          await globalThis.hybridFixture.start();
          const calls = globalThis.hybridFixture.calls;
          const tagging = calls.find(c => c.command === 'start_tagging').args.options;
          const refining = calls.find(c => c.command === 'start_tag_refining').args.options;
          assert.equal(tagging.output_format, outputFormat === 'txt' ? 'txt' : 'json');
          assert.equal(tagging.json_simplified, outputFormat === 'json_simplified');
          assert.equal(refining.file_format, tagging.output_format);
          assert.equal(refining.nl_only, !!mode.nlOnly);
          assert.equal(refining.preserve_tags, !!mode.preserveTags);
          assert.equal(refining.caption_mode, !!mode.captionMode);
          assert.equal(refining.skip_existing_labels, preferExisting);
          assert.equal(refining.trigger_word, configured.triggerWord);
          assert.equal(refining.short_reply_threshold, configured.shortReplyThreshold);
          assert.equal(refining.temperature, configured.sampling.temperature);
          assert.equal(refining.top_p, configured.sampling.topP);
          assert.equal(refining.image_size, configured.sampling.imageSize);
          assert.equal(refining.image_detail, configured.sampling.imageDetail);
          assert.equal(refining.concurrency, configured.sampling.concurrency);
          assert.equal(refining.request_interval_ms, configured.sampling.intervalSec * 1000);
        }
      }
    }
  } finally {
    globalThis.localStorage = oldStorage;
    globalThis.window = oldWindow;
    delete globalThis.hybridFixture;
  }
});
