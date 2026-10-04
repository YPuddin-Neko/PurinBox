import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-workflow-'));
after(() => rmSync(output, { recursive: true, force: true }));

// 后端命令由各测试通过 globalThis.__invoke 应答
const stub = join(output, 'tauri-core.js');
writeFileSync(stub, 'export async function invoke(cmd, args) { return globalThis.__invoke(cmd, args); }\n');
const outfile = join(output, 'workflow.cjs');
buildSync({
  stdin: { contents: `
    export { planWorkflow } from './src/components/workflow/workflowValidation';
    export { fillBlankPrompts, getNodeDef, withDefaults } from './src/components/workflow/nodeDefinitions';
    export { parseWorkflow, serializeWorkflow } from './src/components/workflow/workflowFile';
    export { WorkflowEngine } from './src/components/workflow/WorkflowEngine';
    export { getDefaultPrompts } from './src/utils/llmPrompts';
    export { setSystemStatsInterval, subscribeSystemStats, getSystemStats } from './src/hooks/useSystemStats';
  `, loader: 'ts', resolveDir: root },
  bundle: true, platform: 'node', format: 'cjs', loader: { '.css': 'empty' }, outfile,
  alias: { '@tauri-apps/api/core': stub },
});
const lib = createRequire(import.meta.url)(outfile);

const node = (id, type, params = {}) => ({ id, type: 'baseNode', position: { x: 0, y: 0 }, data: { type, params, status: 'idle' } });
const edge = (source, target, sourceHandle) => ({ id: `${source}-${sourceHandle ?? ''}-${target}`, source, target, sourceHandle });
const input = (id, path = '/data/in') => node(id, 'image-folder', { path });
const out = (id, path = '/data/out') => node(id, 'output-folder', { path });
const issueOf = (nodes, edges) => lib.planWorkflow(nodes, edges).issue;

test('a node fed by two upstreams that can run together is rejected before anything runs', () => {
  const nodes = [input('in'), node('flip', 'flip'), out('o')];
  assert.deepEqual(issueOf(nodes, [edge('in', 'flip'), edge('flip', 'o'), edge('in', 'o')]), { kind: 'multipleInputs', nodeId: 'o' });
  const twoInputs = [input('a', '/data/a'), input('b', '/data/b'), node('scale', 'scale'), out('o')];
  assert.deepEqual(issueOf(twoInputs, [edge('a', 'scale'), edge('b', 'scale'), edge('scale', 'o')]), { kind: 'multipleInputs', nodeId: 'scale' });
});

test('the two branches of a bucket node may merge into one node', () => {
  const nodes = [input('in'), node('b', 'bucket-assign'), node('crop', 'crop'), node('scale', 'scale'), out('o')];
  const edges = [
    edge('in', 'b'), edge('b', 'crop', 'output-a'), edge('b', 'scale', 'output-b'),
    edge('crop', 'o'), edge('scale', 'o'),
  ];
  const plan = lib.planWorkflow(nodes, edges);
  assert.equal(plan.issue, null);
  assert.deepEqual(plan.order, ['in', 'b', 'crop', 'scale', 'o']);
  // 同一分桶出口上的两条支线可以同时运行，汇合就不行
  const sameBranch = [...edges.slice(0, 2), edge('b', 'scale', 'output-a'), ...edges.slice(3)];
  assert.deepEqual(issueOf(nodes, sameBranch), { kind: 'multipleInputs', nodeId: 'o' });
});

