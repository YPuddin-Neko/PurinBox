import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-processing-pages-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'pages.cjs');
const require = createRequire(import.meta.url);

// 服务端渲染不支持 portal：把 createPortal 换成原地渲染，弹窗内容就能出现在静态 HTML 里
const reactDomPath = require.resolve('react-dom', { paths: [root] });
const portalShim = {
  name: 'portal-shim',
  setup(build) {
    build.onResolve({ filter: /^react-dom$/ }, args => (
      args.importer.startsWith(join(root, 'src')) ? { path: 'react-dom-shim', namespace: 'shim' } : undefined
    ));
    build.onLoad({ filter: /.*/, namespace: 'shim' }, () => ({
      contents: `export * from ${JSON.stringify(reactDomPath)}; export const createPortal = children => children;`,
      resolveDir: root,
    }));
  },
};

await build({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import i18n from './src/i18n';
    import zhCN from './src/i18n/locales/zh-CN';
    import en from './src/i18n/locales/en';
    import ja from './src/i18n/locales/ja';
    import { Modal } from './src/components/Modal';
    import HashThresholdFields from './src/components/HashThresholdFields';
    import DeviceToggle from './src/components/ui/DeviceToggle';
    import ResultTable from './src/components/ui/ResultTable';
    import HybridTaggerTab from './src/components/HybridTaggerTab';
    import AestheticPage from './src/pages/AestheticPage';
    import AlphaConvertPage from './src/pages/AlphaConvertPage';
    import BatchRenamePage from './src/pages/BatchRenamePage';
    import BlurNoisePage from './src/pages/BlurNoisePage';
    import BucketPreviewPage from './src/pages/BucketPreviewPage';
    import CropPage from './src/pages/CropPage';
    import FileKeeperPage from './src/pages/FileKeeperPage';
    import FilterPage from './src/pages/FilterPage';
    import FlipPage from './src/pages/FlipPage';
    import FormatConvertPage from './src/pages/FormatConvertPage';
    import ImageClusterPage from './src/pages/ImageClusterPage';
    import ImageDedupPage from './src/pages/ImageDedupPage';
    import PersonCropPage from './src/pages/PersonCropPage';
    import PerspectivePage from './src/pages/PerspectivePage';
    import ResolutionAnalyzePage from './src/pages/ResolutionAnalyzePage';
    import ScalePage from './src/pages/ScalePage';
    import SdMetadataPage from './src/pages/SdMetadataPage';
    import UpscalePage from './src/pages/UpscalePage';
    export const locales = { 'zh-CN': zhCN, en, ja };
    export const setLanguage = lng => i18n.changeLanguage(lng);
    const render = node => renderToStaticMarkup(node);
    export const pages = {
      AestheticPage, AlphaConvertPage, BatchRenamePage, BlurNoisePage, BucketPreviewPage, CropPage, FileKeeperPage,
      FilterPage, FlipPage, FormatConvertPage, ImageClusterPage, ImageDedupPage, PersonCropPage, PerspectivePage,
      ResolutionAnalyzePage, ScalePage, SdMetadataPage, UpscalePage, HybridTaggerTab,
    };
    export const page = name => { const Page = pages[name]; return render(<Page />); };
    export const modal = props => render(<Modal open onClose={() => {}} title="T" {...props}><p>body</p></Modal>);
    export const thresholds = () => render(<HashThresholdFields dhash={10} onDhash={() => {}} phash={10} onPhash={() => {}} color={0.85} onColor={() => {}} />);
    export const device = props => render(<DeviceToggle onChange={() => {}} {...props} />);
    export const table = props => render(<ResultTable title="List" columns={[{ label: '#', width: 30 }, { label: 'Name' }, { label: 'Src', align: 'center', width: 70 }]} {...props}><tr><td>1</td></tr></ResultTable>);
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', outfile,
  loader: { '.css': 'empty' }, plugins: [portalShim], logLevel: 'error',
});

