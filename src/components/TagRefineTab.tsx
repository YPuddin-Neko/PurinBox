import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Image } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProgressLog from './ProgressLog';
import ProcessButton from './ProcessButton';
import LlmApiPanel from './LlmApiPanel';
import LlmSamplingFields, { LLM_SAMPLING_DEFAULTS, type LlmSampling } from './LlmSamplingFields';
import PromptPanel from './PromptPanel';
import IoPathPanel from './IoPathPanel';
import { useLlmBatchRun } from '../hooks/useLlmBatchRun';
import { TAG_REFINE_PROMPT } from '../utils/llmPrompts';
import type { TagRefineOptions } from '../api/commandOptions';

export default function TagRefineTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [prompt, setPrompt] = useState(TAG_REFINE_PROMPT);
  const [sampling, setSampling] = useState<LlmSampling>({ ...LLM_SAMPLING_DEFAULTS });
  const run = useLlmBatchRun({ event: 'tag-refine-progress', taskId: 'tag-refine' });
  const { api } = run;

  const handleStart = () => {
    if (!inputPath || !outputPath) return;
    void run.start({
      taskName: t('tagRefine.taskName'),
      startLogKey: 'tagRefine.startMsg',
      intervalSec: sampling.intervalSec,
      concurrency: sampling.concurrency,
      exec: requestIntervalMs => invoke('start_tag_refining', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          ...run.apiOptions,
          prompt,
          temperature: sampling.temperature,
          image_size: sampling.imageSize,
          image_detail: sampling.imageDetail,
          request_interval_ms: requestIntervalMs,
          concurrency: sampling.concurrency,
          top_p: sampling.topP,
          recursive,
        } satisfies TagRefineOptions,
      }),
    });
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <IoPathPanel inputIcon={Image} inputLabel={t('tagRefine.inputDir')} inputPlaceholder={t('tagRefine.inputPlaceholder')}
          input={inputPath} onInput={setInputPath} pickImage recursive={recursive} onRecursive={setRecursive}
          outputLabel={t('tagRefine.outputDir')} outputPlaceholder={t('tagRefine.outputPlaceholder')}
          output={outputPath} onOutput={setOutputPath} />
        <LlmApiPanel api={api}>
          <LlmSamplingFields value={sampling} onChange={setSampling} image />
        </LlmApiPanel>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <PromptPanel title={t('tagRefine.promptTitle')} value={prompt} onChange={setPrompt} onReset={() => setPrompt(TAG_REFINE_PROMPT)} />
        <ProcessButton {...run.buttonProps} onStart={handleStart}
          disabled={!inputPath || !outputPath || !api.ready}
          cancelCommand="cancel_tag_refining" startText={t('tagRefine.startRefine')} processingText={t('tagRefine.refining')} />
        <ProgressLog {...run.progressLogProps} />
      </div>
    </div>
  );
}
