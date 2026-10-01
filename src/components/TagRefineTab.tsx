import { useLlmApiConfig } from '../hooks/useLlmApiConfig';
import LlmApiPanel from './LlmApiPanel';
import { toIntervalMs, toThreads, toImageSize } from '../utils/taggerOptions';
import { IMAGE_DETAILS, isOneOf, type ImageDetail, type TagRefineOptions } from '../api/commandOptions';
import { useBatchTask } from '../hooks/useBatchTask';
import { useBatchRunStats } from '../hooks/useBatchRunStats';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, FolderOutput, MessageSquare, Timer, Layers, Image, Thermometer, Focus } from 'lucide-react';
import ProgressLog from './ProgressLog';
import { useTranslation } from 'react-i18next';
import CustomSelect from '../components/CustomSelect';
import ProcessButton from '../components/ProcessButton';
import RecursiveScanToggle from './RecursiveScanToggle';
import InputPathPickerButton from './InputPathPickerButton';
import { IMAGE_DETAIL_OPTIONS } from '../utils/imageDetail';

const defaultRefinePrompt = `You are an expert anime image tagger. You will receive an image and its existing tags.

Your task:
1. Compare the image content with the existing tags
2. Fix incorrect tags (e.g. wrong hair color, wrong clothing)
3. Add important missing tags that are clearly visible in the image
4. Remove tags that don't match the image at all
5. Keep the tag format consistent (lowercase, underscores)

Rules:
- Only make changes you are confident about
- Preserve tags that are correct
- Return ONLY the refined tags, comma-separated
- Do NOT add explanations

Existing tags: {tags}

Refined tags:`;

export default function TagRefineTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [prompt, setPrompt] = useState(defaultRefinePrompt);
  const [intervalSec, setIntervalSec] = useState('-1');
  const [concurrency, setConcurrency] = useState('1');
  const [temperature, setTemperature] = useState('0.3');
  const [topP, setTopP] = useState('0');
  const [imageSize, setImageSize] = useState('1024');
  const [imageDetail, setImageDetail] = useState<ImageDetail>('');
  const api = useLlmApiConfig();
  const stats = useBatchRunStats();
  const task = useBatchTask({ event: 'tag-refine-progress', taskId: 'tag-refine',
    onEvent: stats.onEvent, logStatus: p => p.status === 'warning' ? 'warning' : undefined });
  const handleStart = async () => {
    if (task.processing || !inputPath || !outputPath || !api.ready) return;
    stats.reset();
    const intervalMs = toIntervalMs(intervalSec);
    const threads = toThreads(concurrency);
    await task.run({
      taskName: t('tagRefine.taskName'),
      startLog: t('tagRefine.startMsg', { model: api.modelName, api: api.endpoint, threads, interval: intervalMs < 0 ? t('tagSort.noInterval') : `${intervalMs / 1000}s` }),
      exec: () => invoke('start_tag_refining', { options: {
          input_path: inputPath,
          output_path: outputPath,
          api_endpoint: api.endpoint,
          api_key: api.apiKey,
          model_name: api.modelName,
          prompt: prompt,
          temperature: Number(temperature),
          image_size: toImageSize(imageSize),
          image_detail: imageDetail,
          request_interval_ms: intervalMs,
          concurrency: threads,
          top_p: Number(topP),
          recursive,
      } satisfies TagRefineOptions }),
    });
    stats.summarize(task.logger);
  };

  return (
    <div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
        {/* 左栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('pages.pathSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <div className="form-label-row">
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Image style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.inputDir')}
                  </label>
                  <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
                </div>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('tagRefine.inputPlaceholder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <InputPathPickerButton onSelect={setInputPath} />
                </div>
              </div>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOutput style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.outputDir')}
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('tagRefine.outputPlaceholder')} value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setOutputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          <LlmApiPanel api={api}>
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Timer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.interval')}</label>
                  <input className="form-input" type="number" min="-1" max="120" step="1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)} title={t('tagSort.intervalTip')} />
                </div>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Layers style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.concurrency')}</label>
                  <input className="form-input" type="number" min="1" max="32" step="1" value={concurrency} onChange={e => setConcurrency(e.target.value)} />
                </div>
              </div>
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.temperature')}</span>
                    <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{temperature}</span>
                  </label>
                  <input type="range" min="0" max="2" step="0.05" value={temperature} onChange={e => setTemperature(e.target.value)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
                </div>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <span>Top P</span>
                    <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{topP}</span>
                  </label>
                  <input type="range" min="0" max="1" step="0.05" value={topP} onChange={e => setTopP(e.target.value)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
                </div>
              </div>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Image style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.imageSize')}
                </label>
                <input className="form-input" type="number" min="256" max="4096" step="128" value={imageSize} onChange={e => setImageSize(e.target.value)} style={{ width: 120 }} />
              </div>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Focus style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.imageDetail')}</label>
                <CustomSelect value={imageDetail} onChange={v => { if (isOneOf(IMAGE_DETAILS, v)) setImageDetail(v); }} options={IMAGE_DETAIL_OPTIONS(t)} style={{ width: 160 }} />
              </div>
          </LlmApiPanel>
        </div>

        {/* 右栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 提示词 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('tagRefine.promptTitle')}</span>
              <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={() => setPrompt(defaultRefinePrompt)}>{t('tagSort.resetDefault')}</button>
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> Prompt
              </label>
              <textarea className="form-input" rows={8} value={prompt} onChange={e => setPrompt(e.target.value)}
                style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
          </div>

          {/* 操作按钮 */}
          <ProcessButton {...task.buttonProps} onStart={handleStart}
            disabled={!inputPath || !outputPath || !api.ready}
            cancelCommand="cancel_tag_refining" startText={t('tagRefine.startRefine')} processingText={t('tagRefine.refining')} />

          {/* 进度日志 */}
          <ProgressLog {...task.progressLogProps} headerExtra={stats.headerExtra}
            onClearLogs={() => { task.progressLogProps.onClearLogs(); stats.reset(); }} />
        </div>
      </div>
    </div>
  );
}