// i18n 初始化时读 localStorage
Object.defineProperty(globalThis, 'localStorage', {
  value: { getItem: () => null, setItem: () => {} }, configurable: true, writable: true,
});
const lib = require(outfile);

const count = (html, pattern) => (html.match(pattern) || []).length;

test('hybrid trigger word disables native autocomplete but retains the saved value', () => {
  const storage = globalThis.localStorage;
  try {
    globalThis.localStorage = {
      getItem: key => key === 'hybrid_trigger_word' ? 'saved_trigger' : null,
      setItem: () => {},
    };
    const html = lib.page('HybridTaggerTab');
    const input = html.match(/<input\b[^>]*\bid="hybrid-trigger-word"[^>]*>/)?.[0];
    assert.ok(input);
    assert.match(input, /autocomplete="off"/i);
    assert.match(input, /value="saved_trigger"/);
    assert.match(input, /class="form-input"/);
    assert.match(html, /<label\b[^>]*\bfor="hybrid-trigger-word"/);
    assert.doesNotMatch(input, /\b(?:readonly|disabled)=/);
  } finally {
    globalThis.localStorage = storage;
  }
});

test('every two-column processing page renders through ToolPageLayout', () => {
  const twoColumn = [
    'AestheticPage', 'AlphaConvertPage', 'BlurNoisePage', 'CropPage', 'FileKeeperPage', 'FilterPage', 'FlipPage',
    'FormatConvertPage', 'ImageClusterPage', 'PersonCropPage', 'PerspectivePage', 'ResolutionAnalyzePage', 'ScalePage', 'UpscalePage',
  ];
  for (const name of twoColumn) {
    const html = lib.page(name);
    assert.match(html, /^<div class="page"><div class="page-header">/, name);
    assert.equal(count(html, /class="ui-tool-grid"/g), 1, name);
    assert.equal(count(html, /class="ui-tool-col"/g), 2, name);
  }
});

test('filter action cards stack vertically and the option groups use the compact cards', () => {
  const filter = lib.page('FilterPage');
  assert.equal(count(filter, /class="ui-choice ui-choice-inline (is-selected )?is-column( is-danger)?"/g), 2);
  assert.equal(count(filter, /class="ui-choice ui-choice-inline (is-selected )?is-compact"/g), 4);
  assert.equal(count(lib.page('FileKeeperPage'), /class="ui-choice ui-choice-inline (is-selected )?is-compact"/g), 7);
  assert.equal(count(lib.page('FormatConvertPage'), /class="ui-choice ui-choice-inline (is-selected )?is-compact"/g), 5);
  // 其余页面的选项卡片保持常规尺寸
  assert.doesNotMatch(lib.page('FlipPage'), /is-compact/);
  assert.doesNotMatch(lib.page('ScalePage'), /is-compact/);
});

test('the rename-tags option names every sidecar the backend renames', () => {
  for (const [lng, dict] of Object.entries(lib.locales)) {
    for (const ext of ['.txt', '.json', '.caption']) {
      assert.ok(dict.batchRename.renameTags.includes(ext), `${lng} ${ext}`);
    }
  }
  lib.setLanguage('en');
  const html = lib.page('BatchRenamePage');
  assert.match(html, /Rename tag files too \(\.txt, \.json, \.caption\)/);
  // 页签栏是共用的 SegmentedTabs
  assert.match(html, /<div role="tablist" class="ui-seg"[^>]*><button type="button" role="tab" aria-selected="true" class="ui-seg-tab is-active">/);
  lib.setLanguage('zh-CN');
});

test('threshold sliders keep the in-flow spacing of the dedup panels', () => {
  const html = lib.thresholds();
  assert.equal(count(html, /<div class="form-group ui-range-flow" style="margin-bottom:12px">/g), 3);
  assert.equal(count(html, /style="color:#7c5cfc;font-size:13px;font-weight:700"/g), 3);
});

