import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-tagger-token-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'hook.cjs');
await build({
  entryPoints: [join(root, 'src/hooks/useTaggerModels.ts')],
  bundle: true, platform: 'node', format: 'cjs', outfile,
  plugins: [{
    name: 'tagger-token-fixture',
    setup(builder) {
      builder.onResolve({ filter: /^(react|@tauri-apps\/api\/core|\.\.\/utils\/tauriRuntime)$/ }, args => ({ path: args.path, namespace: 'fixture' }));
      builder.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path }) => ({
        contents: path === 'react'
          ? ['useState', 'useEffect', 'useRef', 'useCallback'].map(name =>
            `export const ${name} = (...args) => globalThis.taggerTokenFixture.active.${name}(...args);`).join('\n')
          : path === '@tauri-apps/api/core'
            ? 'export const invoke = (...args) => globalThis.taggerTokenFixture.invoke(...args);'
            : 'export const listen = (...args) => globalThis.taggerTokenFixture.listen(...args);',
      }));
    },
  }],
});
const lib = createRequire(import.meta.url)(outfile);

// Execute the real hook's effects with controlled asynchronous backend replies.
function mountHook(fixture) {
  const slots = [];
  let cursor = 0;
  let effects = [];
  const same = (a, b) => a && b && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));
  const hook = {
    writes: 0, mounted: true,
    useState(initial) {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { value: typeof initial === 'function' ? initial() : initial };
      return [slots[i].value, next => {
        hook.writes++;
        slots[i].value = typeof next === 'function' ? next(slots[i].value) : next;
      }];
    },
    useRef(initial) {
      const i = cursor++;
      return slots[i] ??= { current: initial };
    },
    useCallback(callback, deps) {
      const i = cursor++;
      if (!same(slots[i]?.deps, deps)) slots[i] = { callback, deps };
      return slots[i].callback;
    },
    useEffect(effect, deps) {
      const i = cursor++;
      if (!same(slots[i]?.deps, deps)) effects.push(() => {
        slots[i]?.cleanup?.();
        slots[i] = { deps, cleanup: effect() };
      });
    },
    render() {
      cursor = 0;
      effects = [];
      fixture.active = hook;
      hook.value = lib.useTaggerModels();
      fixture.active = null;
      effects.forEach(run => run());
      return hook.value;
    },
    unmount() {
      if (!hook.mounted) return;
      hook.mounted = false;
      slots.forEach(slot => slot.cleanup?.());
    },
  };
  fixture.hooks.push(hook);
  hook.render();
  return hook;
}

function fixture(t) {
  const previousWindow = globalThis.window;
  const previousFixture = globalThis.taggerTokenFixture;
  const f = {
    hooks: [], requests: [], listeners: [], deferListen: false,
    invoke(command) {
      return new Promise((resolve, reject) => f.requests.push({ command, resolve, reject }));
    },
    listen(event, callback) {
      const subscription = { event, callback, unlistenCalls: 0 };
      f.listeners.push(subscription);
      const unlisten = () => { subscription.unlistenCalls++; };
      return f.deferListen
        ? new Promise(resolve => { subscription.resolve = () => resolve(unlisten); })
        : Promise.resolve(unlisten);
    },
    emitDownload(status) {
      for (const subscription of f.listeners) {
        if (subscription.event === 'tagger-download' && subscription.unlistenCalls === 0) {
          subscription.callback({ payload: { status } });
        }
      }
    },
    take(command) {
      const i = f.requests.findIndex(request => request.command === command);
      assert.notEqual(i, -1, `pending ${command} request`);
      return f.requests.splice(i, 1)[0];
    },
    async flush() {
      await new Promise(resolve => setImmediate(resolve));
      f.hooks.filter(hook => hook.mounted).forEach(hook => hook.render());
    },
  };
  globalThis.window = new EventTarget();
  globalThis.taggerTokenFixture = f;
  t.after(() => {
    f.hooks.forEach(hook => hook.unmount());
    if (previousWindow === undefined) delete globalThis.window;
    else globalThis.window = previousWindow;
    if (previousFixture === undefined) delete globalThis.taggerTokenFixture;
    else globalThis.taggerTokenFixture = previousFixture;
  });
  return f;
}

const model = (requires_token, is_downloaded) => ({
  id: 'fixture-model', name: 'Fixture model', is_builtin: true,
  requires_token, is_downloaded, input_size: 384, supported_categories: ['general', 'character'],
});

test('download token prompt requires an undownloaded protected model and an empty saved token', async t => {
  const cases = [
    [false, false, '', false], [false, false, 'fixture-token', false],
    [false, true, '', false], [false, true, 'fixture-token', false],
    [true, false, '', true], [true, false, 'fixture-token', false],
    [true, true, '', false], [true, true, 'fixture-token', false],
    [true, false, ' \t\n ', true],
  ];
  const f = fixture(t);
  for (const [required, downloaded, token, expected] of cases) {
    const hook = mountHook(f);
    f.take('get_tagger_models').resolve([model(required, downloaded)]);
    f.take('load_huggingface_config').resolve(token);
    await f.flush();
    assert.equal(hook.value.needsDownloadToken, expected, JSON.stringify({ required, downloaded, hasToken: Boolean(token.trim()) }));
  }
});

