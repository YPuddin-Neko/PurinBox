import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-resolution-analysis-'));
after(() => rmSync(output, { recursive: true, force: true }));
const require = createRequire(import.meta.url);
const runtime = require.resolve('react/jsx-runtime');
const outfile = join(output, 'page.cjs');

await build({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import i18n from './src/i18n';
    import Page from './src/pages/ResolutionAnalyzePage';
    export const language = lng => i18n.changeLanguage(lng);
    export const render = () => renderToStaticMarkup(<Page />);
    export const markup = node => renderToStaticMarkup(node);
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', outfile,
  loader: { '.css': 'empty' }, logLevel: 'error',
  plugins: [{
    name: 'resolution-fixture',
    setup(build) {
      // Seed completed analysis state without changing the page's rendering or export handlers.
      build.onLoad({ filter: /\/ResolutionAnalyzePage\.tsx$/ }, ({ path }) => {
        let contents = readFileSync(path, 'utf8');
        for (const [from, to] of [
          ["useState('');\n  const [rareThreshold", "useState('/fixture/input');\n  const [rareThreshold"],
          ['useState<ResolutionAnalyzeResult | null>(null)', 'useState<ResolutionAnalyzeResult | null>(globalThis.resolutionFixture.result)'],
          ['[arTolerance, setArTolerance] = useState(5)', '[arTolerance, setArTolerance] = useState(globalThis.resolutionFixture.tolerance)'],
          ["[aggExportPath, setAggExportPath] = useState('')", "[aggExportPath, setAggExportPath] = useState('/fixture/output')"],
          ['[enableAggExport, setEnableAggExport] = useState(false)', '[enableAggExport, setEnableAggExport] = useState(true)'],
          ['useState<Record<number, string>>({})', 'useState<Record<number, string>>(globalThis.resolutionFixture.targets)'],
        ]) {
          assert.equal(contents.split(from).length, 2, `fixture initializer: ${from}`);
          contents = contents.replace(from, to);
        }
        return { contents, loader: 'tsx' };
      });
      build.onResolve({ filter: /^(react\/jsx-runtime|@tauri-apps\/api\/core|\.\.\/hooks\/useBatchTask)$/ }, args => {
        if (args.importer.endsWith('/ResolutionAnalyzePage.tsx')) return { path: args.path, namespace: 'fixture' };
      });
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path }) => {
        const contents = path === 'react/jsx-runtime' ? `
          import * as runtime from ${JSON.stringify(runtime)};
          export const Fragment = runtime.Fragment;
          const capture = factory => (type, props, key) => {
            const node = factory(type, props, key);
            if (type === 'div' && /^\\d+x\\d+-\\d+$/.test(String(key))) globalThis.resolutionFixture.cards.push(node);
            if (type === 'button' && props.className === 'btn btn-primary') globalThis.resolutionFixture.export = props.onClick;
            return node;
          };
          export const jsx = capture(runtime.jsx);
          export const jsxs = capture(runtime.jsxs);
        ` : path === '@tauri-apps/api/core' ? `
          export const invoke = async (command, args) => {
            globalThis.resolutionFixture.calls.push({ command, args });
            return 'exported';
          };
        ` : `
          export const useBatchTask = () => ({
            processing: false, run: ({ exec }) => exec(), logger: { appendLog() {} },
            buttonProps: { onCancelLog() {} },
            progressLogProps: { current: 0, total: 0, logs: [], isDone: false, hasError: false },
          });
        `;
        return { contents, resolveDir: root };
      });
    },
  }],
});

globalThis.localStorage = { getItem: () => null, setItem: () => {} };
const lib = require(outfile);

function fixture(dimensions, targets = {}, tolerance = 5) {
  const total = dimensions.reduce((sum, [, , count]) => sum + count, 0);
  globalThis.resolutionFixture = {
    targets, tolerance, cards: [], calls: [],
    result: {
      total_images: total, failed_count: 0, failed_files: [], distinct_count: dimensions.length,
      min_width: 1, max_width: 4096, min_height: 1, max_height: 4096,
      groups: dimensions.map(([width, height, count]) => ({
        width, height, count, percent: count / total * 100, aspect_label: '', is_rare: false, files: [],
      })),
    },
  };
  return globalThis.resolutionFixture;
}

const mixed = [
  [1000, 2000, 100], [1024, 2048, 98],
  [1000, 1000, 1], [1024, 1024, 1],
  [300, 2000, 2], [600, 2000, 1], [1400, 2000, 1],
  [2600, 2000, 1], [3400, 2000, 1], [4000, 2000, 1],
];