test('a chain whose result would only stay in .workflow_temp must end in an output node', () => {
  assert.deepEqual(issueOf([input('in'), node('scale', 'scale')], [edge('in', 'scale')]), { kind: 'needsOutput', nodeId: 'scale' });
  // 就地节点跟在临时目录后面，结果仍在临时目录里
  const temp = [input('in'), node('scale', 'scale'), node('tag', 'tagger')];
  assert.deepEqual(issueOf(temp, [edge('in', 'scale'), edge('scale', 'tag')]), { kind: 'needsOutput', nodeId: 'tag' });
  // 就地处理输入文件夹、接了输出文件夹、删除模式的筛选都不需要
  assert.equal(issueOf([input('in'), node('tag', 'tagger')], [edge('in', 'tag')]), null);
  assert.equal(issueOf([input('in'), node('scale', 'scale'), node('tag', 'tagger'), out('o')],
    [edge('in', 'scale'), edge('scale', 'tag'), edge('tag', 'o')]), null);
  assert.equal(issueOf([input('in'), node('f', 'filter', { action: 'delete' })], [edge('in', 'f')]), null);
  assert.deepEqual(issueOf([input('in'), node('f', 'filter', { action: 'copy' })], [edge('in', 'f')]), { kind: 'needsOutput', nodeId: 'f' });
  // 分桶有一个出口没接东西：输入在临时目录时那个分支的结果会丢
  const bucket = [input('in'), node('scale', 'scale'), node('b', 'bucket-assign'), node('crop', 'crop'), out('o')];
  const bucketEdges = [edge('in', 'scale'), edge('scale', 'b'), edge('b', 'crop', 'output-a'), edge('crop', 'o')];
  assert.deepEqual(issueOf(bucket, bucketEdges), { kind: 'needsOutput', nodeId: 'b' });
  assert.equal(issueOf(bucket.filter(n => n.id !== 'scale'), [edge('in', 'b'), ...bucketEdges.slice(2)]), null);
});

test('missing paths, model names, inputs and cycles are reported before the run', () => {
  assert.deepEqual(issueOf([input('in', ' ')], []), { kind: 'missingInputPath', nodeId: 'in' });
  assert.deepEqual(issueOf([input('in'), out('o', '')], [edge('in', 'o')]), { kind: 'missingOutputPath', nodeId: 'o' });
  assert.deepEqual(issueOf([input('in'), node('vlm', 'llm-tagger', { model_name: '' })], [edge('in', 'vlm')]), { kind: 'missingModel', nodeId: 'vlm' });
  assert.equal(issueOf([input('in'), node('vlm', 'llm-tagger', { model_name: 'gpt-4o' })], [edge('in', 'vlm')]), null);
  assert.deepEqual(issueOf([input('in'), node('scale', 'scale'), out('o')], [edge('scale', 'o')]), { kind: 'noInput', nodeId: 'scale' });
  assert.deepEqual(issueOf([input('in'), node('a', 'flip'), node('b', 'crop')], [edge('in', 'a'), edge('a', 'b'), edge('b', 'a')]), { kind: 'cycle', nodeId: '' });
  assert.deepEqual(issueOf([input('in', '/data/.workflow_temp/step_1_scale')], []), { kind: 'inputInTemp', nodeId: 'in' });
  // 端点已删除的连线不算环
  assert.equal(issueOf([input('in'), node('tag', 'tagger')], [edge('in', 'tag'), edge('gone', 'tag'), edge('tag', 'gone')]), null);
});

test('saved node params stay in effect and only missing ones take the new defaults', () => {
  const blur = lib.getNodeDef('blur-noise');
  assert.deepEqual(lib.withDefaults(blur, { blur_radius: 0, noise_strength: 0 }), { blur_radius: 0, noise_strength: 0 });
  assert.deepEqual(lib.withDefaults(blur, {}), { blur_radius: 2, noise_strength: 15 });
  const vlm = lib.getNodeDef('llm-tagger');
  const json = lib.getDefaultPrompts('json', false);
  const old = lib.withDefaults(vlm, { temperature: 0.7, system_prompt: '', output_format: 'json', api_key: 'secret', api_endpoint: 'x' });
  assert.equal(old.temperature, 0.7);
  // 编辑中清空的提示词不回填，文本框保持为空
  assert.equal(old.system_prompt, '');
  assert.equal(old.user_prompt, json.user);
  assert.equal('api_key' in old, false);
  assert.equal('api_endpoint' in old, false);
  assert.equal(lib.withDefaults(vlm, {}).temperature, 0.2);
  // 多行提示词原样保留
  const multi = 'line 1\nline 2';
  assert.equal(lib.withDefaults(vlm, { system_prompt: multi }).system_prompt, multi);
  assert.equal(vlm.params.find(p => p.key === 'system_prompt').multiline, true);
});

