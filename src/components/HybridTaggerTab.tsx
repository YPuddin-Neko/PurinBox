import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';
import {
  Key, Bot, RefreshCw, Loader2, Eye, EyeOff, Save, Thermometer, Image as ImageIcon,
} from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';
import ProcessButton from './ProcessButton';
import InputPathPickerButton from './InputPathPickerButton';
import CustomSelect from './CustomSelect';
import Checkbox from './Checkbox';
import { useTaskQueue } from './TaskContext';
import { useUnifiedTaskLogs } from '../hooks/useUnifiedTaskLogs';
import { useTranslation } from 'react-i18next';

interface ModelInfo { id: string; name: string; description: string; input_size: number; is_builtin: boolean; is_downloaded: boolean; repo_id: string; input_format: string; supported_categories: string[]; }
interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; i18n_key?: string; i18n_params?: Record<string, string>; }

type Phase = '' | 'tagging' | 'refining';

/** 默认调优提示词：补充缺失 / 删除错误 / 修复不准确（{tags} 由后端替换为该图现有标签） */
const defaultHybridPrompt = `You are an expert anime image tagger. You will receive an image and its existing tags produced by a local tagger model.

Your task:
1. Compare the image content with the existing tags
2. Fix incorrect tags (e.g. wrong hair color, wrong clothing, wrong subject count)
3. Add important missing tags that are clearly visible in the image
4. Remove tags that do not match the image at all
5. Keep the tag format consistent (lowercase danbooru-style tags)

Rules:
- Only make changes you are confident about
- Preserve tags that are correct
- Return ONLY the refined tags, comma-separated
- Do NOT add explanations

Existing tags: {tags}

Refined tags:`;

