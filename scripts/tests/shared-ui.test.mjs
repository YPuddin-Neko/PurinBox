import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-shared-ui-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'shared-ui.cjs');
buildSync({
  stdin: { contents: `
    import { renderToStaticMarkup } from 'react-dom/server';
    import i18n from 'i18next';
    import { initReactI18next } from 'react-i18next';
    import { Crop } from 'lucide-react';
    import RangeField from './src/components/ui/RangeField';
    import StatChips from './src/components/ui/StatChips';
    import PathInput from './src/components/ui/PathInput';
    export { menuKeyAction } from './src/components/ui/PathInput';
    import ChoiceCard from './src/components/ui/ChoiceCard';
    import ToolPageLayout from './src/components/ui/ToolPageLayout';
    export { formatBytes } from './src/utils/format';
    export { getDefaultPrompts, switchDefaultPrompts } from './src/utils/llmPrompts';
    i18n.use(initReactI18next).init({ lng: 'en', resources: {}, initImmediate: false });
    const noop = () => {};
    export const range = props => renderToStaticMarkup(<RangeField onChange={noop} {...props} />);
    export const stats = props => renderToStaticMarkup(<StatChips {...props} />);
    export const path = props => renderToStaticMarkup(<PathInput onChange={noop} value="" {...props} />);
    export const card = props => renderToStaticMarkup(<ChoiceCard selected={false} onSelect={noop} {...props} />);
    export const layout = props => renderToStaticMarkup(
      <ToolPageLayout icon={Crop} color="#34d399" title="Crop" aside={<b>aside</b>} {...props}><i>left</i></ToolPageLayout>);
  `, loader: 'tsx', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', loader: { '.css': 'empty' }, outfile,
});
const ui = createRequire(import.meta.url)(outfile);

test('formatBytes keeps one decimal below 100 and steps up at 1024', () => {
  assert.equal(ui.formatBytes(0), '0 B');
  assert.equal(ui.formatBytes(1023), '1023 B');
  assert.equal(ui.formatBytes(1536), '1.5 KB');
  assert.equal(ui.formatBytes(45.3 * 1024 ** 2), '45.3 MB');
  assert.equal(ui.formatBytes(512 * 1024 ** 2), '512 MB');
  assert.equal(ui.formatBytes(15.8 * 1024 ** 3), '15.8 GB');
  assert.equal(ui.formatBytes(1024 ** 2 - 1), '1.0 MB');
  assert.equal(ui.formatBytes(99.96 * 1024), '100 KB');
  assert.equal(ui.formatBytes(-5), '0 B');
  assert.equal(ui.formatBytes(Number.NaN), '0 B');
});

test('memory readouts in the header and the monitor keep their own format', () => {
  const header = { memory: true, compact: true };
  const monitor = { memory: true };
  assert.equal(ui.formatBytes(64 * 1024 ** 2, header), '64M');
  assert.equal(ui.formatBytes(15.8 * 1024 ** 3, header), '15.8G');
  assert.equal(ui.formatBytes(512 * 1024 ** 2, monitor), '512 MB');
  assert.equal(ui.formatBytes(64.4 * 1024 ** 2, monitor), '64 MB');
  assert.equal(ui.formatBytes(1024 ** 3, monitor), '1.0 GB');
  assert.equal(ui.formatBytes(128 * 1024 ** 3, monitor), '128.0 GB');
  assert.equal(ui.formatBytes(2048 * 1024 ** 3, monitor), '2048.0 GB');
  assert.equal(ui.formatBytes(1536, monitor), '0 MB');
  assert.equal(ui.formatBytes(0, header), '0M');
});

test('switching the output format only replaces prompts that are still a default', () => {
  const txt = ui.getDefaultPrompts('txt', false);
  const full = ui.getDefaultPrompts('json', false);
  const flat = ui.getDefaultPrompts('json', true);
  assert.deepEqual(ui.switchDefaultPrompts(txt, 'json', false), full);
  // 前一个格式不一定是 txt：任何格式的默认值都会被换掉
  assert.deepEqual(ui.switchDefaultPrompts(flat, 'txt', false), txt);
  assert.deepEqual(ui.switchDefaultPrompts({ sys: '', user: '  ' }, 'json', true), flat);
  // 系统、用户提示词分别判断
  assert.deepEqual(ui.switchDefaultPrompts({ sys: 'my rules', user: txt.user }, 'json', false), { sys: 'my rules', user: full.user });
  assert.deepEqual(ui.switchDefaultPrompts({ sys: 'my rules', user: 'my ask' }, 'json', false), { sys: 'my rules', user: 'my ask' });
});

test('RangeField renders the label, readout and ends of the processing pages', () => {
  const html = ui.range({ label: 'Blur', value: 2, min: 0, max: 10, step: 0.5, color: '#60a5fa', format: v => v.toFixed(1), ends: ['none', 'strong'] });
  assert.match(html, /^<div class="form-group">/);
  assert.match(html, /<label for="([^"]+)" class="form-label ui-range-head"><span>Blur<\/span><span class="ui-range-value" style="color:#60a5fa">2\.0<\/span><\/label><input id="\1" type="range"/);
  assert.match(html, /min="0" max="10" step="0.5" style="accent-color:#60a5fa" value="2"/);
  assert.match(html, /<div class="ui-range-ends"><span>none<\/span><span>strong<\/span><\/div>/);
  const inline = ui.range({ label: 'Style', value: 0.5, min: 0, max: 1, step: 0.1, inline: true, color: '#f472b6', format: v => v.toFixed(1), ends: ['a', 'b'] });
  assert.match(inline, /class="ui-range-inline"/);
  assert.match(inline, /class="ui-range-inline-label" style="color:#f472b6">Style</);
  assert.match(inline, /class="ui-range-inline-value" style="color:#f472b6">0\.5</);
  assert.doesNotMatch(inline, /ui-range-ends/);
  const styled = ui.range({ label: 'D', value: 10, min: 1, max: 20, valueStyle: { fontSize: 13, fontWeight: 700 }, style: { marginBottom: 12 } });
  assert.match(styled, /style="margin-bottom:12px"/);
  assert.match(styled, /style="color:var\(--color-accent-primary\);font-size:13px;font-weight:700"/);
});

