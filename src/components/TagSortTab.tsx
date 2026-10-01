import { useBatchTask } from '../hooks/useBatchTask';
import { useBatchRunStats } from '../hooks/useBatchRunStats';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, FolderOutput, MessageSquare, Timer, Layers, Thermometer } from 'lucide-react';
import ProgressLog from './ProgressLog';
import { useTranslation } from 'react-i18next';
import ProcessButton from '../components/ProcessButton';
import LlmApiPanel from './LlmApiPanel';
import { useLlmApiConfig } from '../hooks/useLlmApiConfig';
import { toIntervalMs, toThreads } from '../utils/taggerOptions';
import type { TagSortOptions } from '../api/commandOptions';

const defaultPrompt = `Please sort the following tags in this order: character count (e.g. 1girl) → character name → series/source → artist → features → clothing → expression details → clothing details → camera angle/perspective → actions → scene/environment → others.

Important rules:
1. Only rearrange the order of existing tags
2. Do NOT add any new tags, do NOT remove any original tags
3. Return ONLY the sorted tags, comma-separated

Tags to sort: {tags}

Sorted tags:`;

export default function TagSortTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [prompt, setPrompt] = useState(defaultPrompt);
  const [intervalSec, setIntervalSec] = useState('-1');
  const [concurrency, setConcurrency] = useState('1');
  const [temperature, setTemperature] = useState('0');
  const [topP, setTopP] = useState('0');
  const api = useLlmApiConfig();
  const stats = useBatchRunStats();
  const task = useBatchTask({ event: 'tag-sort-progress', taskId: 'tag-sort',
    onEvent: stats.onEvent, logStatus: p => p.status === 'warning' ? 'warning' : undefined });
  const handleStart = async () => {
    if (task.processing || !inputPath || !outputPath || !api.ready) return;
    stats.reset();
    const intervalMs = toIntervalMs(intervalSec);
    const threads = toThreads(concurrency);
    await task.run({
      taskName: t('tagSort.taskName'),
      startLog: t('tagSort.startMsg', { model: api.modelName, api: api.endpoint, threads, interval: intervalMs < 0 ? t('tagSort.noInterval') : `${intervalMs / 1000}s` }),
      exec: () => invoke('start_tag_sorting', { options: {
          input_path: inputPath,
          output_path: outputPath,
          api_endpoint: api.endpoint,
          api_key: api.apiKey,
          model_name: api.modelName,
          prompt,
          temperature: Number(temperature),
          request_interval_ms: intervalMs,
          concurrency: threads,
          top_p: Number(topP),
      } satisfies TagSortOptions }),
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
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOpen style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.inputDir')}
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('tagSort.inputPlaceholder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setInputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOutput style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.outputDir')}
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('tagSort.outputPlaceholder')} value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setOutputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          <LlmApiPanel api={api}>
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Timer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.interval')}</label>
                <input className="form-input" type="number" min="-1" max="120" step="1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)}
                  title={t('tagSort.intervalTip')} />
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
                <input type="range" min="0" max="2" step="0.05" value={temperature}
                  onChange={e => setTemperature(e.target.value)}
                  style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
              </div>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <span>Top P</span>
                  <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{topP}</span>
                </label>
                <input type="range" min="0" max="1" step="0.05" value={topP}
                  onChange={e => setTopP(e.target.value)}
                  style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
              </div>
            </div>
          </LlmApiPanel>
        </div>

        {/* 右栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 提示词 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('tagSort.promptTitle')}</span>
              <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={() => setPrompt(defaultPrompt)}>{t('tagSort.resetDefault')}</button>
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
            cancelCommand="cancel_tag_sorting" startText={t('tagSort.startSort')} processingText={t('tagSort.sorting')} />

          {/* 自定义进度日志 */}
          <ProgressLog {...task.progressLogProps} headerExtra={stats.headerExtra}
            onClearLogs={() => { task.progressLogProps.onClearLogs(); stats.reset(); }} />
        </div>
      </div>
    </div>
  );
}
