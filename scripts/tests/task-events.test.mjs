import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-task-events-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'task-events.cjs');
buildSync({
  stdin: { contents: `
    export { RunIdGate, isCancelledDone } from './src/hooks/useUnifiedTaskLogs';
    export { isCancelMessage, taskStatusFromEvent } from './src/components/TaskContext';
    export { settledTaskStatus } from './src/hooks/useBatchTask';
    export { formatSpeed, nextSpeedSample } from './src/components/ProgressLog';
  `, loader: 'ts', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', jsx: 'automatic', loader: { '.css': 'empty' }, outfile,
});
// i18n 初始化时读 localStorage
Object.defineProperty(globalThis, 'localStorage', {
  value: { getItem: () => null, setItem: () => {} }, configurable: true, writable: true,
});
const lib = createRequire(import.meta.url)(outfile);

test('a new run drops late events of every earlier run on the channel', () => {
  const gate = new lib.RunIdGate();
  assert.equal(gate.accept(3), true);
  // 别处（工作流）跑的一轮，本页不处理，但要记下
  assert.equal(gate.accept(4), true);
  gate.begin();
  assert.equal(gate.accept(3), false);
  assert.equal(gate.accept(4), false);
  assert.equal(gate.accept(5), true);
  assert.equal(gate.accept(5), true);
  gate.begin();
  assert.equal(gate.accept(5), false);
  assert.equal(gate.accept(6), true);
});

test('events without run_id are always accepted', () => {
  const gate = new lib.RunIdGate();
  gate.accept(9);
  gate.begin();
  assert.equal(gate.accept(undefined), true);
  assert.equal(gate.accept(Number.NaN), true);
  assert.equal(gate.accept(9), false);
});

test('only the cancelled flag or a 已取消 prefix count as cancellation', () => {
  assert.equal(lib.isCancelledDone({ status: 'done', cancelled: true }), true);
  assert.equal(lib.isCancelledDone({ status: 'done' }), false);
  assert.equal(lib.isCancelledDone({ status: 'error', cancelled: true }), false);
  assert.equal(lib.isCancelMessage('已取消: 已处理 2, 共 4'), true);
  assert.equal(lib.isCancelMessage('  已取消'), true);
  // 子串匹配会把这些真实失败当成取消
  assert.equal(lib.isCancelMessage('读取失败: D:/data/cancelled_set/a.png'), false);
  assert.equal(lib.isCancelMessage('Request cancelled by proxy'), false);
  assert.equal(lib.isCancelMessage('目录「已取消」不存在'), false);
});

test('per-file warning, skipped, info and error events keep the task running', () => {
  for (const status of ['processing', 'success', 'warning', 'skipped', 'info', 'error']) {
    assert.equal(lib.taskStatusFromEvent({ status }, false), 'running', status);
  }
  assert.equal(lib.taskStatusFromEvent({ status: 'done' }, false), 'done');
  assert.equal(lib.taskStatusFromEvent({ status: 'done' }, true), 'warning');
  assert.equal(lib.taskStatusFromEvent({ status: 'done', cancelled: true }, true), 'cancelled');
  assert.equal(lib.taskStatusFromEvent({ status: 'done', message: '已取消' }, false), 'done');
});

test('a resolved run settles as done, warning or cancelled', () => {
  const base = { cancelled: false, cancelRequested: false, doneSeen: true, failed: false };
  assert.equal(lib.settledTaskStatus(base), 'done');
  assert.equal(lib.settledTaskStatus({ ...base, failed: true }), 'warning');
  assert.equal(lib.settledTaskStatus({ ...base, cancelled: true, failed: true }), 'cancelled');
  // 点了取消但后端照常发完 done：以后端为准
  assert.equal(lib.settledTaskStatus({ ...base, cancelRequested: true }), 'done');
  assert.equal(lib.settledTaskStatus({ ...base, cancelRequested: true, doneSeen: false }), 'cancelled');
});

test('speed is measured from the first counted item, not from the click', () => {
  let sample = lib.nextSpeedSample(null, 0, 1000, 1000);
  assert.equal(sample, null);
  // 部署环境和下载模型用了 60 秒，第一张完成
  sample = lib.nextSpeedSample(sample, 1, 1000, 61_000);
  assert.equal(lib.formatSpeed(sample), '');
  sample = lib.nextSpeedSample(sample, 5, 1000, 63_000);
  assert.equal(lib.formatSpeed(sample), '2.0 it/s');
  sample = lib.nextSpeedSample(sample, 5, 1000, 90_000);
  assert.equal(lib.formatSpeed(sample), '2.0 it/s');
  sample = lib.nextSpeedSample(sample, 6, 1000, 73_000);
  assert.equal(lib.formatSpeed(sample), '2.4 s/it');
  // 换轮或计数归零时重新取样
  assert.equal(lib.nextSpeedSample(sample, 6, 2000, 80_000).firstCount, 6);
  assert.equal(lib.nextSpeedSample(sample, 0, 1000, 80_000), null);
});
