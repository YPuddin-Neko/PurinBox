import ProcessButton from './ProcessButton';
import { useLlmApiConfig } from '../hooks/useLlmApiConfig';
import LlmApiPanel from './LlmApiPanel';
import { toIntervalMs, toThreads, toImageSize } from '../utils/taggerOptions';
import { IMAGE_DETAILS, isOneOf, type ImageDetail, type LlmTaggerOptions } from '../api/commandOptions';
import { useBatchTask } from '../hooks/useBatchTask';
import { useBatchRunStats } from '../hooks/useBatchRunStats';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MessageSquare, Thermometer, Hash, ImageIcon, Timer, Layers, Focus } from 'lucide-react';
import ProgressLog from './ProgressLog';
import CustomSelect from './CustomSelect';
import { useTranslation } from 'react-i18next';
import { IMAGE_DETAIL_OPTIONS } from '../utils/imageDetail';
import InputPathPickerButton from './InputPathPickerButton';
import RecursiveScanToggle from './RecursiveScanToggle';

import { getDefaultPrompts } from '../utils/llmPrompts';

export default function LlmTaggerTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [temperature, setTemperature] = useState('0.2');
  const [maxTokens, setMaxTokens] = useState('-1');
  const [sysPrompt, setSysPrompt] = useState(() => getDefaultPrompts('txt', false).sys);
  const [userPrompt, setUserPrompt] = useState(() => getDefaultPrompts('txt', false).user);
  const [imageSize, setImageSize] = useState('1024');
  const [imageDetail, setImageDetail] = useState<ImageDetail>('');
  const [topP, setTopP] = useState('0');
  const [skipExisting, setSkipExisting] = useState(false);
  const [outputFormat, setOutputFormat] = useState<'txt' | 'json'>('txt');
  const [jsonSimplified, setJsonSimplified] = useState(()=>localStorage.getItem('tagger_json_simplified')==='true');
  const [intervalSec, setIntervalSec] = useState('-1');
  const [concurrency, setConcurrency] = useState('1');
  const [recursive, setRecursive] = useState(false);
  const api = useLlmApiConfig();
  const stats = useBatchRunStats();
  const task = useBatchTask({ event: 'llm-tagger-progress', taskId: 'llm-tagger',
    onEvent: stats.onEvent, logStatus: p => p.status === 'warning' ? 'warning' : undefined });
  const handleStart = async () => {
    if (task.processing || !inputPath || !api.ready) return;
    stats.reset();
    const intervalMs = toIntervalMs(intervalSec);
    const threads = toThreads(concurrency);
    await task.run({
      taskName: t('llmTagger.taskName'),
      startLog: t('llmTagger.startMsg', { model: api.modelName, api: api.endpoint, threads, interval: intervalMs < 0 ? t('tagSort.noInterval') : `${intervalMs / 1000}s` }),
      exec: () => invoke('start_llm_tagging', { options: {
          input_path: inputPath, api_endpoint: api.endpoint, api_key: api.apiKey, model_name: api.modelName,
          system_prompt: sysPrompt, user_prompt: userPrompt,
          temperature: Number(temperature), max_tokens: parseInt(maxTokens) || -1,
          image_size: toImageSize(imageSize),
          image_detail: imageDetail,
          top_p: Number(topP),
          skip_existing: skipExisting,
          output_format: outputFormat,
          json_simplified: jsonSimplified,
          request_interval_ms: intervalMs,
          concurrency: threads,
          recursive,
      } satisfies LlmTaggerOptions }),
    });
    stats.summarize(task.logger);
  };

  const applyFormatDefaults = (format: 'txt' | 'json', simplified: boolean) => {
    const defaults = [getDefaultPrompts('txt', false), getDefaultPrompts('json', false), getDefaultPrompts('json', true)];
    if (defaults.some(p => p.sys === sysPrompt && p.user === userPrompt)) {
      const next = getDefaultPrompts(format, simplified);
      setSysPrompt(next.sys); setUserPrompt(next.user);
    }
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
      {/* 左栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        {/* 路径 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('llmTagger.datasetPath')}</span>
            <RecursiveScanToggle checked={recursive} onChange={setRecursive} />
          </div>
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <input className="form-input" placeholder={t('llmTagger.selectFolder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
            <InputPathPickerButton onSelect={setInputPath} />
          </div>
        </div>

        <LlmApiPanel api={api} />

        {/* 模型设置 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">{t('llmTagger.modelSettings')}</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.temperature')}</span>
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
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><ImageIcon style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.imageSize')}</label>
                <input className="form-input" type="number" min="256" max="4096" step="64" value={imageSize} onChange={e => setImageSize(e.target.value)} placeholder={t('llmTagger.imageSizePlaceholder')} />
              </div>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Hash style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.maxTokens')}</label>
                <input className="form-input" type="number" min="-1" max="8192" step="1" value={maxTokens} onChange={e => setMaxTokens(e.target.value)} placeholder={t('llmTagger.maxTokensPlaceholder')} />
              </div>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Focus style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.imageDetail')}</label>
                <CustomSelect value={imageDetail} onChange={v => { if (isOneOf(IMAGE_DETAILS, v)) setImageDetail(v); }} options={IMAGE_DETAIL_OPTIONS(t)} />
              </div>
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Timer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.interval')}</label>
                <input className="form-input" type="number" min="-1" max="120" step="1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)}
                  title={t('llmTagger.intervalTip')} />
              </div>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Layers style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.concurrency')}</label>
                <input className="form-input" type="number" min="1" max="32" step="1" value={concurrency} onChange={e => setConcurrency(e.target.value)} />
              </div>
            </div>
            {/* 跳过已有描述 + 输出格式 */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div onClick={() => setSkipExisting(!skipExisting)} style={{
                width: 36, height: 20, borderRadius: 10, cursor: 'pointer', position: 'relative', transition: 'background 0.2s',
                background: skipExisting ? 'var(--color-accent-primary)' : 'var(--color-border)',
              }}>
                <div style={{
                  width: 14, height: 14, borderRadius: '50%', background: '#fff', position: 'absolute', top: 3,
                  left: skipExisting ? 19 : 3, transition: 'left 0.2s',
                }} />
              </div>
              <label className="form-label" style={{ margin: 0, display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }} onClick={() => setSkipExisting(!skipExisting)}>
                {t('llmTagger.skipExisting')}
              </label>
              <div style={{ flex: 1 }} />
              <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 11 }}>
                <span style={{ color: 'var(--color-text-tertiary)', fontWeight: 500 }}>{t('llmTagger.outputFormat')}</span>
                {(['txt', 'json'] as const).map(fmt => (
                  <button key={fmt} onClick={() => {
                    setOutputFormat(fmt);
                    applyFormatDefaults(fmt, jsonSimplified);
                  }} style={{ padding: '2px 10px', borderRadius: 'var(--radius-sm)', border: `1px solid ${outputFormat === fmt ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: outputFormat === fmt ? 'rgba(124,92,252,0.08)' : 'transparent', color: outputFormat === fmt ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', fontSize: 11, fontWeight: 600, cursor: 'pointer' }}>.{fmt}</button>
                ))}
                {(['full', 'simplified'] as const).map(mode => {
                  const isJson = outputFormat === 'json';
                  const isActive = isJson && (mode === 'simplified') === jsonSimplified;
                  return (
                    <button key={mode} disabled={!isJson} onClick={() => {
                      const v = mode === 'simplified';
                      setJsonSimplified(v);
                      localStorage.setItem('tagger_json_simplified', String(v));
                      applyFormatDefaults('json', v);
                    }} style={{
                      padding: '2px 10px', borderRadius: 'var(--radius-sm)',
                      border: `1px solid ${isActive ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                      background: isActive ? 'rgba(124,92,252,0.08)' : 'transparent',
                      color: isActive ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
                      fontSize: 11, fontWeight: 600, cursor: isJson ? 'pointer' : 'not-allowed',
                      opacity: isJson ? 1 : 0.4,
                    }}>{mode === 'full' ? t('llmTagger.fullFormat') : t('llmTagger.simplified')}</button>
                  );
                })}
              </div>
            </div>
            {/* System Prompt */}
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> System Prompt</label>
              <textarea className="form-input" rows={3} value={sysPrompt} onChange={e => setSysPrompt(e.target.value)} style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
            {/* User Prompt */}
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> User Prompt</label>
              <textarea className="form-input" rows={2} value={userPrompt} onChange={e => setUserPrompt(e.target.value)} style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
          </div>
        </div>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <ProcessButton {...task.buttonProps} onStart={handleStart} disabled={!inputPath || !api.ready}
          cancelCommand="cancel_llm_tagging" startText={t('llmTagger.startLlmTag')} processingText={t('llmTagger.tagging')} />
        <ProgressLog {...task.progressLogProps} headerExtra={stats.headerExtra}
          onClearLogs={() => { task.progressLogProps.onClearLogs(); stats.reset(); }} />
      </div>
    </div>
  );
}
