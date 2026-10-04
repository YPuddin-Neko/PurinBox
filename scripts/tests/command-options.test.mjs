import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const output = mkdtempSync(join(tmpdir(), 'purin-command-options-'));
after(() => rmSync(output, { recursive: true, force: true }));
const outfile = join(output, 'command-options.cjs');
buildSync({
  entryPoints: [join(root, 'src/api/commandOptions.ts')],
  bundle: true, platform: 'node', format: 'cjs', outfile,
});
const api = createRequire(import.meta.url)(outfile);
const { getDefaultPrompts } = createRequire(import.meta.url)(buildPrompts());

function buildPrompts() {
  const file = join(output, 'prompts.cjs');
  buildSync({ entryPoints: [join(root, 'src/utils/llmPrompts.ts')], bundle: true, platform: 'node', format: 'cjs', outfile: file });
  return file;
}

const io = { input_path: '/in', output_path: '/out', recursive: true };

test('every builder fills omitted settings with the page defaults', () => {
  assert.deepEqual(api.buildCropOptions(io), { ...io, ...api.CROP_DEFAULTS });
  assert.deepEqual(api.buildFlipOptions(io), { ...io, direction: 'horizontal' });
  assert.deepEqual(api.buildPerspectiveOptions(io), { ...io, intensity: 0.1 });
  assert.deepEqual(api.buildBlurNoiseOptions(io), { ...io, blur_radius: 2, noise_strength: 15 });
  assert.deepEqual(api.buildAlphaConvertOptions(io), { ...io, background: 'white' });
  assert.deepEqual(api.buildFormatConvertOptions(io), { ...io, target_format: 'png' });
  assert.deepEqual(api.buildFilterOptions(io), { ...io, action: 'copy', condition: 'below_resolution', width: 512, height: 512 });
  assert.deepEqual(api.buildPersonCropOptions(io), { ...io, ...api.PERSON_CROP_DEFAULTS });
  // undefined 与缺省同义：节点参数缺项时不会把 undefined 送进 IPC
  assert.deepEqual(api.buildFlipOptions(io, { direction: undefined }), { ...io, direction: 'horizontal' });
});

test('person crop fills settings a caller does not expose with the page defaults', () => {
  const options = api.buildPersonCropOptions(io, { use_gpu: true, person_enabled: true, upper_enabled: false, head_enabled: false, person_conf: 0.3 });
  assert.equal(options.upper_conf, 0.5);
  assert.equal(options.head_conf, 0.4);
  assert.equal(options.eyes_scale, 2.4);
  assert.equal(options.use_gpu, true);
  assert.equal(options.upper_enabled, false);
});

test('scale picks the target pair by mode and only sends the downscale target for both', () => {
  const sizes = { upscale_width: 1024, upscale_height: 768, downscale_width: 512, downscale_height: 384 };
  assert.deepEqual(api.buildScaleOptions(io, { mode: 'upscale', ...sizes }), {
    ...io, mode: 'upscale', target_width: 1024, target_height: 768, down_target_width: 0, down_target_height: 0,
  });
  assert.deepEqual(api.buildScaleOptions(io, { mode: 'downscale', ...sizes }), {
    ...io, mode: 'downscale', target_width: 512, target_height: 384, down_target_width: 0, down_target_height: 0,
  });
  assert.deepEqual(api.buildScaleOptions(io, { mode: 'both', ...sizes }), {
    ...io, mode: 'both', target_width: 1024, target_height: 768, down_target_width: 512, down_target_height: 384,
  });
  assert.equal(api.buildScaleOptions(io, { mode: 'sideways' }).mode, 'upscale');
});

test('numbers are normalized to what the Rust fields accept', () => {
  const crop = api.buildCropOptions(io, { target_width: NaN, target_height: '640', aspect_ratio: 0, crop_top: -5, crop_left: 3.6 });
  assert.equal(crop.target_width, 1024);
  assert.equal(crop.target_height, 640);
  assert.equal(crop.aspect_ratio, 1);
  assert.equal(crop.crop_top, 0);
  assert.equal(crop.crop_left, 4);
  assert.equal(api.buildPerspectiveOptions(io, { intensity: 0.9 }).intensity, 0.5);
  assert.equal(api.buildBlurNoiseOptions(io, { noise_strength: 250 }).noise_strength, 100);
  assert.equal(api.buildBlurNoiseOptions(io, { blur_radius: -1 }).blur_radius, 0);
  assert.equal(api.buildFilterOptions(io, { width: '' }).width, 512);
  for (const options of [crop, api.buildScaleOptions(io, { upscale_width: Infinity })]) {
    assert.ok(Object.values(options).every(value => typeof value !== 'number' || Number.isFinite(value)));
  }
});

test('filter falls back to the input folder when no output is given (delete mode)', () => {
  assert.equal(api.buildFilterOptions({ ...io, output_path: '' }, { action: 'delete' }).output_path, '/in');
  assert.equal(api.buildFilterOptions(io, { action: 'delete' }).output_path, '/out');
});

