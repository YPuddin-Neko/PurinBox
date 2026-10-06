import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Hash, Save, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { errorText, listen } from '../utils/tauriRuntime';
import { Modal } from './Modal';
import ProgressLog from './ProgressLog';
import ProcessButton from './ProcessButton';
import CustomSelect from './CustomSelect';
import TaggerModelSelect from './TaggerModelSelect';
import Checkbox from './Checkbox';
import DatasetPathPanel from './DatasetPathPanel';
import LlmApiPanel from './LlmApiPanel';
import LlmSamplingFields, { LLM_SAMPLING_DEFAULTS, type LlmSampling } from './LlmSamplingFields';
import { TaggerCategoryGrid, ThresholdSliders } from './TaggerControls';
import DeviceToggle from './ui/DeviceToggle';
import { isCancelMessage, useTaskQueue } from './TaskContext';
import { RunIdGate, isCancelledDone, resolveProgressMessage, textsOverlap, useTaskLog, type UnifiedProgressPayload } from '../hooks/useUnifiedTaskLogs';
import { notifyTaggerModelsChanged, useTaggerModels } from '../hooks/useTaggerModels';
import { usePythonEnvEvents } from '../hooks/usePythonEnvEvents';
import { useTaggerDownloadLog } from '../hooks/useTaggerDownloadLog';
import { settledTaskStatus } from '../hooks/useBatchTask';
import { useLlmApiConfig } from '../hooks/useLlmApiConfig';
import { isTaggerCategory, splitOutputFormat, toIntervalMs, type TagOutputChoice } from '../utils/taggerOptions';
import { initialSkipExisting, loadHybridSettings, saveHybridSettings, storedCount, storedNumber } from '../utils/hybridSettings';
import {
  applyHybridPreset, compatibleHybridFormat, defaultHybridPrompt, hybridBuiltinPreset, parseHybridPresets,
  restoreHybridSelection, saveHybridPreset, type HybridPromptPreset, type HybridVlmValues,
} from '../utils/hybridPresets';
import { HYBRID_PROMPT_JSON, HYBRID_PROMPT_TXT, applyTriggerWord } from '../utils/llmPrompts';
import {
  IMAGE_DETAILS, SHORT_REPLY_THRESHOLD, buildTaggerOptions, isOneOf,
  type PrepareHybridTagsOptions, type ProcessResult, type TagRefineOptions,
} from '../api/commandOptions';

type Phase = '' | 'converting' | 'tagging' | 'refining';

const OUTPUT_CHOICES: readonly TagOutputChoice[] = ['txt', 'json', 'json_simplified'];

const MAX_CONCURRENCY = 16;
const U32_MAX = 0xffffffff;

/** 各阶段的取消命令：准备和本地打标共用打标进程 */
const cancelCommandFor = (phase: Phase) => phase === 'refining' ? 'cancel_tag_refining' : 'force_cancel_tagging';

const TRIGGER_WORD_KEY = 'hybrid_trigger_word';
const CUSTOM_PRESETS_KEY = 'hybrid_prompt_presets';

const loadCustomPresets = (): HybridPromptPreset[] => {
  try {
    return parseHybridPresets(localStorage.getItem(CUSTOM_PRESETS_KEY));
  } catch { return []; }
};

const processResult = (value: unknown): Pick<ProcessResult, 'fail_count' | 'warning_count'> | null =>
  typeof value === 'object' && value !== null && typeof (value as ProcessResult).fail_count === 'number' ? value as ProcessResult : null;