test('toggle buttons share one style: device, algorithm, feature and map theme', () => {
  const device = lib.device({ useGpu: true });
  assert.match(device, /class="ui-toggle ui-device-btn" aria-pressed="false" style="--toggle-color:#fbbf24"/);
  assert.match(device, /class="ui-toggle ui-device-btn" aria-pressed="true" style="--toggle-color:#4ade80"/);
  assert.match(lib.device({ useGpu: true, cpuDisabled: true }), /aria-pressed="false" disabled=""/);
  const cluster = lib.page('ImageClusterPage');
  // 2 个算法、3 个特征、2 个分布图主题、2 个设备
  assert.equal(count(cluster, /class="ui-toggle( ui-device-btn)?"/g), 9);
  assert.equal(count(cluster, /aria-pressed="true"/g), 4);
  assert.match(cluster, /--toggle-color:#60a5fa/);
  // 选中态的边框、底色、文字色都来自 .ui-toggle，按钮上不再写内联配色
  assert.doesNotMatch(cluster, /class="ui-toggle"[^>]*style="(?:[^"]*;)?(border|background|color):/);
});

test('ResultTable renders the shared header cells, scroll area and footer', () => {
  const html = lib.table({ extra: '9 items', footer: 'pager' });
  assert.match(html, /^<div class="ui-result-table"><div class="ui-result-table-head"><span class="ui-result-table-title">List<\/span>9 items<\/div>/);
  assert.match(html, /<div class="ui-result-table-scroll"><table class="ui-result-table-grid"><thead><tr>/);
  assert.match(html, /<th style="text-align:left;width:30px">#<\/th><th style="text-align:left">Name<\/th><th style="text-align:center;width:70px">Src<\/th>/);
  assert.match(html, /<tbody><tr><td>1<\/td><\/tr><\/tbody><\/table><\/div>pager<\/div>$/);
});

test('Modal merges dialog and title styles over the sectioned look', t => {
  // createPortal 的第二个参数在调用前求值，给一个占位的 document
  globalThis.document = { body: null };
  t.after(() => { delete globalThis.document; });
  const sectioned = lib.modal({ sectioned: true });
  assert.match(sectioned, /border-radius:16px/);
  assert.match(sectioned, /max-height:90vh/);
  assert.match(sectioned, /font-size:13px;font-weight:700/);
  // 元数据详情弹窗：标题 14/600、圆角 12、最大高度 85vh
  const detail = lib.modal({ sectioned: true, dialogStyle: { borderRadius: 12, maxHeight: '85vh' }, titleStyle: { fontSize: 14, fontWeight: 600 } });
  assert.match(detail, /border-radius:12px/);
  assert.match(detail, /max-height:85vh/);
  assert.doesNotMatch(detail, /max-height:90vh/);
  assert.match(detail, /font-size:14px;font-weight:600/);
  // 默认外观不变
  assert.match(lib.modal({}), /border-radius:12px;padding:20px 24px;min-width:320px;max-width:min\(480px, calc\(100vw - 32px\)\)/);
});

test('result pages use the shared panels, chips and path inputs', () => {
  const dedup = lib.page('ImageDedupPage');
  assert.equal(count(dedup, /<div class="ui-path-row is-sm" style="gap:6px">/g), 1);
  assert.doesNotMatch(dedup, /border-radius:var\(--radius-lg\);padding:20px/);
  const rename = lib.page('BatchRenamePage');
  // 序号重命名 1 个 + 查重重命名的 A、B 文件夹
  assert.equal(count(rename, /class="ui-path-row( is-sm)?"/g), 3);
  const metadata = lib.page('SdMetadataPage');
  assert.match(metadata, /<input id="[^"]+" class="form-input" placeholder="[^"]+" readOnly="" value=""\/>/);
  assert.equal(count(lib.page('BucketPreviewPage'), /class="ui-path-row"/g), 1);
  assert.equal(count(lib.page('ResolutionAnalyzePage'), /class="ui-path-row"/g), 1);
});