test('upscale derives gpu_id and the engine default model', () => {
  assert.deepEqual(api.buildUpscaleOptions(io), {
    ...io, engine_id: 'realcugan', model_id: 'models-se', scale: 2, denoise_level: -1, tta: false, gpu_id: 0, tile_size: -1,
  });
  const esrgan = api.buildUpscaleOptions(io, { engine_id: 'realesrgan', scale: '4', use_gpu: false, denoise_level: 9 });
  assert.equal(esrgan.model_id, 'realesrgan-x4plus');
  assert.equal(esrgan.scale, 4);
  assert.equal(esrgan.gpu_id, -1);
  assert.equal(esrgan.denoise_level, 3);
  assert.equal(api.buildUpscaleOptions(io, { engine_id: 'waifu2x', model_id: 'models-upconv_7_photo' }).model_id, 'models-upconv_7_photo');
});

test('aesthetic and tagger only batch on the GPU', () => {
  assert.equal(api.buildAestheticOptions(io, { use_gpu: false, batch_size: 8 }).batch_size, 1);
  assert.equal(api.buildAestheticOptions(io, { use_gpu: true, batch_size: 200 }).batch_size, 64);
  assert.equal(api.buildAestheticOptions({ input_path: '/in' }).output_path, '');
  assert.equal(api.buildTaggerOptions(io, { use_gpu: false, batch_size: 8 }).batch_size, 1);
  assert.equal(api.buildTaggerOptions(io, { use_gpu: true, batch_size: 8 }).batch_size, 8);
});

test('tagger keeps known categories once and drops removed fields', () => {
  const options = api.buildTaggerOptions(io, { enabled_categories: ['general', 'bogus', 'general', 'rating'], sort_by: 'random' });
  assert.deepEqual(options.enabled_categories, ['general', 'rating']);
  assert.equal(options.sort_by, 'confidence');
  assert.equal('output_path' in options, false);
  assert.equal('also_skip_json' in options, false);
  assert.deepEqual(api.buildTaggerOptions(io).enabled_categories, ['general', 'character']);
});

test('VLM tagging uses the page defaults, omits top_p unless set, and fills prompts by format', () => {
  const llmApi = { api_endpoint: 'http://localhost/v1', api_key: 'k' };
  const options = api.buildLlmTaggerOptions(io, llmApi, { model_name: 'm', output_format: 'json', json_simplified: true });
  const prompts = getDefaultPrompts('json', true);
  assert.equal(options.system_prompt, prompts.sys);
  assert.equal(options.user_prompt, prompts.user);
  assert.equal(options.temperature, 0.2);
  assert.equal(options.top_p, 0);
  assert.equal(options.request_interval_ms, -1);
  assert.equal(options.concurrency, 1);
  assert.equal(api.buildLlmTaggerOptions(io, llmApi, { system_prompt: '' }).system_prompt, '');
  const odd = api.buildLlmTaggerOptions(io, llmApi, { request_interval_ms: -30, concurrency: 0, image_size: NaN, image_detail: 'ultra' });
  assert.equal(odd.request_interval_ms, -1);
  assert.equal(odd.concurrency, 1);
  assert.equal(odd.image_size, 1024);
  assert.equal(odd.image_detail, '');
});

test('rename only sends a seed when shuffling', () => {
  assert.equal(api.buildRenameOptions({ input_path: '/in' }, { shuffle: false, shuffle_seed: 42 }).shuffle_seed, undefined);
  assert.equal(api.buildRenameOptions({ input_path: '/in' }, { shuffle: true, shuffle_seed: 42.7 }).shuffle_seed, 42);
  const options = api.buildRenameOptions({ input_path: '/in' }, { digit_count: 30, start_number: -1 });
  assert.equal(options.digit_count, 10);
  assert.equal(options.start_number, 0);
  assert.equal(options.rename_tags, true);
});

test('bucket options follow the page rules for each mode', () => {
  const legacy = api.buildBucketOptions(io, { no_upscale: false, min_bucket_reso: 320, max_bucket_reso: 1536, drop_last: true });
  assert.equal(legacy.min_bucket_reso, 320);
  assert.equal(legacy.max_bucket_reso, 1536);
  assert.equal(legacy.drop_last, false);
  assert.equal(legacy.dp_min_ar, null);
  const noUpscale = api.buildBucketOptions(io, { no_upscale: true });
  assert.equal(noUpscale.min_bucket_reso, null);
  assert.equal(noUpscale.max_bucket_reso, null);
  const dp = api.buildBucketOptions(io, { bucket_mode: 'diffusion_pipe', no_upscale: false, drop_last: true });
  assert.equal(dp.no_upscale, true);
  assert.equal(dp.min_bucket_reso, null);
  assert.equal(dp.dp_min_ar, 0.5);
  assert.equal(dp.dp_num_ar_buckets, 7);
  assert.equal(dp.drop_last, true);
});