test('a blank VLM prompt means the default prompt of the output format when loaded and when run', async () => {
  const flat = lib.getDefaultPrompts('json', true);
  const vlmFile = params => JSON.stringify({
    version: 1, name: 'w', edges: [],
    nodes: [{ id: 'vlm', type: 'llm-tagger', position: { x: 0, y: 0 }, data: { type: 'llm-tagger', params } }],
  });
  const loaded = lib.parseWorkflow(vlmFile({ model_name: 'm', output_format: 'json_simplified', system_prompt: '', user_prompt: ' \n ', temperature: 0.7 }));
  const params = loaded.nodes[0].data.params;
  assert.equal(params.system_prompt, flat.sys);
  assert.equal(params.user_prompt, flat.user);
  assert.equal(params.temperature, 0.7);
  const kept = lib.parseWorkflow(vlmFile({ output_format: 'txt', system_prompt: 'my rules', user_prompt: '' })).nodes[0].data.params;
  assert.equal(kept.system_prompt, 'my rules');
  assert.equal(kept.user_prompt, lib.getDefaultPrompts('txt', false).user);
  // 其他节点的空串参数不受影响
  assert.equal(lib.fillBlankPrompts('tagger', { exclude_tags: '' }).exclude_tags, '');

  globalThis.__invoke = async () => ({ preset: 'openai', custom_endpoint: '', api_keys: { openai: 'key' } });
  const def = lib.getNodeDef('llm-tagger');
  const run = async params => (await def.buildOptions(lib.withDefaults(def, params), { input_path: '/in', output_path: '/out' })).options;
  const cleared = await run({ model_name: 'm', output_format: 'json_simplified', system_prompt: '', user_prompt: '   ' });
  assert.equal(cleared.system_prompt, flat.sys);
  assert.equal(cleared.user_prompt, flat.user);
  const custom = await run({ model_name: 'm', output_format: 'json', system_prompt: 'sys', user_prompt: 'ask' });
  assert.deepEqual([custom.system_prompt, custom.user_prompt], ['sys', 'ask']);
});

test('nodes build their command options with the shared builders', async () => {
  const io = { input_path: '/in', output_path: '/out', recursive: false };
  const build = async (type, params) => {
    const def = lib.getNodeDef(type);
    return (await def.buildOptions(lib.withDefaults(def, params), io)).options;
  };
  const crop = await build('person-crop', {});
  assert.deepEqual(
    [crop.use_gpu, crop.upper_enabled, crop.upper_conf, crop.head_conf, crop.eyes_enabled, crop.eyes_scale, crop.upper_tag, crop.keep_original_tags],
    [true, false, 0.5, 0.4, false, 2.4, '', true],
  );
  const bucket = await build('bucket-assign', { res_width: 768, res_height: 1280 });
  assert.equal(bucket.steps, 64);
  assert.equal(bucket.max_bucket_reso, 1280);
  globalThis.__invoke = async cmd => {
    assert.equal(cmd, 'load_api_config');
    return { preset: 'openai', custom_endpoint: '', api_keys: { openai: 'key' } };
  };
  const vlm = await build('llm-tagger', { model_name: 'm', request_interval: 1.5, output_format: 'json_simplified' });
  assert.deepEqual(
    [vlm.api_key, vlm.request_interval_ms, vlm.top_p, vlm.output_format, vlm.json_simplified, vlm.temperature],
    ['key', 1500, 0, 'json', true, 0.2],
  );
});

test('a .purin file with the wrong structure is rejected without throwing', () => {
  assert.equal(lib.parseWorkflow('null'), null);
  assert.equal(lib.parseWorkflow('{"nodes": [null], "edges": []}'), null);
  assert.equal(lib.parseWorkflow('{"nodes": [{"id": "a", "data": {"type": "scale"}}], "edges": []}'), null);
  assert.equal(lib.parseWorkflow('{"nodes": [], "edges": [null]}'), null);
  assert.equal(lib.parseWorkflow('not json'), null);
  const file = JSON.stringify({
    version: 1, name: 'w',
    nodes: [
      { id: 'node_1', type: 'blur-noise', position: { x: 1, y: 2 }, data: { type: 'blur-noise', params: { blur_radius: 0 } } },
      { id: 'node_2', type: 'output-folder', position: { x: 3, y: 4 }, data: { type: 'output-folder', params: { path: '/o' } } },
    ],
    edges: [{ id: 'e1', source: 'node_1', target: 'node_2', sourceHandle: null }, { id: 'e2', source: 'node_9', target: 'node_2' }],
  });
  const loaded = lib.parseWorkflow(file);
  assert.deepEqual(loaded.nodes[0].data.params, { blur_radius: 0, noise_strength: 15 });
  assert.deepEqual(loaded.edges.map(e => e.id), ['e1']);
  assert.equal(loaded.edges[0].sourceHandle, undefined);
  assert.deepEqual(lib.parseWorkflow(lib.serializeWorkflow('w', loaded.nodes, loaded.edges)), loaded);
});

