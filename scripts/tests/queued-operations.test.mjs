import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';
const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-queue-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'queue.cjs');
buildSync({ stdin: { contents: `export * from './src/utils/queuedInvoker'; export * from './src/queuedCommands'; export { TASK_ROUTE_MAP } from './src/appRegistry';`, resolveDir: root, loader: 'ts' }, bundle: true, platform: 'node', format: 'cjs', outfile });
const { createQueuedInvoker, resultHasFailures, QUEUED_COMMANDS, TASK_ROUTE_MAP } = createRequire(import.meta.url)(outfile);
function fixture(command, exec) {
  const tasks = new Map(), listeners = new Map();
  const deps = {
    commands: QUEUED_COMMANDS, name: c => c,
    actions: { addTask: (id, name) => tasks.set(id, { name, status: 'running' }), updateTask: (id, patch) => Object.assign(tasks.get(id), patch) },
    listen: async (event, handler) => { listeners.set(event, handler); return () => listeners.delete(event); },
    invoke: async (id) => id === command ? exec() : undefined,
  };
  return { tasks, listeners, invoke: createQueuedInvoker(deps), remount: () => createQueuedInvoker(deps), emit: p => [...listeners.values()].forEach(fn => fn({ payload: p })) };
}
const tick = () => new Promise(resolve => setImmediate(resolve));
test('every omitted command has a task route and its page uses queued invoke', () => {
  assert.equal(Object.keys(QUEUED_COMMANDS).length, 15);
  for (const [id, def] of Object.entries(QUEUED_COMMANDS)) assert.equal(TASK_ROUTE_MAP[id], def.route);
  for (const page of ['ImageDedup', 'BucketPreview', 'DatasetBalancer', 'Settings', 'TagManager', 'PersonCrop']) {
    assert.match(readFileSync(join(root, `src/pages/${page}Page.tsx`), 'utf8'), /const invoke = useQueuedInvoke\(\)/);
  }
  assert.equal(TASK_ROUTE_MAP['image-dedup'], '/image-dedup');
  assert.equal(TASK_ROUTE_MAP.workflow, '/workflow');
});
test('progress and early done do not finish task before backend returns', async () => {
  let resolve;
  const f = fixture('analyze_buckets', () => new Promise(r => { resolve = r; }));
  const pending = f.invoke('analyze_buckets'); await tick();
  f.emit({ status: 'processing', current: 2, total: 5, message: 'scanning' });
  assert.equal(f.tasks.get('analyze_buckets').current, 2);
  f.emit({ status: 'done' });
  assert.equal(f.tasks.get('analyze_buckets').status, 'running');
  resolve({}); await pending;
  assert.equal(f.tasks.get('analyze_buckets').status, 'done');
  assert.equal(f.listeners.size, 0);
});
test('failure releases task and retry works after remount', async () => {
  let fail = true;
  const f = fixture('scan_concept_folders', () => { if (fail) throw Error('disk error'); return []; });
  await assert.rejects(f.invoke('scan_concept_folders'), /disk error/);
  assert.equal(f.tasks.get('scan_concept_folders').status, 'error');
  fail = false; await f.remount()('scan_concept_folders');
  assert.equal(f.tasks.get('scan_concept_folders').status, 'done');
});
test('duplicate start does not replace active task', async () => {
  let resolve;
  const f = fixture('export_buckets', () => new Promise(r => { resolve = r; }));
  const pending = f.invoke('export_buckets'); await tick();
  await assert.rejects(f.remount()('export_buckets'), /任务正在进行/);
  assert.equal(f.tasks.get('export_buckets').status, 'running');
  resolve('exported'); await pending;
});
test('cancel stays running until command exits and then allows retry', async () => {
  let reject;
  const f = fixture('recommend_bucket_params', () => new Promise((_, r) => { reject = r; }));
  const pending = f.invoke('recommend_bucket_params'); await tick();
  await f.invoke('cancel_bucket_recommend');
  assert.equal(f.tasks.get('recommend_bucket_params').status, 'running');
  reject('已取消分析'); await assert.rejects(pending);
  assert.equal(f.tasks.get('recommend_bucket_params').status, 'cancelled');
});
test('backend cancellation returned as success is still cancelled', async () => {
  let resolve;
  const f = fixture('download_danbooru_tags', () => new Promise(r => { resolve = r; }));
  const pending = f.invoke('download_danbooru_tags'); await tick();
  f.emit({ status: 'done', cancelled: true }); resolve(0); await pending;
  assert.equal(f.tasks.get('download_danbooru_tags').status, 'cancelled');
});
test('partial save/delete failures and event warnings cannot become successful', async () => {
  for (const result of [{ saved: 2, failed: [{ path: 'x' }] }, { deleted: 2, failed: 1 }, { failed_files: ['x'] }]) assert.ok(resultHasFailures(result));
  assert.equal(resultHasFailures({ saved: 3, failed: [] }), false);
  const f = fixture('save_all_tag_files', () => ({ saved: 2, failed: [{ path: 'x' }] }));
  await f.invoke('save_all_tag_files');
  assert.equal(f.tasks.get('save_all_tag_files').status, 'warning');
});
test('all registered commands finish with a task record', async () => {
  for (const command of Object.keys(QUEUED_COMMANDS)) {
    const f = fixture(command, () => undefined);
    await f.invoke(command);
    assert.equal(f.tasks.get(command).status, 'done', command);
  }
});
test('late callbacks cannot overwrite a retry and old run events are ignored', async () => {
  let resolve;
  const f = fixture('analyze_buckets', () => new Promise(r => { resolve = r; }));
  let pending = f.invoke('analyze_buckets'); await tick();
  const oldCallback = f.listeners.get('bucket-progress');
  f.emit({ status: 'done', run_id: 10 }); resolve({}); await pending;
  pending = f.remount()('analyze_buckets'); await tick();
  oldCallback({ payload: { status: 'error', message: 'late' } });
  f.emit({ status: 'done', cancelled: true, run_id: 10 });
  f.emit({ status: 'processing', message: 'new run', run_id: 11 });
  resolve({}); await pending;
  assert.equal(f.tasks.get('analyze_buckets').status, 'done');
  assert.equal(f.tasks.get('analyze_buckets').message, 'new run');
});
test('cancel during subscription prevents starting a command that would clear cancellation', async () => {
  let subscribed, calls = 0;
  const tasks = new Map();
  const invoke = createQueuedInvoker({
    commands: QUEUED_COMMANDS, name: c => c,
    actions: { addTask: id => tasks.set(id, { status: 'running' }), updateTask: (id, patch) => Object.assign(tasks.get(id), patch) },
    listen: () => new Promise(resolve => { subscribed = resolve; }),
    invoke: async command => { if (command === 'analyze_buckets') calls++; },
  });
  const pending = invoke('analyze_buckets');
  await invoke('cancel_bucket_analysis'); subscribed(() => {});
  await assert.rejects(pending, /已取消/);
  assert.equal(calls, 0);
  assert.equal(tasks.get('analyze_buckets').status, 'cancelled');
});