test('StatChips makes only clickable chips buttons', () => {
  const html = ui.stats({ items: [
    { label: 'Total', value: 12, color: '#60a5fa' },
    { label: 'Unmatched', value: 3, color: '#f59e0b', onClick: () => {} },
  ] });
  assert.match(html, /^<div class="ui-stats">/);
  assert.match(html, /<div class="ui-stat"><span class="ui-stat-value" style="color:#60a5fa">12<\/span><span class="ui-stat-label">Total<\/span><\/div>/);
  assert.match(html, /<button type="button" class="ui-stat is-clickable" style="border-color:color-mix\(in srgb, #f59e0b 20%, transparent\)">/);
  assert.match(html, /aria-hidden="true">↗<\/span><\/button>/);
  assert.match(ui.stats({ items: [], size: 'md' }), /class="ui-stats is-md"/);
});

test('PathInput picks placeholders and buttons by mode', () => {
  assert.match(ui.path({}), /placeholder="pages.selectInputFolder"/);
  assert.match(ui.path({}), /aria-label="pages.selectInputTitle"/);
  assert.match(ui.path({ kind: 'output' }), /placeholder="pages.selectOutputFolder"[\s\S]*aria-label="pages.selectOutputTitle"/);
  const both = ui.path({ pick: 'folderOrImage' });
  assert.match(both, /placeholder="pages.selectInputPath"/);
  assert.match(both, /title="pages.selectInputPathTitle" aria-haspopup="menu" aria-expanded="false"/);
  assert.match(ui.path({ dialogTitle: 'Pick dataset' }), /<button type="button" class="btn btn-secondary ui-path-btn" aria-label="Pick dataset">/);
  const small = ui.path({ size: 'sm', readOnly: true, placeholder: 'Pick', id: 'p1' });
  assert.match(small, /^<div class="ui-path-row is-sm">/);
  assert.match(small, /<input id="p1" class="form-input" placeholder="Pick" readOnly="" value=""\/>/);
  assert.match(small, /style="width:14px;height:14px"/);
});

test('the folder-or-image menu moves with the arrow keys and closes on Esc or Tab', () => {
  assert.deepEqual(ui.menuKeyAction('ArrowDown', 0, 2), { focus: 1 });
  assert.deepEqual(ui.menuKeyAction('ArrowDown', 1, 2), { focus: 0 });
  assert.deepEqual(ui.menuKeyAction('ArrowUp', 0, 2), { focus: 1 });
  assert.deepEqual(ui.menuKeyAction('ArrowUp', 1, 2), { focus: 0 });
  // 焦点不在菜单项上时，向下到第一项、向上到最后一项
  assert.deepEqual(ui.menuKeyAction('ArrowDown', -1, 2), { focus: 0 });
  assert.deepEqual(ui.menuKeyAction('ArrowUp', -1, 2), { focus: 1 });
  assert.deepEqual(ui.menuKeyAction('Escape', 1, 2), { close: 'restoreFocus' });
  assert.deepEqual(ui.menuKeyAction('Tab', 0, 2), { close: 'leave' });
  assert.equal(ui.menuKeyAction('Enter', 0, 2), null);
  assert.equal(ui.menuKeyAction('ArrowDown', -1, 0), null);
});

test('ChoiceCard column layout and ToolPageLayout skeleton', () => {
  assert.match(ui.card({ layout: 'column', children: 'Copy' }), /class="ui-choice ui-choice-inline is-column"/);
  assert.doesNotMatch(ui.card({ compact: true, children: 'x' }), /is-column/);
  const html = ui.layout({ subtitle: 'sub' });
  assert.match(html, /^<div class="page"><div class="page-header">/);
  assert.match(html, /<div class="ui-tool-grid"><div class="ui-tool-col"><i>left<\/i><\/div><div class="ui-tool-col"><b>aside<\/b><\/div><\/div><\/div>$/);
  assert.match(ui.layout({ gap: 'var(--space-5)' }), /<div class="ui-tool-grid" style="gap:var\(--space-5\)">/);
});
