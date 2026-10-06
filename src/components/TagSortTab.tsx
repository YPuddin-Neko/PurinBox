import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProgressLog from './ProgressLog';
import ProcessButton from './ProcessButton';
import LlmApiPanel from './LlmApiPanel';
import LlmSamplingFields, { LLM_SAMPLING_DEFAULTS, type LlmSampling } from './LlmSamplingFields';
import PromptPanel from './PromptPanel';
import IoPathPanel from './IoPathPanel';
import { useLlmBatchRun } from '../hooks/useLlmBatchRun';
import { TAG_SORT_PROMPT } from '../utils/llmPrompts';
import type { TagSortOptions } from '../api/commandOptions';

export default function TagSortTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [prompt, setPrompt] = useState(TAG_SORT_PROMPT);
  const [sampling, setSampling] = useState<LlmSampling>({ ...LLM_SAMPLING_DEFAULTS, temperature: 0 });
  const run = useLlmBatchRun({ event: 'tag-sort-progress', taskId: 'tag-sort' });
  const { api } = run;

  const handleStart = () => {
    if (!inputPath || !outputPath) return;
    void run.start({
      taskName: t('tagSort.taskName'),
      startLogKey: 'tagSort.startMsg',
      intervalSec: sampling.intervalSec,
      concurrency: sampling.concurrency,
      exec: requestIntervalMs => invoke('start_tag_sorting', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          ...run.apiOptions,
          prompt,
          temperature: sampling.temperature,
          request_interval_ms: requestIntervalMs,
          concurrency: sampling.concurrency,
          top_p: sampling.topP,
        } satisfies TagSortOptions,
      }),
    });
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <IoPathPanel inputIcon={FolderOpen} inputLabel={t('tagSort.inputDir')} inputPlaceholder={t('tagSort.inputPlaceholder')}
          input={inputPath} onInput={setInputPath}
          outputLabel={t('tagSort.outputDir')} outputPlaceholder={t('tagSort.outputPlaceholder')}
          output={outputPath} onOutput={setOutputPath} />
        <LlmApiPanel api={api}>
          <LlmSamplingFields value={sampling} onChange={setSampling} />
        </LlmApiPanel>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <PromptPanel title={t('tagSort.promptTitle')} hint={t('tagSort.promptHint')} value={prompt} onChange={setPrompt} onReset={() => setPrompt(TAG_SORT_PROMPT)} />
        <ProcessButton {...run.buttonProps} onStart={handleStart}
          disabled={!inputPath || !outputPath || !api.ready}
          cancelCommand="cancel_tag_sorting" startText={t('tagSort.startSort')} processingText={t('tagSort.sorting')} />
        <ProgressLog {...run.progressLogProps} />
      </div>
    </div>
  );
}
