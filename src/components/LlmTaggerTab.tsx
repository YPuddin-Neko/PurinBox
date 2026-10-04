import { useEffect, useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Hash, MessageSquare } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProcessButton from './ProcessButton';
import ProgressLog from './ProgressLog';
import LlmApiPanel from './LlmApiPanel';
import LlmSamplingFields, { LLM_SAMPLING_DEFAULTS, type LlmSampling } from './LlmSamplingFields';
import DatasetPathPanel from './DatasetPathPanel';
import NumberInput from './ui/NumberInput';
import Switch from './ui/Switch';
import { useLlmBatchRun } from '../hooks/useLlmBatchRun';
import { useTaggerJsonSimplified } from '../hooks/useTaggerJsonSimplified';
import { getDefaultPrompts, switchDefaultPrompts } from '../utils/llmPrompts';
import { LLM_TAGGER_DEFAULTS, buildLlmTaggerOptions, type TagFileFormat } from '../api/commandOptions';

const ICON = { width: 13, height: 13, color: 'var(--color-text-tertiary)' } as const;

export default function LlmTaggerTab() {
  const { t } = useTranslation();
  const ids = useId();
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [sampling, setSampling] = useState<LlmSampling>({ ...LLM_SAMPLING_DEFAULTS, temperature: LLM_TAGGER_DEFAULTS.temperature });
  const [maxTokens, setMaxTokens] = useState(LLM_TAGGER_DEFAULTS.max_tokens);
  const [skipExisting, setSkipExisting] = useState(false);
  const [outputFormat, setOutputFormat] = useState<TagFileFormat>('txt');
  const [jsonSimplified, setJsonSimplified] = useTaggerJsonSimplified();
  const [prompts, setPrompts] = useState(() => getDefaultPrompts('txt', false));
  const run = useLlmBatchRun({ event: 'llm-tagger-progress', taskId: 'llm-tagger' });
  const { api } = run;

  // 输出格式变化（包括在 Tagger 打标子页改了简化格式）时，仍是默认值的提示词换成新格式的默认值
  useEffect(() => {
    setPrompts(prev => {
      const next = switchDefaultPrompts(prev, outputFormat, jsonSimplified);
      return next.sys === prev.sys && next.user === prev.user ? prev : next;
    });
  }, [outputFormat, jsonSimplified]);

  const handleStart = () => {
    if (!inputPath) return;
    void run.start({
      taskName: t('llmTagger.taskName'),
      startLogKey: 'llmTagger.startMsg',
      intervalSec: sampling.intervalSec,
      concurrency: sampling.concurrency,
      exec: requestIntervalMs => invoke('start_llm_tagging', {
        options: buildLlmTaggerOptions({ input_path: inputPath, recursive }, { api_endpoint: api.endpoint, api_key: api.apiKey }, {
          model_name: api.modelName,
          system_prompt: prompts.sys,
          user_prompt: prompts.user,
          temperature: sampling.temperature,
          max_tokens: maxTokens,
          image_size: sampling.imageSize,
          image_detail: sampling.imageDetail,
          top_p: sampling.topP,
          skip_existing: skipExisting,
          output_format: outputFormat,
          json_simplified: jsonSimplified,
          request_interval_ms: requestIntervalMs,
          concurrency: sampling.concurrency,
        }),
      }),
    });
  };

  const formatButton = (active: boolean, enabled = true) => ({
    padding: '2px 10px', borderRadius: 'var(--radius-sm)',
    border: `1px solid ${active ? 'var(--color-border-active)' : 'var(--color-border)'}`,
    background: active ? 'rgba(124,92,252,0.08)' : 'transparent',
    color: active ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
    fontSize: 11, fontWeight: 600, cursor: enabled ? 'pointer' : 'not-allowed', opacity: enabled ? 1 : 0.4,
  });

  const maxTokensField = (
    <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
      <label className="form-label" htmlFor={`${ids}-max-tokens`} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <Hash style={ICON} /> {t('llmTagger.maxTokens')}
      </label>
      <NumberInput id={`${ids}-max-tokens`} integer min={-1} max={8192} step={1} fallback={-1} value={maxTokens} onChange={setMaxTokens}
        placeholder={t('llmTagger.maxTokensPlaceholder')} />
    </div>
  );

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <DatasetPathPanel value={inputPath} onChange={setInputPath} recursive={recursive} onRecursive={setRecursive} />

        <LlmApiPanel api={api} />

        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">{t('llmTagger.modelSettings')}</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <LlmSamplingFields value={sampling} onChange={setSampling} image samplingFirst extra={maxTokensField}
              imageSizePlaceholder={t('llmTagger.imageSizePlaceholder')} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Switch id={`${ids}-skip`} checked={skipExisting} onChange={setSkipExisting} />
              <label className="form-label" htmlFor={`${ids}-skip`} style={{ margin: 0, display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
                {t('llmTagger.skipExisting')}
              </label>
              <div style={{ flex: 1 }} />
              <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 11 }}>
                <span style={{ color: 'var(--color-text-tertiary)', fontWeight: 500 }}>{t('llmTagger.outputFormat')}</span>
                {(['txt', 'json'] as const).map(format => (
                  <button key={format} onClick={() => setOutputFormat(format)} style={formatButton(outputFormat === format)}>.{format}</button>
                ))}
                {(['full', 'simplified'] as const).map(mode => {
                  const isJson = outputFormat === 'json';
                  return (
                    <button key={mode} disabled={!isJson} onClick={() => setJsonSimplified(mode === 'simplified')}
                      style={formatButton(isJson && (mode === 'simplified') === jsonSimplified, isJson)}>
                      {mode === 'full' ? t('llmTagger.fullFormat') : t('llmTagger.simplified')}
                    </button>
                  );
                })}
              </div>
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" htmlFor={`${ids}-sys`} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <MessageSquare style={ICON} /> System Prompt
              </label>
              <textarea id={`${ids}-sys`} className="form-input" rows={3} value={prompts.sys}
                onChange={e => { const sys = e.target.value; setPrompts(prev => ({ ...prev, sys })); }}
                style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" htmlFor={`${ids}-user`} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <MessageSquare style={ICON} /> User Prompt
              </label>
              <textarea id={`${ids}-user`} className="form-input" rows={2} value={prompts.user}
                onChange={e => { const user = e.target.value; setPrompts(prev => ({ ...prev, user })); }}
                style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
          </div>
        </div>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <ProcessButton {...run.buttonProps} onStart={handleStart} disabled={!inputPath || !api.ready}
          cancelCommand="cancel_llm_tagging" startText={t('llmTagger.startLlmTag')} processingText={t('llmTagger.tagging')} />
        <ProgressLog {...run.progressLogProps} />
      </div>
    </div>
  );
}