test('unknown or unreadable token configuration does not produce a missing-token prompt', async t => {
  const f = fixture(t);
  const hook = mountHook(f);
  f.take('get_tagger_models').resolve([model(true, false)]);
  await f.flush();
  assert.equal(hook.value.needsDownloadToken, false);
  f.take('load_huggingface_config').reject(new Error('Fixture config unavailable'));
  await f.flush();
  assert.equal(hook.value.needsDownloadToken, false);
});

test('saved and cleared token notifications refresh both mounted tagging tabs', async t => {
  const f = fixture(t);
  const hooks = [mountHook(f), mountHook(f)];
  for (const hook of hooks) {
    f.take('get_tagger_models').resolve([model(true, false)]);
    f.take('load_huggingface_config').resolve('');
  }
  await f.flush();
  assert.deepEqual(hooks.map(hook => hook.value.needsDownloadToken), [true, true]);
  for (const [token, expected] of [['fixture-saved-token', false], ['', true]]) {
    lib.notifyHuggingFaceConfigChanged();
    assert.equal(f.requests.length, 2);
    hooks.forEach(() => f.take('load_huggingface_config').resolve(token));
    await f.flush();
    assert.deepEqual(hooks.map(hook => hook.value.needsDownloadToken), [expected, expected]);
  }
});

test('a stale token reply or failure cannot overwrite a newer saved-token result', async t => {
  const f = fixture(t);
  const hook = mountHook(f);
  f.take('get_tagger_models').resolve([model(true, false)]);
  const initial = f.take('load_huggingface_config');
  lib.notifyHuggingFaceConfigChanged();
  const olderRefresh = f.take('load_huggingface_config');
  lib.notifyHuggingFaceConfigChanged();
  f.take('load_huggingface_config').resolve('');
  await f.flush();
  assert.equal(hook.value.needsDownloadToken, true);
  initial.resolve('fixture-old-token');
  olderRefresh.reject(new Error('Fixture stale failure'));
  await f.flush();
  assert.equal(hook.value.needsDownloadToken, true);
});

test('unmount removes token listeners and ignores pending configuration replies', async t => {
  const f = fixture(t);
  const hook = mountHook(f);
  const token = f.take('load_huggingface_config');
  const models = f.take('get_tagger_models');
  hook.unmount();
  const writes = hook.writes;
  lib.notifyHuggingFaceConfigChanged();
  lib.notifyTaggerModelsChanged();
  assert.deepEqual(f.requests, []);
  token.resolve('');
  models.resolve([model(true, false)]);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(hook.writes, writes);
});

test('download completion refreshes both tabs before task completion and clearing the token keeps the prompt hidden', async t => {
  const f = fixture(t);
  const hooks = [mountHook(f), mountHook(f)];
  hooks.forEach(() => {
    f.take('get_tagger_models').resolve([model(true, false)]);
    f.take('load_huggingface_config').resolve('fixture-token');
  });
  await f.flush();
  assert.deepEqual(hooks.map(hook => hook.value.cur.is_downloaded), [false, false]);

  // The backend download finishes while inference has not sent a task completion or models-changed event.
  f.emitDownload('done');
  assert.deepEqual(f.requests.map(request => request.command), ['get_tagger_models', 'get_tagger_models']);
  hooks.forEach(() => f.take('get_tagger_models').resolve([model(true, true)]));
  await f.flush();
  assert.deepEqual(hooks.map(hook => hook.value.cur.is_downloaded), [true, true]);

  lib.notifyHuggingFaceConfigChanged();
  hooks.forEach(() => f.take('load_huggingface_config').resolve(''));
  await f.flush();
  assert.deepEqual(hooks.map(hook => hook.value.needsDownloadToken), [false, false]);
});

test('download progress, errors and cancellation do not reload model lists', async t => {
  const f = fixture(t);
  mountHook(f);
  f.take('get_tagger_models').resolve([model(true, false)]);
  f.take('load_huggingface_config').resolve('');
  await f.flush();
  for (const status of ['progress', 'error', 'cancelled']) {
    f.emitDownload(status);
    assert.deepEqual(f.requests, [], status);
  }
});

test('unmount before asynchronous listener registration resolves unsubscribes and ignores late download events', async t => {
  const f = fixture(t);
  f.deferListen = true;
  const hook = mountHook(f);
  f.take('get_tagger_models').resolve([model(true, false)]);
  f.take('load_huggingface_config').resolve('');
  await f.flush();
  assert.equal(f.listeners.length, 1);
  const subscription = f.listeners[0];
  assert.equal(subscription.event, 'tagger-download');

  hook.unmount();
  subscription.callback({ payload: { status: 'done' } });
  assert.deepEqual(f.requests, []);
  subscription.resolve();
  await f.flush();
  assert.equal(subscription.unlistenCalls, 1);
  subscription.callback({ payload: { status: 'done' } });
  assert.deepEqual(f.requests, []);
});

test('an older model request cannot replace the downloaded list returned after a download event', async t => {
  const f = fixture(t);
  const hook = mountHook(f);
  const initialModels = f.take('get_tagger_models');
  f.take('load_huggingface_config').resolve('');
  f.emitDownload('done');
  f.take('get_tagger_models').resolve([model(true, true)]);
  await f.flush();
  assert.equal(hook.value.cur.is_downloaded, true);
  assert.equal(hook.value.needsDownloadToken, false);

  initialModels.resolve([model(true, false)]);
  await f.flush();
  assert.equal(hook.value.cur.is_downloaded, true);
  assert.equal(hook.value.needsDownloadToken, false);
});