const t = (key, params) => (params ? `${key} ${JSON.stringify(params)}` : key);

async function run(nodes, edges, handler) {
  const calls = [];
  globalThis.__invoke = async (cmd, args) => {
    calls.push(cmd);
    return handler(cmd, args);
  };
  const status = new Map();
  const errors = [];
  let completed = false;
  await new lib.WorkflowEngine(t).execute(nodes, edges, {
    onNodeStatusChange: (id, s, message) => status.set(id, { status: s, message }),
    onStepStart: () => {},
    onComplete: () => { completed = true; },
    onError: (id, error) => errors.push({ id, error }),
  });
  return { calls, status, errors, completed };
}

const ok = { success_count: 1, fail_count: 0, total: 1, errors: [] };

test('a failed copy into the output folder marks the output node as failed', async () => {
  const result = await run([input('in'), node('scale', 'scale'), out('o')], [edge('in', 'scale'), edge('scale', 'o')], (cmd, args) => {
    if (cmd === 'carry_tag_sidecars' && args.copyImages) throw '复制图片失败: 磁盘已满';
    return cmd === 'scale_images' ? ok : 0;
  });
  assert.equal(result.completed, false);
  assert.deepEqual(result.status.get('o'), { status: 'error', message: '复制图片失败: 磁盘已满' });
  assert.deepEqual(result.errors, [{ id: 'o', error: '复制图片失败: 磁盘已满' }]);
  assert.ok([...result.status.values()].every(s => s.status !== 'running'));
});

test('a workflow that fails the pre-run check runs no command and keeps the temp folder', async () => {
  const result = await run([input('in'), node('scale', 'scale')], [edge('in', 'scale')], () => ok);
  assert.deepEqual(result.calls, []);
  assert.deepEqual(result.status.get('scale'), { status: 'error', message: 'workflow.errNeedsOutput {"name":"workflow.nodeScale"}' });
  assert.deepEqual(result.errors, [{ id: 'scale', error: 'workflow.errNeedsOutput {"name":"workflow.nodeScale"}' }]);
});

test('only the active branch of a bucket node runs and the merged output receives it', async () => {
  const nodes = [input('in'), node('b', 'bucket-assign'), node('crop', 'crop'), node('scale', 'scale'), out('o')];
  const edges = [edge('in', 'b'), edge('b', 'crop', 'output-a'), edge('b', 'scale', 'output-b'), edge('crop', 'o'), edge('scale', 'o')];
  const result = await run(nodes, edges, cmd => {
    if (cmd === 'analyze_buckets') return { bucket_count: 1, buckets: [{ bucket_width: 1024, bucket_height: 1024, image_count: 10 }] };
    return cmd === 'crop_images' ? ok : 0;
  });
  assert.equal(result.completed, true);
  assert.ok(result.calls.includes('crop_images'));
  assert.ok(!result.calls.includes('scale_images'));
  assert.equal(result.status.get('scale').status, 'idle');
  assert.equal(result.status.get('o').status, 'done');
  assert.match(result.status.get('b').message, /^workflow\.bucketResult .*"branch":"workflow\.bucketBranchUniform"/);
  // 运行前后各清理一次临时目录
  assert.equal(result.calls.filter(c => c === 'cleanup_workflow_temp').length, 2);
});

test('changing the monitor interval keeps the stats on screen', async () => {
  globalThis.window = { __TAURI_INTERNALS__: {} };
  try {
    let polls = 0;
    globalThis.__invoke = async cmd => {
      assert.equal(cmd, 'get_system_stats');
      polls += 1;
      return { cpu_usage: polls };
    };
    const seen = [];
    const unsubscribe = lib.subscribeSystemStats(() => seen.push(lib.getSystemStats()));
    lib.setSystemStatsInterval(1000);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(lib.getSystemStats().cpu_usage, 1);
    // 换间隔立即重新采样，中途不清空
    lib.setSystemStatsInterval(2000);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(lib.getSystemStats().cpu_usage, 2);
    // 订阅者暂时为空时保留数据
    unsubscribe();
    const again = lib.subscribeSystemStats(() => {});
    assert.equal(lib.getSystemStats().cpu_usage, 2);
    assert.ok(!seen.includes(null));
    // 关闭监控才清空
    lib.setSystemStatsInterval(0);
    assert.equal(lib.getSystemStats(), null);
    again();
  } finally {
    delete globalThis.window;
  }
});
