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
    export { RunIdGate, isCancelledDone, isSkippedProgressEvent } from './src/hooks/useUnifiedTaskLogs';
    export { isCancelMessage, taskStatusFromEvent } from './src/components/TaskContext';
    export { settledTaskStatus, processResultHasIssues } from './src/hooks/useBatchTask';
    export {
      formatSpeed,
      nextSpeedSample,
      formatElapsed,
      estimateRemainingMs,
      formatEtaDuration,
    } from './src/components/ProgressLog';
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

test('command warning counts settle as warning even before progress events arrive', () => {
  for (const result of [
    { success_count: 0, fail_count: 0, warning_count: 1, total: 1 },
    { success_count: 1, fail_count: 0, warning_count: 1, total: 1 },
    { success_count: 0, fail_count: 1, total: 1 },
  ]) {
    assert.equal(lib.processResultHasIssues(result), true);
    assert.equal(lib.settledTaskStatus({ cancelled: false, cancelRequested: false, doneSeen: true, failed: lib.processResultHasIssues(result) }), 'warning');
  }
  for (const result of [null, undefined, {}, { fail_count: 0 }, { fail_count: 0, warning_count: 0 }, { warning_count: '1' }]) {
    assert.equal(lib.processResultHasIssues(result), false);
  }
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

test('ETA only displays for tasks projected to exceed 5 minutes and formats long durations cleanly', () => {
  // Fast task (~25s total): never shows ETA
  let fast = lib.nextSpeedSample(null, 1, 1000, 2000);
  fast = lib.nextSpeedSample(fast, 51, 1000, 4500); // 50 items in 2.5s = 50ms/it
  assert.equal(lib.estimateRemainingMs(fast, 51, 500, false, 4500, 1000), null);

  // Medium task (4 min total): stays hidden under 5-minute threshold
  let medium = lib.nextSpeedSample(null, 1, 1000, 2000);
  medium = lib.nextSpeedSample(medium, 4, 1000, 8000); // 3 items in 6s = 2s/it; 120 items = 4m
  assert.equal(lib.estimateRemainingMs(medium, 4, 120, false, 8000, 1000), null);

  // Immediately computes ETA as soon as speed appears (even after 1 delta in 2s) when projected >= 5 min
  let earlyLong = lib.nextSpeedSample(null, 1, 1000, 2000);
  earlyLong = lib.nextSpeedSample(earlyLong, 2, 1000, 4000); // 1 item in 2s = 2s/it; 300 items = 10m
  assert.equal(lib.formatSpeed(earlyLong), '2.0 s/it');
  const earlyRem = lib.estimateRemainingMs(earlyLong, 2, 300, false, 4000, 1000);
  assert.equal(earlyRem, 298 * 2000);
  assert.equal(lib.formatEtaDuration(earlyRem), '10m');

  // Long VLM task (86.4s/it, 146/434 done -> ~6h 55m remaining)
  let slow = lib.nextSpeedSample(null, 1, 0, 1000);
  slow = lib.nextSpeedSample(slow, 146, 0, 1000 + 145 * 86_400);
  const rem = lib.estimateRemainingMs(slow, 146, 434, false, 1000 + 145 * 86_400, 1000);
  assert.equal(rem, 288 * 86_400);
  assert.equal(lib.formatEtaDuration(rem), '6h 55m');

  // Near completion of a long task still counts down smoothly instead of vanishing at 5m left
  const nearEnd = lib.estimateRemainingMs(slow, 433, 434, false, 1000 + 145 * 86_400 + 50_000, 1000);
  assert.ok(nearEnd !== null && nearEnd < 60_000);
  assert.equal(lib.formatEtaDuration(nearEnd), '< 1m');

  // Completed tasks hide ETA immediately
  assert.equal(lib.estimateRemainingMs(slow, 434, 434, true, 1000 + 433 * 86_400, 1000), null);

  // Elapsed formatting switches to hours cleanly
  assert.equal(lib.formatElapsed(45_000), '45s');
  assert.equal(lib.formatElapsed(12 * 60_000 + 30_000), '12m30s');
  assert.equal(lib.formatElapsed(209 * 60_000 + 45_000), '3h29m45s');
  assert.equal(lib.formatEtaDuration(28 * 3600_000), '1d 4h');
});

test('skipped file progress events are classified separately from successes', () => {
  assert.equal(lib.isSkippedProgressEvent({ status: 'skipped', message: 'a.png' }), true);
  assert.equal(lib.isSkippedProgressEvent({ status: 'success', message: '[跳过] a.png (已有描述)' }), true);
  assert.equal(lib.isSkippedProgressEvent({ status: 'success', message: '  [SKIP] b.png' }), true);
  assert.equal(lib.isSkippedProgressEvent({ status: 'success', message: '[完成] a.png' }), false);
  assert.equal(lib.isSkippedProgressEvent({ status: 'error', message: '[跳过] bad.png' }), false);
});
