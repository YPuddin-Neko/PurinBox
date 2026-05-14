import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  FolderOpen, Play, Loader2, Globe, Key, MessageSquare, Bot,
  RefreshCw, Thermometer, Hash, StopCircle, Save, ImageIcon,
  CheckCircle2, XCircle, Info, ScrollText, Trash2, Eye, EyeOff
} from 'lucide-react';
import { LogEntry, getTimeStr } from './ProgressLog';
import { useTaskQueue } from './TaskContext';
import CustomSelect from './CustomSelect';
import { useTranslation } from 'react-i18next';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

// ── 默认提示词模板 ──────────────────────────────────

// TXT 自然语言描述
const defaultSystemPrompt_txt = `You are a professional image captioning assistant. Provide a detailed, natural language description of the image suitable for training image generation models.`;
const defaultUserPrompt_txt = `Please describe this image in detail.`;

// JSON 完整格式 (Full) — 嵌套 ai_output 结构
const defaultSystemPrompt_json_full = `You are an anime image tagging expert. Output ONLY valid JSON.

Output a JSON object with an "ai_output" wrapper containing these fields:
- count: string — character count tag ("1girl", "2boys", "1girl, 1boy", "no humans")
- appearance: string[] — visual features (hair color/style, eye color, clothing, accessories, body features)
- tags: string[] — actions, expressions, poses, composition, held objects
- environment: string[] — background, location, lighting, weather, atmosphere
- nl: string — one fluent sentence describing the overall scene

Example output:
{"ai_output": {"count": "1girl", "appearance": ["long hair", "black hair", "red eyes", "kimono", "hair ornament"], "tags": ["standing", "smile", "looking at viewer", "upper body"], "environment": ["indoors", "traditional room", "soft lighting"], "nl": "A graceful girl in traditional attire smiles warmly in a serene room."}}

Rules:
- Use lowercase booru-style tags with spaces, not underscores
- Each tag is a separate array element
- Only describe what is clearly visible
- Do NOT include quality, character name, series, or artist tags (those are managed separately)
- Output ONLY the JSON object, no markdown fences or explanation`;

const defaultUserPrompt_json_full = `Analyze this image and output structured tags as JSON with the "ai_output" wrapper.`;

// JSON 简化格式 (Simplified) — 扁平结构，官方推荐格式
const defaultSystemPrompt_json_simplified = `You are an anime image tagging expert. Output ONLY valid JSON.

JSON fields (all tag arrays use lowercase strings):
1. count: string — character count ("1girl", "2boys", "1girl, 1boy", "no humans")
2. appearance: string[] — visual features (hair color, eye color, hairstyle, clothing, accessories)
3. tags: string[] — actions, expressions, poses, composition, objects
4. environment: string[] — background, location, lighting, atmosphere
5. nl: string — one sentence natural language description

Example:
{"count": "1girl", "appearance": ["long hair", "blue hair", "twintails", "blue eyes", "school uniform"], "tags": ["singing", "microphone", "dynamic pose"], "environment": ["stage", "spotlight", "crowd", "night"], "nl": "Miku performs energetically on stage under bright spotlights."}

Rules:
- Use lowercase booru-style tags with spaces, not underscores
- Each tag is a separate array element
- Only describe what is clearly visible
- Do NOT include quality, character name, series, or artist tags (those are managed separately)
- Output ONLY the JSON object, no markdown fences or explanation`;

const defaultUserPrompt_json_simplified = `Analyze this image and output structured tags as a flat JSON object.`;

// 根据输出格式获取默认提示词
function getDefaultPrompts(format: 'txt' | 'json', simplified: boolean) {
  if (format === 'json') {
    return simplified
      ? { sys: defaultSystemPrompt_json_simplified, user: defaultUserPrompt_json_simplified }
      : { sys: defaultSystemPrompt_json_full, user: defaultUserPrompt_json_full };
  }
  return { sys: defaultSystemPrompt_txt, user: defaultUserPrompt_txt };
}