export default function HybridTaggerTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);

  // 上次使用的设置整体从 localStorage 回填
  const savedRef = useRef<ReturnType<typeof loadHybridSettings> | undefined>(undefined);
  if (savedRef.current === undefined) savedRef.current = loadHybridSettings();
  const saved = savedRef.current;
  const sv = saved ?? {};
  const [customPresets, setCustomPresets] = useState<HybridPromptPreset[]>(loadCustomPresets);
  const [restored] = useState(() => restoreHybridSelection(sv, customPresets));
  const initialFormat = restored.outputFormat;

  // ── 本地打标 ──
  const { models, selectedModel, setSelectedModel, genTh, setGenTh, charTh, setCharTh,
    enabled: enabledCats, setEnabled: setEnabledCats, cur } = useTaggerModels({
    initialId: sv.modelId, general: sv.genTh ?? 0.35, character: sv.charTh,
    categories: sv.enabledCats?.filter(isTaggerCategory),
  });
  const [useGpu, setUseGpu] = useState(sv.useGpu ?? true);
  const [replaceUnderscore, setReplaceUnderscore] = useState(sv.replaceUnderscore ?? true);
  const [escapeParentheses, setEscapeParentheses] = useState(sv.escapeParentheses ?? false);
  const [preferExisting, setPreferExisting] = useState(sv.preferExisting ?? true);
  const [skipExisting, setSkipExisting] = useState(() => initialSkipExisting(saved));
  const changePreferExisting = (checked: boolean) => {
    setPreferExisting(checked);
    if (!checked) setSkipExisting(false);
  };

  // ── VLM 调优 ──
  const api = useLlmApiConfig({ initialModelName: sv.modelName });
  const [prompt, setPrompt] = useState(restored.prompt);
  const [presetId, setPresetId] = useState(restored.presetId);
  const [shortReplyThreshold, setShortReplyThreshold] = useState(() =>
    storedCount(sv.shortReplyThreshold, SHORT_REPLY_THRESHOLD.default, SHORT_REPLY_THRESHOLD.max));
  const [showSaveModal, setShowSaveModal] = useState(false);
  const [newPresetName, setNewPresetName] = useState('');
  const [triggerWord, setTriggerWord] = useState(() => {
    try { return localStorage.getItem(TRIGGER_WORD_KEY) || ''; } catch { return ''; }
  });
  useEffect(() => {
    try { localStorage.setItem(TRIGGER_WORD_KEY, triggerWord); } catch { /* 配额满等，忽略 */ }
  }, [triggerWord]);
  const [sampling, setSampling] = useState<LlmSampling>(() => ({
    temperature: storedNumber(sv.temperature, 0.3),
    topP: storedNumber(sv.topP, LLM_SAMPLING_DEFAULTS.topP),
    imageSize: storedCount(sv.imageSize, LLM_SAMPLING_DEFAULTS.imageSize, U32_MAX),
    imageDetail: isOneOf(IMAGE_DETAILS, sv.imageDetail) ? sv.imageDetail : LLM_SAMPLING_DEFAULTS.imageDetail,
    concurrency: storedCount(sv.concurrency, LLM_SAMPLING_DEFAULTS.concurrency, MAX_CONCURRENCY),
    intervalSec: storedNumber(sv.intervalSec, LLM_SAMPLING_DEFAULTS.intervalSec),
  }));
  const [outputFormat, setOutputFormat] = useState<TagOutputChoice>(initialFormat);

  // ── 执行状态 ──
  const [processing, setProcessing] = useState(false);
  const [phase, setPhase] = useState<Phase>('');
  const phaseRef = useRef<Phase>('');
  // 用户点过取消：阶段之间就此停下，不再请求 LLM 改写标签文件
  const cancelRequestedRef = useRef(false);
  // 后端报告本轮已取消，及其「已取消: …」汇总（已写进日志）
  const cancelledRef = useRef<{ summary: string } | null>(null);
  // 当前阶段收到过 done 事件
  const phaseDoneRef = useRef(false);
  const taggerGate = useRef(new RunIdGate());
  const refineGate = useRef(new RunIdGate());
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const taskLogs = useTaskLog();
  const { logs, setLogs, appendProgressLog } = taskLogs;
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const { addTask, updateTask } = useTaskQueue();

  // 设置变更即持久化，下次打开原样回填
  useEffect(() => {
    saveHybridSettings({
      modelId: selectedModel, genTh, charTh, useGpu,
      replaceUnderscore, escapeParentheses, preferExisting, skipExisting,
      enabledCats: [...enabledCats],
      modelName: api.modelName,
      temperature: String(sampling.temperature), topP: String(sampling.topP), imageSize: String(sampling.imageSize),
      shortReplyThreshold: String(shortReplyThreshold),
      imageDetail: sampling.imageDetail, concurrency: String(sampling.concurrency), intervalSec: String(sampling.intervalSec),
      outputFormat, presetId, prompt,
    });
  }, [selectedModel, genTh, charTh, useGpu, replaceUnderscore, escapeParentheses, preferExisting, skipExisting,
    enabledCats, api.modelName, sampling, outputFormat, shortReplyThreshold, presetId, prompt]);

  // 准备与本地打标走 tagger-progress，VLM 调优走 tag-refine-progress，统一进日志与进度条
  useEffect(() => {
    let active = true;
    const handler = (gate: RunIdGate, phases: Phase[]) => ({ payload: p }: { payload: UnifiedProgressPayload }) => {
      // 先过运行 ID：别处跑同一命令的轮次也要记下，下一阶段开始时才能把它们当旧轮丢弃
      if (!active || !gate.accept(p.run_id) || !phases.includes(phaseRef.current)) return;
      if (p.total > 0) { setPCur(p.current); setPTot(p.total); }
      if (p.status === 'error') setHasErr(true);
      if (p.status === 'done') phaseDoneRef.current = true;
      const message = resolveProgressMessage(p);
      if (isCancelledDone(p) || (p.status === 'done' && cancelRequestedRef.current && isCancelMessage(p.message))) {
        cancelledRef.current = { summary: message };
      }
      // 各阶段的 done 不是整条流程的终态，终态在 handleStart 收尾时落定
      updateTask('hybrid-tagger', { status: 'running', message, ...(p.total > 0 ? { current: p.current, total: p.total } : {}) });
      if (p.status !== 'processing') appendProgressLog(p);
    };
    const l1 = listen<UnifiedProgressPayload>('tagger-progress', handler(taggerGate.current, ['converting', 'tagging']));
    const l2 = listen<UnifiedProgressPayload>('tag-refine-progress', handler(refineGate.current, ['refining']));
    return () => { active = false; void l1.then(off => off()); void l2.then(off => off()); };
  }, [appendProgressLog, updateTask]);

  usePythonEnvEvents(processing, taskLogs);
  useTaggerDownloadLog(() => phaseRef.current === 'converting' || phaseRef.current === 'tagging', taskLogs);

  const isJson = outputFormat !== 'txt';
  const canStart = !!inputPath && !!selectedModel && api.ready && enabledCats.size > 0;

  // 内置预设随输出格式变化：完整调优在 txt 下只调标签（txt 没有 nl 字段），JSON 下兼补 nl；
  // 归类字段与仅补 nl 依赖 JSON 的字段结构，详细自然语言打标只用于 txt
  const builtin = (id: string, name: string): HybridPromptPreset => ({ ...hybridBuiltinPreset(id, outputFormat)!, name });
  const builtinPresets: HybridPromptPreset[] = [
    builtin('builtin_full', isJson ? t('hybridTagger.presetFull') : t('hybridTagger.presetTagsOnly')),
    ...(isJson ? [
      builtin('builtin_sort', t('hybridTagger.presetSortOnly')),
      builtin('builtin_nl', t('hybridTagger.presetNlOnly')),
    ] : []),
    ...(!isJson ? [builtin('builtin_caption', t('hybridTagger.presetDetailedCaption'))] : []),
  ];
  const allPresets = [...builtinPresets, ...customPresets];
  const isCustomPreset = customPresets.some(p => p.id === presetId);
  const activePreset = allPresets.find(p => p.id === presetId);
  const captionMode = !!activePreset?.captionMode;
  const preserveTags = !!activePreset?.preserveTags;
  const nlOnly = !!activePreset?.nlOnly;

  const vlmValues: HybridVlmValues = { prompt, triggerWord, shortReplyThreshold, sampling, outputFormat, skipExisting };

  const applyPreset = (id: string) => {
    if (processing) return;
    const preset = allPresets.find(p => p.id === id);
    if (!preset) return;
    const next = applyHybridPreset(preset, vlmValues, preferExisting);
    setPresetId(id);
    setPrompt(next.prompt);
    setTriggerWord(next.triggerWord);
    setShortReplyThreshold(next.shortReplyThreshold);
    setSampling(next.sampling);
    setOutputFormat(next.outputFormat);
    setSkipExisting(next.skipExisting);
  };

  const persistPresets = (list: HybridPromptPreset[]) => {
    try {
      localStorage.setItem(CUSTOM_PRESETS_KEY, JSON.stringify(list));
      setCustomPresets(list);
      return true;
    } catch (error) {
      taskLogs.appendCatchError(errorText(error), t('pages.errorPrefix'));
      return false;
    }
  };

  const handleSavePreset = () => {
    const name = newPresetName.trim();
    if (!name || processing) return;
    const existing = customPresets.find(p => p.name === name);
    const id = existing ? existing.id : `u_${Date.now()}`;
    if (!persistPresets(saveHybridPreset(customPresets, name, id, vlmValues, { captionMode, preserveTags, nlOnly }))) return;
    setPresetId(id);
    setShowSaveModal(false);
    setNewPresetName('');
  };

  const handleDeletePreset = () => {
    if (processing) return;
    if (!persistPresets(customPresets.filter(p => p.id !== presetId))) return;
    setPresetId('builtin_full');
    setPrompt(defaultHybridPrompt(outputFormat));
  };

  // 切换输出格式时联动默认提示词（JSON 模式要求标记格式以补写 nl 字段）；
  // 格式与模式兼容时保留用户编辑的提示词。
  const handleFormatChange = (value: string) => {
    if (processing || !isOneOf(OUTPUT_CHOICES, value)) return;
    setOutputFormat(value);
    // 内置和自定义预设都不能把 JSON 专用模式用于 TXT，反之亦然。
    if (activePreset && compatibleHybridFormat(activePreset, value) !== value) {
      setPresetId('builtin_full');
      setPrompt(defaultHybridPrompt(value));
      return;
    }
    if (presetId === 'builtin_full') {
      setPrompt(prev => prev === HYBRID_PROMPT_TXT || prev === HYBRID_PROMPT_JSON ? defaultHybridPrompt(value) : prev);
    }
  };

  const enterPhase = (next: Exclude<Phase, ''>, message: string) => {
    phaseRef.current = next;
    phaseDoneRef.current = false;
    setPhase(next);
    (next === 'refining' ? refineGate : taggerGate).current.begin();
    setPCur(0); setPTot(0);
    taskLogs.appendLog(message, 'info');
    updateTask('hybrid-tagger', { status: 'running', current: 0, total: 0, message });
  };

  const handleStart = async () => {
    if (!canStart || processing || phaseRef.current) return;
    cancelRequestedRef.current = false;
    cancelledRef.current = null;
    let failures = 0;
    let warnings = 0;
    // 任务面板已落定完成、警告或出错
    let settled = false;
    const count = (result: unknown) => {
      const counts = processResult(result);
      failures += counts?.fail_count ?? 0;
      warnings += counts?.warning_count ?? 0;
    };
    const stopped = () => cancelRequestedRef.current || cancelledRef.current !== null;
    // 事件回调随时会写入 cancelledRef，经函数读取，类型才不会被上面的置空收窄成 null
    const cancelSummary = () => cancelledRef.current?.summary;
    setProcessing(true); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    addTask('hybrid-tagger', t('hybridTagger.taskName'));
    taskLogs.setInitialLog();

    try {
      // 复用的已有标签先准备成中间文件；两个选项都开启时有标签的图整张跳过，不用准备
      if (preferExisting && !skipExisting) {
        enterPhase('converting', t('hybridTagger.phaseConverting'));
        count(await invoke('prepare_hybrid_tags', {
          options: {
            input_path: inputPath,
            model_id: selectedModel,
            file_format: isJson ? 'json' : 'txt',
            json_simplified: outputFormat === 'json_simplified',
            recursive,
          } satisfies PrepareHybridTagsOptions,
        }));
        if (stopped()) return;
      }

      // 本地结果写进专用中间文件，正式标签由 VLM 调优完成后写入
      enterPhase('tagging', t('hybridTagger.phaseTagging'));
      count(await invoke('start_tagging', {
        options: buildTaggerOptions({ input_path: inputPath, recursive }, {
          model_id: selectedModel,
          general_threshold: genTh,
          character_threshold: charTh,
          enabled_categories: [...enabledCats],
          use_gpu: useGpu,
          replace_underscore: replaceUnderscore,
          ...splitOutputFormat(outputFormat),
          escape_parentheses: escapeParentheses,
          existing_tags_action: preferExisting ? 'skip' : 'overwrite',
          hybrid_mode: true,
        }),
      }));
      if (stopped()) return;

      // LLM 二次确认与调优（就地更新标签文件）
      enterPhase('refining', t('hybridTagger.phaseRefining'));
      count(await invoke('start_tag_refining', {
        options: {
          input_path: inputPath,
          output_path: inputPath,
          api_endpoint: api.endpoint,
          api_key: api.apiKey,
          model_name: api.modelName,
          prompt: applyTriggerWord(prompt, triggerWord),
          temperature: sampling.temperature,
          image_size: sampling.imageSize,
          image_detail: sampling.imageDetail,
          top_p: sampling.topP,
          short_reply_threshold: shortReplyThreshold,
          request_interval_ms: toIntervalMs(sampling.intervalSec),
          concurrency: sampling.concurrency,
          recursive,
          // JSON 写回：回复带字段分段时按 LLM 的归属重排 count/appearance/tags/environment，
          // 没有分段时差量写回（保留原字段归属，只应用增删）
          file_format: isJson ? 'json' : 'txt',
          // 自然语言打标：LLM 回复整段写入 txt，不做标签解析（仅 txt 有意义）
          caption_mode: !isJson && captionMode,
          // 触发词：txt 强制置于开头（标签/自然语言都是），JSON 追加进 artist 字段
          trigger_word: triggerWord,
          // 只归类不增删：标签集合由后端保证恒定（仅 JSON 有字段结构）
          preserve_tags: isJson && preserveTags,
          nl_only: isJson && nlOnly,
          hybrid_mode: true,
          skip_existing_labels: preferExisting && skipExisting,
        } satisfies TagRefineOptions,
      }));
      // 与批处理页同一规则：命令可能先于 done 事件返回，点过取消又没收到这一阶段的 done 时按取消收尾
      const status = settledTaskStatus({
        cancelled: cancelledRef.current !== null, cancelRequested: cancelRequestedRef.current,
        doneSeen: phaseDoneRef.current, failed: failures > 0 || warnings > 0,
      });
      if (status === 'cancelled') return;

      settled = true;
      if (status === 'warning') {
        const message = warnings > 0
          ? t('hybridTagger.doneWithWarnings', { warnings, failures })
          : t('hybridTagger.doneWithFailures', { n: failures });
        setHasErr(true);
        taskLogs.appendLog(message, 'warning');
        updateTask('hybrid-tagger', { status: 'warning', message });
      } else {
        taskLogs.appendLog(t('hybridTagger.allDone'), 'success');
        updateTask('hybrid-tagger', { status: 'done', message: t('hybridTagger.allDone') });
      }
    } catch (error: unknown) {
      const text = errorText(error);
      if (cancelRequestedRef.current && isCancelMessage(text)) {
        if (!textsOverlap(cancelSummary(), text)) taskLogs.appendLog(text, 'warning');
        cancelledRef.current ??= { summary: text };
      } else {
        settled = true;
        taskLogs.appendCatchError(text, t('pages.errorPrefix'));
        setHasErr(true);
        updateTask('hybrid-tagger', { status: 'error', message: text });
      }
    } finally {
      if (!settled) {
        // 后端的取消汇总没在命令返回前送到（晚到的事件不再接收），补一条终态日志
        const summary = cancelSummary();
        if (summary === undefined) taskLogs.appendLog(t('header.cancelled'), 'warning');
        updateTask('hybrid-tagger', summary ? { status: 'cancelled', message: summary } : { status: 'cancelled' });
      }
      setIsDone(true);
      setProcessing(false);
      phaseRef.current = '';
      setPhase('');
      notifyTaggerModelsChanged();
    }
  };

  const clearLogs = () => { setLogs([]); setIsDone(false); setHasErr(false); };
  const cancelCommand = cancelCommandFor(phase);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
      <DatasetPathPanel value={inputPath} onChange={setInputPath} recursive={recursive} onRecursive={setRecursive} />

      {/* 本地模型 | LLM 模型 —— 一行两块 */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)', alignItems: 'stretch' }}>
        <div className="tool-panel" style={{ marginBottom: 0 }}>
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('hybridTagger.localPhase')}</span>
            <div style={{ display: 'flex', gap: 4 }}>
              <DeviceToggle useGpu={useGpu} onChange={setUseGpu} />
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <TaggerModelSelect models={models} value={selectedModel} onChange={setSelectedModel}
              formatLabel={m => m.name + (m.is_downloaded ? ' ✓' : ' ⬇')} />
            {cur?.requires_token && <div style={{ fontSize: 11, color: 'var(--color-warning)' }}>{t('aiTagger.requiresToken')}</div>}
            <TaggerCategoryGrid enabled={enabledCats} onChange={setEnabledCats} supported={cur?.supported_categories} />
            <ThresholdSliders general={genTh} character={charTh} onGeneral={setGenTh} onCharacter={setCharTh} />
            <div style={{ display: 'flex', gap: 'var(--space-4)', flexWrap: 'wrap' }}>
              <Checkbox checked={replaceUnderscore} onChange={setReplaceUnderscore} label={t('aiTagger.replaceUnderscore')} size={14} />
              <span title={t('aiTagger.escapeParenthesesTip')}>
                <Checkbox checked={escapeParentheses} onChange={setEscapeParentheses} label={t('aiTagger.escapeParentheses')} size={14} />
              </span>
              <span title={t('hybridTagger.preferExistingTip')}>
                <Checkbox checked={preferExisting} onChange={changePreferExisting} disabled={processing} label={t('hybridTagger.preferExisting')} size={14} />
              </span>
            </div>
          </div>
        </div>

        <LlmApiPanel api={api} compact equalPresetWidths={false} style={{ marginBottom: 0 }} />
      </div>

      {/* 调优设置聚合：提示词 + 参数 + 输出格式 */}
      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('hybridTagger.llmPhase')}</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <CustomSelect compact value={presetId} onChange={applyPreset}
              options={allPresets.map(p => ({ value: p.id, label: p.name }))} style={{ width: 190 }} />
            <button className="btn btn-ghost btn-sm" style={{ fontSize: 10, padding: '4px 6px' }}
              title={t('hybridTagger.savePreset')}
              onClick={() => { setNewPresetName(isCustomPreset ? (activePreset?.name || '') : ''); setShowSaveModal(true); }}>
              <Save style={{ width: 13, height: 13 }} />
            </button>
            {/* 常驻显示，内置预设时置灰——藏起来会让人以为没有删除功能 */}
            <button className="btn btn-ghost btn-sm" disabled={!isCustomPreset}
              style={{ fontSize: 10, padding: '4px 6px', color: isCustomPreset ? '#f87171' : undefined, opacity: isCustomPreset ? 1 : 0.35 }}
              title={t('hybridTagger.deletePreset')} onClick={handleDeletePreset}>
              <Trash2 style={{ width: 13, height: 13 }} />
            </button>
            <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }}
              onClick={() => applyPreset(presetId)}>{t('llmApi.resetDefault')}</button>
          </div>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 300px', gap: 'var(--space-4)', alignItems: 'stretch' }}>
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            <label className="form-label" htmlFor="hybrid-prompt">{t('hybridTagger.promptLabel')}</label>
            <textarea id="hybrid-prompt" className="form-input" value={prompt} onChange={e => setPrompt(e.target.value)}
              style={{ fontSize: 11, fontFamily: 'monospace', lineHeight: 1.6, resize: 'vertical', flex: 1, minHeight: 200 }} />
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <LlmSamplingFields layout="compact" image maxConcurrency={MAX_CONCURRENCY} value={sampling} onChange={setSampling}
              shortReplyWarning={{ value: shortReplyThreshold, onChange: setShortReplyThreshold }} extra={
              <div style={{ flex: 1, minWidth: 0 }}>
                <label className="form-label">{t('hybridTagger.outputFormat')}</label>
                <CustomSelect
                  value={outputFormat}
                  onChange={handleFormatChange}
                  options={[
                    { value: 'txt', label: t('hybridTagger.formatTxt') },
                    { value: 'json', label: t('hybridTagger.formatJson') },
                    { value: 'json_simplified', label: t('hybridTagger.formatJsonSimplified') },
                  ]}
                />
              </div>
            } />
            <div style={{ display: 'flex', alignItems: 'flex-end', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
              <div style={{ flex: '1 1 100px', minWidth: 0 }}>
                <label htmlFor="hybrid-trigger-word" className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                  <Hash style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /> {t('hybridTagger.triggerWord')}
                </label>
                <input id="hybrid-trigger-word" className="form-input" autoComplete="off" value={triggerWord}
                  onChange={e => setTriggerWord(e.target.value)} />
              </div>
              <span title={preferExisting ? undefined : t('hybridTagger.skipExistingRequiresReuse')} style={{ display: 'flex', alignItems: 'center', minHeight: 34 }}>
                <Checkbox checked={skipExisting} onChange={setSkipExisting} disabled={processing || !preferExisting} size={14} label={t('hybridTagger.skipExisting')} />
              </span>
            </div>
          </div>
        </div>
      </div>

      {/* 操作 + 进度 + 日志 */}
      <ProcessButton
        processing={processing}
        onStart={handleStart}
        disabled={!canStart}
        cancelCommand={cancelCommand}
        startText={t('hybridTagger.startText')}
        processingText={
          phase === 'converting' ? t('hybridTagger.phaseShortConverting')
          : phase === 'tagging' ? t('hybridTagger.phaseShortTagging')
          : phase === 'refining' ? t('hybridTagger.phaseShortRefining')
          : t('pages.processing')
        }
        onCancelLog={message => {
          cancelRequestedRef.current = true;
          taskLogs.appendLog(message, 'warning');
          // 阶段刚切换、按钮还没重新渲染时，按钮带的仍是上一阶段的取消命令
          const actual = cancelCommandFor(phaseRef.current);
          if (actual !== cancelCommand) void invoke(actual).catch(() => {});
        }}
      />
      <ProgressLog
        current={pCur}
        total={pTot}
        logs={logs}
        isDone={isDone}
        hasError={hasErr}
        onClearLogs={clearLogs}
      />

      <Modal open={showSaveModal} onClose={() => setShowSaveModal(false)} title={t('hybridTagger.savePreset')}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <input className="form-input" autoFocus value={newPresetName}
            placeholder={t('hybridTagger.presetNamePlaceholder')}
            onChange={e => setNewPresetName(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') handleSavePreset(); }} />
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 'var(--space-2)' }}>
            <button className="btn btn-secondary btn-sm" onClick={() => setShowSaveModal(false)}>{t('common.cancel')}</button>
            <button className="btn btn-primary btn-sm" disabled={!newPresetName.trim()} onClick={handleSavePreset}>{t('common.save')}</button>
          </div>
        </div>
      </Modal>
    </div>
  );
}
