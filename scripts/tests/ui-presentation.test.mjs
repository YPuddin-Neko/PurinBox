import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import ts from 'typescript';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-ui-presentation-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'ui.cjs');
await build({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import i18n from 'i18next';
    import { initReactI18next } from 'react-i18next';
    import { ListPlus } from 'lucide-react';
    import { Modal } from './src/components/Modal';
    import ScopeToggle from './src/components/ScopeToggle';
    import LlmApiPanel from './src/components/LlmApiPanel';
    import ImageGridColumn from './src/components/ImageGridColumn';
    import NaturalLangTab from './src/components/NaturalLangTab';
    import en from './src/i18n/locales/en';
    import zh from './src/i18n/locales/zh-CN';
    import ja from './src/i18n/locales/ja';
    i18n.use(initReactI18next).init({ lng: 'en', resources: {
      en: { translation: en }, 'zh-CN': { translation: zh }, ja: { translation: ja },
    }, initImmediate: false });
    const noop = () => {};
    export const modal = props => renderToStaticMarkup(<Modal open onClose={noop} title="Batch" {...props}>Body</Modal>);
    export const sectioned = () => modal({ sectioned: true, width: 440, headerIcon: <ListPlus /> });
    export const scope = props => renderToStaticMarkup(<ScopeToggle value="all" onChange={noop} hasCurrent {...props} />);
    export const api = props => renderToStaticMarkup(<LlmApiPanel api={{
      preset: 'openai', endpoint: '', apiKey: '', modelName: '', modelList: [],
      setPreset: noop, setApiKey: noop, setModelName: noop, saveConfig: noop, fetchModels: noop,
    }} {...props} />);
    export const grid = () => renderToStaticMarkup(<ImageGridColumn width={220} items={[]} total={0} tagged={0}
      search="" onSearch={noop} filter="all" onFilter={noop} selected={-1} onSelect={noop} badge={noop} />);
    export const natural = () => renderToStaticMarkup(<NaturalLangTab images={[]} setImages={noop} onError={noop} />);
    export const hint = lng => i18n.getResource(lng, 'translation', 'naturalLang.clickToTranslate');
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', loader: { '.css': 'empty' }, outfile,
  plugins: [{
    name: 'server-portal',
    setup(builder) {
      builder.onResolve({ filter: /^react-dom$/ }, args => basename(args.importer) === 'Modal.tsx'
        ? { path: 'portal', namespace: 'fixture' } : undefined);
      builder.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: 'export const createPortal = children => children;' }));
    },
  }],
});
const ui = createRequire(import.meta.url)(outfile);
const renderModal = callback => {
  const previous = globalThis.document;
  globalThis.document = { body: {} };
  try { return callback(); }
  finally {
    if (previous === undefined) delete globalThis.document;
    else globalThis.document = previous;
  }
};

test('TXT batch dialogs retain their plain frame without changing shared default dialogs', () => {
  const plain = renderModal(() => ui.modal({ plain: true, width: 380 }));
  assert.match(plain, /role="dialog"[^>]*aria-modal="true"[^>]*padding:20px;width:380px;max-width:90vw/);
  assert.match(plain, /margin-bottom:14px/);
  assert.match(plain, /font-size:14px;font-weight:700/);
  assert.doesNotMatch(plain, /<svg|backdrop-filter|box-shadow|animation:|min-width:320px/);
  assert.doesNotMatch(plain, /<button/);
  assert.match(plain, /max-height:90vh/);
  assert.match(plain, /overflow:auto;min-height:0/);

  const normal = renderModal(() => ui.modal({ variant: 'warning' }));
  assert.match(normal, /lucide-triangle-alert/);
  assert.match(normal, /lucide-x/);
  assert.match(normal, /backdrop-filter:blur\(4px\)/);
  assert.match(normal, /box-shadow:0 16px 48px/);
  assert.match(normal, /animation:slideUp 0.2s ease/);
  assert.match(normal, /font-size:14px;font-weight:600/);
  assert.equal(ui.modal({ open: false }), '');
});

test('JSON batch dialogs keep their sectioned header, icon and original framing', () => {
  const html = renderModal(ui.sectioned);
  assert.match(html, /border-radius:16px;padding:0;width:440px/);
  assert.match(html, /lucide-list-plus/);
  assert.match(html, /lucide-x/);
  assert.match(html, /backdrop-filter:blur\(4px\)/);
  assert.match(html, /padding:16px 20px;border-bottom:1px solid var\(--color-border\)/);
});