export default function LlmTaggerTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const [preset, setPreset] = useState('openai');
  const [customEndpoint, setCustomEndpoint] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [modelName, setModelName] = useState('');
  const [modelList, setModelList] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchMsg, setFetchMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [temperature, setTemperature] = useState('0.2');
  const [maxTokens, setMaxTokens] = useState('');
  const [sysPrompt, setSysPrompt] = useState(() => getDefaultPrompts('txt', false).sys);
  const [userPrompt, setUserPrompt] = useState(() => getDefaultPrompts('txt', false).user);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const [saveMsg, setSaveMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [imageSize, setImageSize] = useState('1024');
  const [topP, setTopP] = useState('');
  const [skipExisting, setSkipExisting] = useState(false);
  const [outputFormat, setOutputFormat] = useState<'txt' | 'json'>('txt');
  const [jsonSimplified, setJsonSimplified] = useState(()=>localStorage.getItem('tagger_json_simplified')==='true');
  const [showKey, setShowKey] = useState(false);
  const [successCnt, setSuccessCnt] = useState(0);
  const [failCnt, setFailCnt] = useState(0);
  const [startTime, setStartTime] = useState<number>(0);
  const [elapsed, setElapsed] = useState('');
  const [errorFiles, setErrorFiles] = useState<string[]>([]);

  const PRESETS: Record<string, { label: string; url: string }> = {
    openai: { label: 'OpenAI', url: 'https://api.openai.com/v1/' },
    gemini: { label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai/' },
    deepseek: { label: 'DeepSeek', url: 'https://api.deepseek.com/v1/' },
    custom: { label: t('llmTagger.customLabel'), url: '' },
  };

  const endpoint = preset === 'custom' ? customEndpoint : (PRESETS[preset]?.url || '');

  // 加载保存的配置
  useEffect(() => {
    invoke<[string, string, string]>('load_api_config').then(([p, ce, key]) => {
      if (p) setPreset(p);
      if (ce) setCustomEndpoint(ce);
      if (key) setApiKey(key);
    }).catch(() => {});
  }, []);

  const handleSaveConfig = async () => {
    try {
      await invoke('save_api_config', { preset, customEndpoint, apiKey });
      setSaveMsg({ text: t('llmTagger.configSaved'), ok: true });
    } catch (e: any) {
      setSaveMsg({ text: `${t('llmTagger.saveFailed')}: ${String(e)}`, ok: false });
    }
    setTimeout(() => setSaveMsg(null), 2000);
  };

  useEffect(() => {
    let cancelled = false;
    const listenPromise = listen<ProgressPayload>('llm-tagger-progress', (e) => {
      if (cancelled) return;
      const p = e.payload; setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') { setIsDone(true); setProcessing(false); }
      if (p.status === 'error') {
        setHasErr(true);
        setFailCnt(c => c + 1);
        const m = p.message.match(/\[错误\] ([^:(]+)/);
        if (m) setErrorFiles(prev => [...prev, m[1].trim()]);
      }
      if (p.status === 'success') setSuccessCnt(c => c + 1);
      setLogs(prev => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'] }]);
    });
    return () => { cancelled = true; listenPromise.then(fn => fn()); };
  }, []);

  const handleFetchModels = async () => {
    if (!endpoint) return;
    setFetchingModels(true);
    try {
      const models = await invoke<string[]>('fetch_llm_models', { apiEndpoint: endpoint, apiKey: apiKey });
      setModelList(models);
      if (models.length > 0 && !models.includes(modelName)) {
        setModelName(models[0]);
      }
      setFetchMsg({ text: t('llmTagger.fetchOk', { n: models.length }), ok: true });
    } catch (e: any) {
      setFetchMsg({ text: `${t('llmTagger.fetchFail')}: ${String(e)}`, ok: false });
    } finally {
      setFetchingModels(false);
      setTimeout(() => setFetchMsg(null), 3000);
    }
  };

  const { addTask } = useTaskQueue();

  const handleStart = async () => {
    if (!inputPath || !endpoint || !modelName) return;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    setSuccessCnt(0); setFailCnt(0); setErrorFiles([]); setStartTime(Date.now()); setElapsed('');
    addTask('llm-tagger', t('llmTagger.taskName'));
    setLogs([{ time: getTimeStr(), message: t('llmTagger.startMsg', { model: modelName, api: endpoint }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_llm_tagging', {
        options: {
          input_path: inputPath, api_endpoint: endpoint, api_key: apiKey, model_name: modelName,
          system_prompt: sysPrompt, user_prompt: userPrompt,
          temperature: parseFloat(temperature) || 0.2, max_tokens: parseInt(maxTokens) || -1,
          image_size: parseInt(imageSize) || 1024,
          top_p: parseFloat(topP) || 0,
          skip_existing: skipExisting,
          output_format: outputFormat,
          json_simplified: jsonSimplified,
        },
      });
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      setHasErr(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); setSuccessCnt(0); setFailCnt(0); setErrorFiles([]); setElapsed(''); }, []);

  // 耗时计时器
  useEffect(() => {
    if (!processing || startTime === 0) return;
    const timer = setInterval(() => {
      const sec = Math.floor((Date.now() - startTime) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(m > 0 ? `${m}m${s}s` : `${s}s`);
    }, 1000);
    return () => clearInterval(timer);
  }, [processing, startTime]);

  // 完成后输出失败文件摘要
  useEffect(() => {
    if (!isDone) return;
    if (startTime > 0) {
      const sec = Math.floor((Date.now() - startTime) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(m > 0 ? `${m}m${s}s` : `${s}s`);
    }
    if (errorFiles.length > 0) {
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('llmTagger.failedFiles')}: ${errorFiles.join(', ')}`, status: 'error' }]);
    }
  }, [isDone]);

  // 日志自动滚动
  const logContainerRef = useRef<HTMLDivElement>(null);
  const isNearBottomRef = useRef(true);
  const handleLogScroll = () => {
    const el = logContainerRef.current;
    if (!el) return;
    isNearBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  };
  useEffect(() => {
    if (isNearBottomRef.current && logContainerRef.current) {
      logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight;
    }
  }, [logs.length]);

  const statusIcon = (status: LogEntry['status']) => {
    switch (status) {
      case 'success': return <CheckCircle2 className="log-entry-icon success" />;
      case 'error': return <XCircle className="log-entry-icon error" />;
      case 'processing': return <Loader2 className="log-entry-icon processing" />;
      default: return <Info className="log-entry-icon info" />;
    }
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
      {/* 左栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        {/* 路径 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">{t('llmTagger.datasetPath')}</span></div>
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <input className="form-input" placeholder={t('llmTagger.selectFolder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
            <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setInputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
          </div>
        </div>

        {/* API 设置 */}
        <div className="tool-panel">
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
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Globe style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('llmTagger.apiEndpoint')}</label>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                {Object.entries(PRESETS).map(([key, { label }]) => (
                  <button key={key} className={`btn btn-sm ${preset === key ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setPreset(key)} style={{ flex: 1, fontSize: 11 }}>{label}</button>
                ))}
              </div>
              {preset === 'custom' && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginTop: 6 }}>
                  <span style={{ fontSize: 11, color: 'var(--color-text-secondary)' }}>{t('llmTagger.apiAddress')}</span>
                  <span title={t('llmTagger.openaiOnly')} style={{ cursor: 'help', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: '50%', fontSize: 9, fontWeight: 700, color: 'var(--color-text-tertiary)', border: '1px solid var(--color-border)' }}>?</span>
                </div>
              )}
              {preset === 'custom' && (
                <input className="form-input" placeholder={t('llmTagger.customPlaceholder')} value={customEndpoint}
                  onChange={e => setCustomEndpoint(e.target.value)} style={{ marginTop: 4 }} />
              )}
              {preset !== 'custom' && (
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
                <CustomSelect value={modelName} onChange={v => setModelName(v)}
                  options={modelList.map(m => ({ value: m, label: m }))} />
              ) : (
                <input className="form-input" placeholder={t('llmTagger.modelPlaceholder')} value={modelName} onChange={e => setModelName(e.target.value)} />
              )}
              {fetchMsg && (
                <div style={{ fontSize: 11, marginTop: 4, color: fetchMsg.ok ? '#4ade80' : '#f87171' }}>
                  {fetchMsg.ok ? '✓' : '✗'} {fetchMsg.text}
                </div>
              )}
            </div>
          </div>
        </div>

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
                  <span style={{ fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' }}>{topP || '0'}</span>
                </label>
                <input type="range" min="0" max="1" step="0.05" value={topP || '0'}
                  onChange={e => setTopP(e.target.value === '0' ? '' : e.target.value)}
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
                    // 自动切换提示词（仅当用户没有自定义时）
                    const allDefaults = [defaultSystemPrompt_txt, defaultSystemPrompt_json_full, defaultSystemPrompt_json_simplified];
                    if (allDefaults.some(d => sysPrompt === d)) {
                      const p = getDefaultPrompts(fmt, jsonSimplified);
                      setSysPrompt(p.sys); setUserPrompt(p.user);
                    }
                  }} style={{ padding: '2px 10px', borderRadius: 'var(--radius-sm)', border: `1px solid ${outputFormat === fmt ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: outputFormat === fmt ? 'rgba(124,92,252,0.08)' : 'transparent', color: outputFormat === fmt ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', fontSize: 11, fontWeight: 600, cursor: 'pointer' }}>.{fmt}</button>
                ))}
                {outputFormat==='json'&&(
                  <select className="form-input" value={jsonSimplified?'simplified':'full'} onChange={e=>{
                    const v=e.target.value==='simplified';
                    setJsonSimplified(v);
                    localStorage.setItem('tagger_json_simplified',String(v));
                    // 自动切换提示词
                    const allDefaults = [defaultSystemPrompt_txt, defaultSystemPrompt_json_full, defaultSystemPrompt_json_simplified];
                    if (allDefaults.some(d => sysPrompt === d)) {
                      const p = getDefaultPrompts('json', v);
                      setSysPrompt(p.sys); setUserPrompt(p.user);
                    }
                  }} style={{fontSize:10,height:24,padding:'0 6px',width:'auto',marginLeft:2}}>
                    <option value="full">{t('llmTagger.fullFormat')}</option>
                    <option value="simplified">{t('llmTagger.simplified')}</option>
                  </select>
                )}
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

      {/* 右栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
          <button className="btn btn-primary btn-lg" style={{ flex: 1, height: 48 }} onClick={handleStart}
            disabled={processing || !inputPath || !endpoint || !modelName}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> {t('llmTagger.tagging')}</> : <><Play style={{ width: 18, height: 18 }} /> {t('llmTagger.startLlmTag')}</>}
          </button>
          {processing && (
            <button className="btn btn-lg" style={{ height: 48, background: 'rgba(248,113,113,0.1)', color: '#f87171', border: '1px solid rgba(248,113,113,0.3)' }}
              onClick={() => invoke('cancel_llm_tagging')}>
              <StopCircle style={{ width: 18, height: 18 }} />
            </button>
          )}
        </div>

        {/* 自定义进度日志 */}
        <div className="progress-section">
          <div className="progress-header">
            <span className="progress-label">{isDone ? t('llmTagger.progressDone') : t('llmTagger.progressLabel')}</span>
            <span className="progress-percent">
              {(() => {
                if (!startTime || pCur <= 0) return null;
                const el = (Date.now() - startTime) / 1000;
                if (el < 0.5) return null;
                const spd = pCur / el;
                const txt = spd >= 1 ? `${spd.toFixed(1)} it/s` : `${(1 / spd).toFixed(1)} s/it`;
                return <span style={{ marginRight: 8, fontSize: 11, color: 'var(--color-text-tertiary)', fontWeight: 400 }}>{txt}</span>;
              })()}
              {Math.round(progress)}%
            </span>
          </div>
          <div className="progress-bar-lg">
            <div className={`progress-fill-lg ${isDone ? (hasErr ? 'has-error' : 'done') : ''}`} style={{ width: `${progress}%` }} />
          </div>
          <div className="progress-count">{pCur} / {pTot} {t('llmTagger.fileCount')}</div>

          <div className="log-panel" style={{ marginTop: 'var(--space-4)' }}>
            <div className="log-panel-header">
              <div className="log-panel-title"><ScrollText style={{ width: 14, height: 14 }} /> {t('llmTagger.logTitle')}</div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', fontSize: 12 }}>
                {elapsed && <span style={{ color: 'var(--color-text-tertiary)' }}>⏱ {elapsed}</span>}
                {successCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#4ade80' }}><CheckCircle2 style={{ width: 12, height: 12 }} /> {successCnt}</span>}
                {failCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#f87171' }}><XCircle style={{ width: 12, height: 12 }} /> {failCnt}</span>}
                <span className="log-panel-count">{t('llmTagger.logCount', { n: logs.length })}</span>
                <button className="btn btn-ghost btn-sm" onClick={clearLogs} style={{ padding: '2px 6px' }} title={t('llmTagger.logTitle')}><Trash2 style={{ width: 12, height: 12 }} /></button>
              </div>
            </div>

            <div className="log-content" ref={logContainerRef} onScroll={handleLogScroll}>
              {logs.length === 0 ? (
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--color-text-tertiary)', fontSize: 12 }}>{t('llmTagger.noLogs')}</div>
              ) : logs.map((log, i) => (
                <div key={i} className={`log-entry ${i === logs.length - 1 ? 'log-entry-new' : ''}`}>
                  <span className="log-entry-time">{log.time}</span>
                  {statusIcon(log.status)}
                  <span className={`log-entry-message ${log.status}`}>{log.message}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
