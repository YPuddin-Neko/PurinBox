import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { buildSync } from 'esbuild';
import ts from 'typescript';

const root = fileURLToPath(new URL('../../', import.meta.url));
const require = createRequire(import.meta.url);
const { outputFiles } = buildSync({
  stdin: {
    contents: `
      export { default as zh } from './src/i18n/locales/zh-CN';
      export { default as en } from './src/i18n/locales/en';
      export { default as ja } from './src/i18n/locales/ja';
    `,
    resolveDir: root,
  },
  bundle: true, platform: 'node', format: 'cjs', write: false,
});
const module = { exports: {} };
new Function('module', 'exports', 'require', outputFiles[0].text)(module, module.exports, require);

function flatten(value, prefix = '', result = {}) {
  for (const [key, text] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (text && typeof text === 'object') flatten(text, path, result);
    else result[path] = text;
  }
  return result;
}

const locales = Object.fromEntries(Object.entries(module.exports).map(([lang, value]) => [lang, flatten(value)]));

test('all locales contain the same translation keys', () => {
  const keys = Object.keys(locales.zh).sort();
  for (const [lang, values] of Object.entries(locales)) {
    assert.deepEqual(Object.keys(values).sort(), keys, lang);
  }
});

for (const file of ['src/pages/UpscalePage.tsx', 'src/components/HybridTaggerTab.tsx']) {
  test(`${file} resolves literal translation calls in every locale`, () => {
    const source = ts.createSourceFile(file, readFileSync(new URL(`../../${file}`, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const keys = new Set();
    const visit = node => {
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 't') {
        const argument = node.arguments[0];
        if (argument && ts.isStringLiteral(argument)) keys.add(argument.text);
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
    assert.ok(keys.size > 0);
    for (const [lang, values] of Object.entries(locales)) {
      for (const key of keys) assert.equal(typeof values[key], 'string', `${lang}: ${key}`);
    }
  });
}