test('all groups use numbered cards and the same target control regardless of image count', async () => {
  const f = fixture(mixed, { 0: '1024x2048' });
  const html = lib.render();
  assert.match(html, /共 8 组/);
  assert.ok(!html.includes('独立组'));
  assert.equal(f.cards.length, 8);
  const cards = f.cards.map(lib.markup);
  for (const [i, card] of cards.entries()) {
    assert.ok(card.includes(`第 ${i + 1} 组`));
    assert.ok(card.includes('目标分辨率'));
    assert.match(card, /class="cs-root [^"]*cs-compact/);
  }
  await f.export();
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].command, 'export_resolution_aggregation');
  const { plan } = f.calls[0].args.options;
  assert.equal(plan.length, 8);
  assert.equal(plan[0].folder, '1024x2048');
  assert.deepEqual(plan.flatMap(entry => entry.resolutions), mixed.map(([w, h]) => [w, h]));
  for (let i = 0; i < 6; i++) {
    const [w, h] = mixed[i + 4];
    assert.equal(plan[i + 2].folder, `${w}x${h}`);
    assert.ok(cards[i + 2].includes(`${w}×${h}`));
  }
});

test('single-resolution groups keep original dimensions by default, even with multiple images', async () => {
  const f = fixture([[797, 1200, 3], [1920, 1080, 2]]);
  assert.match(lib.render(), /共 2 组/);
  assert.equal(f.cards.length, 2);
  await f.export();
  assert.deepEqual(f.calls[0].args.options.plan.map(entry => entry.folder), ['797x1200', '1920x1080']);
});

test('a one-image group can select and export the recommended target like any other group', async () => {
  const f = fixture([[797, 1200, 1]], { 0: '768x1216' });
  const html = lib.render();
  assert.match(html, /第 1 组/);
  assert.match(html, /共 1 张/);
  assert.match(html, /推荐 768×1216（计算值）/);
  await f.export();
  assert.deepEqual(f.calls[0].args.options.plan, [{ folder: '768x1216', resolutions: [[797, 1200]] }]);
});

test('raising and lowering tolerance regroups every resolution without losing images', async () => {
  const dimensions = [[1000, 2000, 30], [1100, 2000, 5]];
  for (const [tolerance, groupCount] of [[5, 2], [12, 1], [5, 2]]) {
    const f = fixture(dimensions, {}, tolerance);
    assert.ok(lib.render().includes(`共 ${groupCount} 组`));
    assert.equal(f.cards.length, groupCount);
    await f.export();
    const plan = f.calls[0].args.options.plan;
    assert.equal(plan.length, groupCount);
    assert.deepEqual(plan.flatMap(entry => entry.resolutions), [[1000, 2000], [1100, 2000]]);
    const cards = f.cards.map(lib.markup);
    if (groupCount === 1) assert.ok(cards[0].includes('共 35 张'));
    else {
      assert.ok(cards[0].includes('共 30 张'));
      assert.ok(cards[1].includes('共 5 张'));
    }
  }
});

test('target selections keep the displayed group index when single- and multi-resolution groups are interleaved', async () => {
  const f = fixture([[300, 2000, 50], [1000, 2000, 30], [1100, 2000, 20]], { 1: '1100x2000' }, 12);
  lib.render();
  assert.equal(f.cards.length, 2);
  assert.ok(lib.markup(f.cards[0]).includes('300×2000'));
  assert.ok(lib.markup(f.cards[1]).includes('1100×2000（20 张）'));
  await f.export();
  assert.deepEqual(f.calls[0].args.options.plan, [
    { folder: '300x2000', resolutions: [[300, 2000]] },
    { folder: '1100x2000', resolutions: [[1000, 2000], [1100, 2000]] },
  ]);
});

test('empty analysis results do not show or export phantom groups', () => {
  const f = fixture([]);
  lib.render();
  assert.equal(f.cards.length, 0);
  assert.equal(f.export, undefined);
  assert.deepEqual(f.calls, []);
});

test('all-merged datasets retain target selection and render in all supported languages', async () => {
  try {
    for (const language of ['zh-CN', 'en', 'ja']) {
      await lib.language(language);
      const f = fixture(mixed.slice(0, 4), { 1: '1000x1000' });
      const html = lib.render();
      assert.equal(f.cards.length, 2);
      assert.ok(!html.includes('resolutionAnalyze.'));
      await f.export();
      assert.equal(f.calls[0].args.options.plan[1].folder, '1000x1000');
    }
  } finally {
    await lib.language('zh-CN');
  }
});