export default function HybridTaggerTab() {
  const { t } = useTranslation();
  const cats = [
    { key: 'general', label: t('aiTagger.catGeneral'), default: true },
    { key: 'character', label: t('aiTagger.catCharacter'), default: true },
    { key: 'rating', label: t('aiTagger.catRating'), default: false },
    { key: 'artist', label: t('aiTagger.catArtist'), default: false },
    { key: 'copyright', label: t('aiTagger.catCopyright'), default: false },
    { key: 'meta', label: t('aiTagger.catMeta'), default: false },
    { key: 'quality', label: t('aiTagger.catQuality'), default: false },
    { key: 'model', label: t('aiTagger.catModel'), default: false },
  ];

  // ── 路径 ──
  const [inputPath, setInputPath] = useState('');
  const [recursive, setRecursive] = useState(false);

  // ── 本地打标 ──
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('');
  const [genTh, setGenTh] = useState(0.35);
  const [charTh, setCharTh] = useState(0.85);
  const [useGpu, setUseGpu] = useState(true);
  const [replaceUnderscore, setReplaceUnderscore] = useState(true);
  const [escapeParentheses, setEscapeParentheses] = useState(false);
  const [enabledCats, setEnabledCats] = useState<Set<string>>(new Set(cats.filter(c => c.default).map(c => c.key)));

  // ── LLM 调优 ──
  const [preset, setPreset] = useState('openai');
  const [customEndpoint, setCustomEndpoint] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [modelName, setModelName] = useState('');
  const [modelList, setModelList] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchMsg, setFetchMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [saveMsg, setSaveMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [prompt, setPrompt] = useState(defaultHybridPrompt);
  const [temperature, setTemperature] = useState('0.3');
  const [topP, setTopP] = useState('0');
  const [imageSize, setImageSize] = useState('1024');
  const [concurrency, setConcurrency] = useState('1');
  const [intervalSec, setIntervalSec] = useState('-1');

  // ── 输出 ──
  const [outputFormat, setOutputFormat] = useState<'txt' | 'json' | 'json_simplified'>('txt');

  // ── 执行状态 ──
  const [processing, setProcessing] = useState(false);
  const [phase, setPhase] = useState<Phase>('');
  const phaseRef = useRef<Phase>('');
  phaseRef.current = phase;
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const taskLogs = useUnifiedTaskLogs(setLogs);
  const { addTask, updateTask } = useTaskQueue();

  const PRESETS: Record<string, { label: string; url: string }> = {
    openai: { label: 'OpenAI', url: 'https://api.openai.com/v1/' },
    gemini: { label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai/' },
    deepseek: { label: 'DeepSeek', url: 'https://api.deepseek.com/v1/' },
    custom: { label: t('llmTagger.customLabel'), url: '' },
  };
  const endpoint = preset === 'custom' ? customEndpoint : (PRESETS[preset]?.url || '');

  // 模型列表 + 已保存的 API 配置
  useEffect(() => {
    invoke<ModelInfo[]>('get_tagger_models').then(l => {
      setModels(l);
      const firstDownloaded = l.find(m => m.is_downloaded);
      if (!selectedModel) setSelectedModel((firstDownloaded || l[0])?.id || '');
    }).catch(() => {});
    invoke<{ preset: string; custom_endpoint: string; api_keys: Record<string, string> }>('load_api_config').then((cfg) => {
      const known = ['openai', 'gemini', 'deepseek', 'custom'];
      if (cfg.preset) setPreset(known.includes(cfg.preset) ? cfg.preset : 'custom');
      if (cfg.custom_endpoint) setCustomEndpoint(cfg.custom_endpoint);
      const key = cfg.api_keys?.[cfg.preset] || '';
      if (key) setApiKey(key);
    }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 模型切换时移除其不支持的类别选择（WD 系列只支持部分类别）
  useEffect(() => {
    const curModel = models.find(m => m.id === selectedModel);
    if (!curModel) return;
    const supported = new Set(curModel.supported_categories);
    setEnabledCats(prev => {
      const next = new Set([...prev].filter(k => supported.has(k)));
      if (next.size === 0) cats.filter(c => c.default && supported.has(c.key)).forEach(c => next.add(c.key));
      if (next.size === 0 && curModel.supported_categories.length > 0) next.add(curModel.supported_categories[0]);
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedModel, models]);

  // 本地打标/转换走 tagger-progress，LLM 调优走 tag-refine-progress，统一进日志与进度条
  useEffect(() => {
    let cancelled = false;
    const handler = (expectPhases: Phase[]) => (e: { payload: ProgressPayload }) => {
      if (cancelled || !expectPhases.includes(phaseRef.current)) return;
      const p = e.payload;
      if (p.total > 0) {
        setPCur(p.current); setPTot(p.total);
        setProgress((p.current / p.total) * 100);
      }
      if (p.status === 'error') setHasErr(true);
      if (p.status !== 'processing') taskLogs.appendProgressLog(p);
    };
    const l1 = listen<ProgressPayload>('tagger-progress', handler(['tagging']));
    const l2 = listen<ProgressPayload>('tag-refine-progress', handler(['refining']));
    return () => { cancelled = true; l1.then(fn => fn()); l2.then(fn => fn()); };
  }, [taskLogs]);

  const handleFetchModels = async () => {
    if (!endpoint) return;
    setFetchingModels(true);
    try {
      const list = await invoke<string[]>('fetch_llm_models', { apiEndpoint: endpoint, apiKey });
      setModelList(list);
      if (list.length > 0 && !list.includes(modelName)) setModelName(list[0]);
      setFetchMsg({ text: t('llmTagger.fetchOk', { n: list.length }), ok: true });
    } catch (e: any) {
      setFetchMsg({ text: `${t('llmTagger.fetchFail')}: ${String(e)}`, ok: false });
    } finally {
      setFetchingModels(false);
      setTimeout(() => setFetchMsg(null), 3000);
    }
  };

  const handleSaveConfig = async () => {
    try {
      await invoke('save_api_config', { preset, customEndpoint, apiKeys: { [preset]: apiKey } });
      setSaveMsg({ text: t('llmTagger.configSaved'), ok: true });
    } catch (e: any) {
      setSaveMsg({ text: `${t('llmTagger.saveFailed')}: ${String(e)}`, ok: false });
    }
    setTimeout(() => setSaveMsg(null), 2000);
  };

  const toggleCat = (key: string) => {
    setEnabledCats(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };

  const isJson = outputFormat !== 'txt';
  const canStart = !!inputPath && !!selectedModel && !!endpoint && !!modelName && enabledCats.size > 0;

  const handleStart = async () => {
    if (!canStart) return;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    addTask('tagger', t('hybridTagger.taskName'));
    const sec = parseFloat(intervalSec);
    const intervalMs = sec < 0 ? -1 : Math.round(sec * 1000);
    const threads = Math.max(1, parseInt(concurrency) || 1);
    taskLogs.setInitialLog(t('hybridTagger.phaseTagging'));

    try {
      // 本地打标（直接按所选格式输出）
      setPhase('tagging');
      await invoke<ProcessResult>('start_tagging', {
        options: {
          input_path: inputPath,
          model_id: selectedModel,
          general_threshold: genTh,
          character_threshold: charTh,
          enabled_categories: [...enabledCats],
          use_gpu: useGpu,
          exclude_tags: '',
          append_tags: '',
          append_position: 'append',
          replace_underscore: replaceUnderscore,
          output_format: isJson ? 'json' : 'txt',
          json_simplified: outputFormat === 'json_simplified',
          escape_parentheses: escapeParentheses,
          sort_by: 'confidence',
          existing_tags_action: 'overwrite',
          batch_size: 1,
          recursive,
        },
      });

      // LLM 二次确认与调优（就地更新标签文件）
      setPhase('refining');
      setProgress(0); setPCur(0); setPTot(0);
      updateTask('tagger', { status: 'running', message: t('hybridTagger.phaseRefining') });
      taskLogs.appendLog(t('hybridTagger.phaseRefining'), 'info');
      await invoke<ProcessResult>('start_tag_refining', {
        options: {
          input_path: inputPath,
          output_path: inputPath,
          api_endpoint: endpoint,
          api_key: apiKey,
          model_name: modelName,
          prompt,
          temperature: parseFloat(temperature) || 0.3,
          max_tokens: -1,
          image_size: parseInt(imageSize) || 1024,
          top_p: parseFloat(topP) || 0,
          request_interval_ms: intervalMs,
          concurrency: threads,
          recursive,
          // JSON 模式差量写回：保留本地打标的字段归属，仅应用 LLM 的增删
          file_format: isJson ? 'json' : 'txt',
        },
      });

      setIsDone(true);
      taskLogs.appendLog(t('hybridTagger.allDone'), 'success');
      updateTask('tagger', { status: 'done', message: t('hybridTagger.allDone') });
    } catch (e: any) {
      const errStr = typeof e === 'string' ? e : e?.message || String(e);
      taskLogs.appendCatchError(errStr, t('pages.errorPrefix'));
      setHasErr(true); setIsDone(true);
      updateTask('tagger', {
        status: /已取消|cancel/i.test(errStr) ? 'cancelled' : 'error',
        message: errStr,
      });
    } finally {
      setProcessing(false);
      setPhase('');
    }
  };

  const clearLogs = () => { setLogs([]); setIsDone(false); setHasErr(false); };

  const numInput = (v: string, set: (n: number) => void, fallback: number, min: number, max: number) => {
    const n = parseFloat(v);
    set(Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback);
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
      {/* 数据集路径 */}
      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('llmTagger.datasetPath')}</span>
          <label style={{ display: 'flex', alignItems: 'center', gap: 4, cursor: 'pointer', fontSize: 11, color: 'var(--color-text-secondary)' }}>
            <Checkbox checked={recursive} onChange={setRecursive} size={14} />
            {t('llmTagger.recursiveScan')}
          </label>
        </div>
        <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
          <input className="form-input" placeholder={t('llmTagger.selectFolder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
          <InputPathPickerButton onSelect={setInputPath} />
        </div>
      </div>

      {/* 本地模型 | LLM 模型 —— 一行两块 */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-4)', alignItems: 'stretch' }}>
        {/* 本地打标 */}
        <div className="tool-panel" style={{ marginBottom: 0 }}>
          <div className="tool-panel-header"><span className="tool-panel-title">{t('hybridTagger.localPhase')}</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div style={{ display: 'flex', gap: 'var(--space-3)', alignItems: 'center' }}>
              <div style={{ flex: 1 }}>
                <CustomSelect
                  value={selectedModel}
                  onChange={setSelectedModel}
                  options={models.map(m => ({ value: m.id, label: m.is_downloaded ? m.name : `${m.name} (${t('hybridTagger.notDownloaded')})` }))}
                />
              </div>
              <Checkbox checked={useGpu} onChange={setUseGpu} label="GPU" size={14} />
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6 }}>
              {(() => {
                const curModel = models.find(m => m.id === selectedModel);
                const supported = new Set(curModel?.supported_categories || cats.map(c => c.key));
                return cats.map(c => {
                  const on = enabledCats.has(c.key);
                  const avail = supported.has(c.key);
                  return (
                    <div key={c.key} onClick={() => { if (avail) toggleCat(c.key); }}
                      title={avail ? undefined : t('hybridTagger.catUnsupported')}
                      style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 8px', borderRadius: 'var(--radius-sm)', border: `1px solid ${on && avail ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: !avail ? 'rgba(0,0,0,0.04)' : on ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)', cursor: avail ? 'pointer' : 'not-allowed', transition: 'all 0.15s', opacity: avail ? 1 : 0.35, minWidth: 0 }}>
                      <div style={{ width: 14, height: 14, borderRadius: 3, minWidth: 14, border: `2px solid ${on && avail ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, background: on && avail ? 'var(--color-accent-primary)' : 'transparent' }} />
                      <span style={{ fontSize: 12, fontWeight: 600, color: avail ? 'var(--color-text-primary)' : 'var(--color-text-tertiary)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.label}</span>
                    </div>
                  );
                });
              })()}
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-4)' }}>
              <div style={{ flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                  <span style={{ fontSize: 12 }}>{t('aiTagger.generalTh')}</span>
                  <span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{genTh.toFixed(2)}</span>
                </label>
                <input type="range" min="0.05" max="1" step="0.01" value={genTh} onChange={e => numInput(e.target.value, setGenTh, 0.35, 0.05, 1)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
              </div>
              <div style={{ flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                  <span style={{ fontSize: 12 }}>{t('aiTagger.charTh')}</span>
                  <span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{charTh.toFixed(2)}</span>
                </label>
                <input type="range" min="0.05" max="1" step="0.01" value={charTh} onChange={e => numInput(e.target.value, setCharTh, 0.85, 0.05, 1)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
              </div>
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-4)', flexWrap: 'wrap' }}>
              <Checkbox checked={replaceUnderscore} onChange={setReplaceUnderscore} label={t('aiTagger.replaceUnderscore')} size={14} />
              <span title={t('aiTagger.escapeParenthesesTip')}>
                <Checkbox checked={escapeParentheses} onChange={setEscapeParentheses} label={t('aiTagger.escapeParentheses')} size={14} />
              </span>
            </div>
          </div>
        </div>

        {/* API 设置 */}
        <div className="tool-panel" style={{ marginBottom: 0 }}>
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('llmTagger.apiSettings')}</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              {saveMsg && <span style={{ fontSize: 11, color: saveMsg.ok ? '#4ade80' : '#f87171' }}>{saveMsg.ok ? '✓' : '✗'} {saveMsg.text}</span>}
              <button className="btn btn-ghost btn-sm" onClick={handleSaveConfig} style={{ padding: '2px 8px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}>
                <Save style={{ width: 12, height: 12 }} /> {t('llmTagger.saveConfig')}
              </button>
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <div style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
                {Object.entries(PRESETS).map(([key, { label }]) => (
                  <button key={key} className={`btn btn-sm ${preset === key ? 'btn-primary' : 'btn-secondary'}`} onClick={() => setPreset(key)} style={{ fontSize: 11 }}>{label}</button>
                ))}
              </div>
              {preset === 'custom' ? (
                <input className="form-input" placeholder="https://api.example.com/v1/" value={customEndpoint} onChange={e => setCustomEndpoint(e.target.value)} style={{ marginTop: 6 }} />
              ) : (
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>{endpoint}</div>
              )}
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Key style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> API Key</label>
              <div style={{ position: 'relative' }}>
                <input className="form-input" type={showKey ? 'text' : 'password'} placeholder="sk-..." value={apiKey} onChange={e => setApiKey(e.target.value)} style={{ paddingRight: 32 }} />
                <button onClick={() => setShowKey(!showKey)} style={{ position: 'absolute', right: 6, top: '50%', transform: 'translateY(-50%)', background: 'none', border: 'none', cursor: 'pointer', color: 'var(--color-text-tertiary)', display: 'flex', padding: 2 }}>
                  {showKey ? <EyeOff style={{ width: 14, height: 14 }} /> : <Eye style={{ width: 14, height: 14 }} />}
                </button>
              </div>
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Bot style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.modelLabel')}</span>
                <button className="btn btn-ghost btn-sm" onClick={handleFetchModels} disabled={fetchingModels || !endpoint} style={{ padding: '2px 8px', fontSize: 11 }}>
                  {fetchingModels ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 12, height: 12 }} />} {t('llmTagger.fetchModels')}
                </button>
              </label>
              {modelList.length > 0 ? (
                <CustomSelect value={modelName} onChange={setModelName} options={modelList.map(m => ({ value: m, label: m }))} />
              ) : (
                <input className="form-input" placeholder={t('llmTagger.modelPlaceholder')} value={modelName} onChange={e => setModelName(e.target.value)} />
              )}
              {fetchMsg && <div style={{ fontSize: 11, marginTop: 4, color: fetchMsg.ok ? '#4ade80' : '#f87171' }}>{fetchMsg.ok ? '✓' : '✗'} {fetchMsg.text}</div>}
            </div>
          </div>
        </div>
      </div>

      {/* 调优设置聚合：提示词 + 参数 + 输出格式 */}
      <div className="tool-panel">
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('hybridTagger.llmPhase')}</span>
          <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={() => setPrompt(defaultHybridPrompt)}>{t('tagSort.resetDefault')}</button>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 300px', gap: 'var(--space-4)', alignItems: 'stretch' }}>
          {/* 左：提示词 */}
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            <label className="form-label">{t('hybridTagger.promptLabel')}</label>
            <textarea className="form-input" value={prompt} onChange={e => setPrompt(e.target.value)}
              style={{ fontSize: 11, fontFamily: 'monospace', lineHeight: 1.6, resize: 'vertical', flex: 1, minHeight: 200 }} />
            <p style={{ fontSize: 10, color: 'var(--color-text-tertiary)', margin: '4px 0 0' }}>{t('hybridTagger.promptHint')}</p>
          </div>
          {/* 右：参数 + 输出 */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.temperature')}</span>
                <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{temperature}</span>
              </label>
              <input type="range" min="0" max="2" step="0.05" value={temperature} onChange={e => setTemperature(e.target.value)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            <div>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
                <span>Top P</span>
                <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{topP || '0'}</span>
              </label>
              <input type="range" min="0" max="1" step="0.05" value={topP || '0'} onChange={e => setTopP(e.target.value)} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
              <div style={{ flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 4 }}><ImageIcon style={{ width: 12, height: 12, color: 'var(--color-text-tertiary)' }} /> {t('tagRefine.imageSize')}</label>
                <input className="form-input" type="number" min="256" max="4096" step="128" value={imageSize} onChange={e => setImageSize(e.target.value)} />
              </div>
              <div style={{ flex: 1 }}>
                <label className="form-label">{t('hybridTagger.concurrency')}</label>
                <input className="form-input" type="number" min={1} max={16} value={concurrency} onChange={e => setConcurrency(e.target.value)} />
              </div>
              <div style={{ flex: 1 }}>
                <label className="form-label">{t('hybridTagger.interval')}</label>
                <input className="form-input" type="number" step="0.1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)} />
              </div>
            </div>
            <div>
              <label className="form-label">{t('hybridTagger.outputFormat')}</label>
              <CustomSelect
                value={outputFormat}
                onChange={v => setOutputFormat(v as typeof outputFormat)}
                options={[
                  { value: 'txt', label: t('hybridTagger.formatTxt') },
                  { value: 'json', label: t('hybridTagger.formatJson') },
                  { value: 'json_simplified', label: t('hybridTagger.formatJsonSimplified') },
                ]}
              />
            </div>
          </div>
        </div>
      </div>

      {/* 操作 + 进度 + 日志 */}
      <ProcessButton
        processing={processing}
        onStart={handleStart}
        disabled={!canStart}
        cancelCommand={phase === 'refining' ? 'cancel_tag_refining' : 'force_cancel_tagging'}
        startText={t('hybridTagger.startText')}
        processingText={
          phase === 'tagging' ? t('hybridTagger.phaseShortTagging')
          : phase === 'refining' ? t('hybridTagger.phaseShortRefining')
          : t('pages.processing')
        }
        onCancelLog={(msg) => setLogs(prev => [...prev, { time: getTimeStr(), message: msg, status: 'warning' }])}
      />
      <ProgressLog
        progress={progress}
        current={pCur}
        total={pTot}
        logs={logs}
        isDone={isDone}
        hasError={hasErr}
        onClearLogs={clearLogs}
      />
    </div>
  );
}