test('TXT scope options are grouped native radios and cannot select a missing current image', () => {
  const html = ui.scope({ appearance: 'radio', hasCurrent: false, style: { marginBottom: 16 } });
  assert.doesNotMatch(html, /tag-option/);
  assert.match(html, /margin-bottom:16px/);
  const radios = [...html.matchAll(/<input type="radio"[^>]*>/g)].map(match => match[0]);
  assert.equal(radios.length, 2);
  assert.equal(radios[0].match(/name="([^"]+)"/)[1], radios[1].match(/name="([^"]+)"/)[1]);
  assert.match(radios[0], /checked/);
  assert.match(radios[1], /disabled/);
  assert.doesNotMatch(radios[1], /checked/);
});

test('JSON scope options keep colored chips by default', () => {
  const html = ui.scope({ value: 'current', color: '#f87171' });
  assert.match(html, /class="tag-option[^"]*"/);
  assert.match(html, /#f87171/);
  assert.equal([...html.matchAll(/type="radio"/g)].length, 2);
});

test('API provider buttons support content widths without changing equal-width callers', () => {
  for (const [props, expected] of [[{}, true], [{ equalPresetWidths: false }, false], [{ compact: true, equalPresetWidths: false }, false]]) {
    const html = ui.api(props);
    const buttons = [...html.matchAll(/<button[^>]*style="([^"]*)">(?:OpenAI|Gemini|DeepSeek|Custom)<\/button>/g)];
    assert.equal(buttons.length, 4);
    for (const [, style] of buttons) {
      assert.equal(style.includes('flex:1'), expected);
      assert.match(style, /font-size:11px/);
    }
  }
});

test('only VLM tagging and assisted tagging opt into content-width API presets', () => {
  const attributes = (file, name) => {
    const source = ts.createSourceFile(file, readFileSync(join(root, file), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const found = [];
    const visit = node => {
      if ((ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) && node.tagName.getText(source) === name) {
        found.push(Object.fromEntries(node.attributes.properties.filter(ts.isJsxAttribute)
          .map(prop => [prop.name.getText(source), prop.initializer?.getText(source) ?? true])));
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
    return found;
  };
  for (const component of ['LlmTaggerTab', 'HybridTaggerTab']) {
    assert.deepEqual(attributes(`src/components/${component}.tsx`, 'LlmApiPanel').map(props => props.equalPresetWidths), ['{false}']);
  }
  for (const component of ['TagSortTab', 'TagRefineTab']) {
    assert.deepEqual(attributes(`src/components/${component}.tsx`, 'LlmApiPanel').map(props => props.equalPresetWidths), [undefined]);
  }
  assert.deepEqual(attributes('src/pages/TagManagerPage.tsx', 'Modal').map(props => [props.width, props.plain]), Array(3).fill(['{380}', true]));
  assert.deepEqual(attributes('src/pages/TagManagerPage.tsx', 'ScopeToggle').map(props => props.appearance), ['"radio"', '"radio"']);
  for (const props of attributes('src/components/JsonTagTab.tsx', 'Modal')) assert.equal(props.plain, undefined);
  for (const props of attributes('src/components/JsonTagTab.tsx', 'ScopeToggle')) assert.equal(props.appearance, undefined);
});

test('empty tag grids keep the subdued folder icon and hint', () => {
  const html = ui.grid();
  assert.match(html, /lucide-folder-open[^>]*style="opacity:0.2"/);
  assert.match(html, /<span style="opacity:0.6">/);
});

test('natural-language translation keeps its italic empty state in every locale', () => {
  assert.match(ui.natural(), /<span style="color:var\(--color-text-tertiary\);font-style:italic">Click translate button to see results<\/span>/);
  assert.equal(ui.hint('en'), 'Click translate button to see results');
  assert.equal(ui.hint('zh-CN'), '\u70b9\u51fb\u7ffb\u8bd1\u6309\u94ae\u67e5\u770b\u7ffb\u8bd1\u7ed3\u679c');
  assert.equal(ui.hint('ja'), '\u7ffb\u8a33\u30dc\u30bf\u30f3\u3092\u30af\u30ea\u30c3\u30af\u3057\u3066\u7d50\u679c\u3092\u8868\u793a');
});
