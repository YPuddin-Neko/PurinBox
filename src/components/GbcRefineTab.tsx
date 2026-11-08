import { useLocation } from 'react-router-dom';
import Checkbox from './Checkbox';
import { useEffect, useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { Hash } from 'lucide-react';
import DatasetPathPanel from './DatasetPathPanel';
import NumberInput from './ui/NumberInput';
import LlmApiPanel from './LlmApiPanel';
import LlmSamplingFields, { LLM_SAMPLING_DEFAULTS } from './LlmSamplingFields';
import GbcAnalysisFields, { useGbcAnalysisLimits } from './GbcAnalysisFields';
import DeviceToggle from './ui/DeviceToggle';
import ProcessButton from './ProcessButton';
import ProgressLog from './ProgressLog';
import { useLlmBatchRun } from '../hooks/useLlmBatchRun';
import { useGbcOptions } from '../hooks/useGbcOptions';
import { buildLlmTaggerOptions, type GbcRefineOptions } from '../api/commandOptions';

export default function GbcRefineTab() {
  const { t } = useTranslation();
  const id = useId();
  const [path, setPath] = useState('');
  const location = useLocation();
  useEffect(() => { if (location.state?.gbcImage) setPath(location.state.gbcImage); }, [location.key, location.state]);
  const [saveGraph, setSaveGraph] = useState(false);
  const [recursive, setRecursive] = useState(false);
  const [useGpu, setUseGpu] = useState(false);
  const [sampling, setSampling] = useState({ ...LLM_SAMPLING_DEFAULTS });
  const [maxTokens, setMaxTokens] = useState(-1);
  const [limits, setLimits] = useGbcAnalysisLimits();
  const gbc = { ...useGbcOptions(useGpu), ...limits, save_graph: saveGraph };
  const run = useLlmBatchRun({ event: 'gbc-refine-progress', taskId: 'gbc-refine' });
  const start = () => void run.start({ taskName: t('gbc.refineTab'), startLogKey: 'gbc.refineStart', intervalSec: sampling.intervalSec, concurrency: sampling.concurrency,
    exec: requestIntervalMs => invoke('start_llm_tagging', { options: {
      ...buildLlmTaggerOptions({ input_path: path, recursive }, { api_endpoint: run.api.endpoint, api_key: run.api.apiKey }, {
        model_name: run.api.modelName, temperature: sampling.temperature, top_p: sampling.topP,
        image_size: sampling.imageSize, image_detail: sampling.imageDetail, max_tokens: maxTokens,
        request_interval_ms: requestIntervalMs, concurrency: sampling.concurrency,
      }), refine_existing: true, gbc,
    } satisfies GbcRefineOptions }),
  });
  return <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
      <DatasetPathPanel value={path} onChange={setPath} recursive={recursive} onRecursive={setRecursive} />
      <LlmApiPanel api={run.api} equalPresetWidths={false} />
      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('gbc.analysisSettings')}</span><DeviceToggle useGpu={useGpu} onChange={v => { if (!run.processing) setUseGpu(v); }} /></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <GbcAnalysisFields value={limits} onChange={setLimits} disabled={run.processing} />
          <LlmSamplingFields value={sampling} onChange={setSampling} image samplingFirst extra={<div className="form-group" style={{ marginBottom: 0, flex: 1 }}><label className="form-label" htmlFor={`${id}-tokens`} style={{ display: 'flex', gap: 6, alignItems: 'center' }}><Hash size={13} />{t('llmTagger.maxTokens')}</label><NumberInput id={`${id}-tokens`} integer min={-1} max={8192} value={maxTokens} onChange={setMaxTokens} /></div>} />
          <Checkbox checked={saveGraph} onChange={setSaveGraph} disabled={run.processing} label={t('gbc.keepData')} size={14} />
        </div>
      </div>
    </div>
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
      <ProcessButton {...run.buttonProps} onStart={start} disabled={!path || !run.api.ready} cancelCommand="cancel_llm_tagging" startText={t('gbc.refineButton')} processingText={t('gbc.refining')} />
      <ProgressLog {...run.progressLogProps} />
    </div>
  </div>;
}
