import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Play, Loader2, Globe, Key, MessageSquare, Bot, RefreshCw, ChevronDown, Thermometer, Hash, StopCircle, Save } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';
import { useTaskQueue } from './TaskContext';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const defaultSystemPrompt = `You are an expert image tagger for AI training datasets. Analyze the given image and output descriptive tags separated by commas. Focus on: subject, actions, composition, style, colors, lighting, and quality. Output ONLY tags, no explanation.`;
const defaultUserPrompt = `Please analyze this image and provide descriptive tags for AI training. Output comma-separated tags only.`;

export default function LlmTaggerTab() {
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
  const [sysPrompt, setSysPrompt] = useState(defaultSystemPrompt);
  const [userPrompt, setUserPrompt] = useState(defaultUserPrompt);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const [saveMsg, setSaveMsg] = useState<{ text: string; ok: boolean } | null>(null);

  const PRESETS: Record<string, { label: string; url: string }> = {
    openai: { label: 'OpenAI', url: 'https://api.openai.com/v1/' },
    gemini: { label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai/' },
    deepseek: { label: 'DeepSeek', url: 'https://api.deepseek.com/v1/' },
    custom: { label: '自定义', url: '' },
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
      setSaveMsg({ text: '配置已保存', ok: true });
    } catch (e: any) {
      setSaveMsg({ text: `保存失败: ${String(e)}`, ok: false });
    }
    setTimeout(() => setSaveMsg(null), 2000);
  };

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('llm-tagger-progress', (e) => {
      if (cancelled) return;
      const p = e.payload; setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasErr(true);
      setLogs(prev => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'] }]);
    }).then(fn => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
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
      setFetchMsg({ text: `获取成功，共 ${models.length} 个模型`, ok: true });
    } catch (e: any) {
      setFetchMsg({ text: `获取失败: ${String(e)}`, ok: false });
    } finally {
      setFetchingModels(false);
      setTimeout(() => setFetchMsg(null), 3000);
    }
  };

  const { addTask } = useTaskQueue();

  const handleStart = async () => {
    if (!inputPath || !endpoint || !modelName) return;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    addTask('llm-tagger', 'LLM 打标');
    setLogs([{ time: getTimeStr(), message: `开始 LLM 打标 | 模型: ${modelName} | API: ${endpoint}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_llm_tagging', {
        options: { input_path: inputPath, api_endpoint: endpoint, api_key: apiKey, model_name: modelName, system_prompt: sysPrompt, user_prompt: userPrompt, temperature: parseFloat(temperature) || 0.2, max_tokens: parseInt(maxTokens) || -1 },
      });
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasErr(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); }, []);

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
      {/* 左栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        {/* 路径 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">数据集路径</span></div>
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <input className="form-input" placeholder="选择图片文件夹..." value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
            <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setInputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
          </div>
        </div>

        {/* API 设置 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">API 设置</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              {saveMsg && <span style={{ fontSize: 11, color: saveMsg.ok ? '#4ade80' : '#f87171' }}>{saveMsg.ok ? '✓' : '✗'} {saveMsg.text}</span>}
              <button className="btn btn-ghost btn-sm" onClick={handleSaveConfig} style={{ padding: '2px 8px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}>
                <Save style={{ width: 12, height: 12 }} /> 保存配置
              </button>
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Globe style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> API 端点</label>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                {Object.entries(PRESETS).map(([key, { label }]) => (
                  <button key={key} className={`btn btn-sm ${preset === key ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setPreset(key)} style={{ flex: 1, fontSize: 11 }}>{label}</button>
                ))}
              </div>
              {preset === 'custom' && (
                <input className="form-input" placeholder="输入自定义 API 端点 URL..." value={customEndpoint}
                  onChange={e => setCustomEndpoint(e.target.value)} style={{ marginTop: 6 }} />
              )}
              {preset !== 'custom' && (
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>{endpoint}</div>
              )}
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Key style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> API Key</label>
              <input className="form-input" type="password" placeholder="sk-..." value={apiKey} onChange={e => setApiKey(e.target.value)} />
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Bot style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 模型</span>
                <button className="btn btn-ghost btn-sm" onClick={handleFetchModels} disabled={fetchingModels || !endpoint} style={{ padding: '2px 8px', fontSize: 11 }}>
                  {fetchingModels ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 12, height: 12 }} />} 获取模型列表
                </button>
              </label>
              {modelList.length > 0 ? (
                <div style={{ position: 'relative' }}>
                  <select className="form-input" value={modelName} onChange={e => setModelName(e.target.value)}
                    style={{ width: '100%', appearance: 'none', paddingRight: 32, cursor: 'pointer' }}>
                    {modelList.map(m => <option key={m} value={m}>{m}</option>)}
                  </select>
                  <ChevronDown style={{ position: 'absolute', right: 10, top: '50%', transform: 'translateY(-50%)', width: 14, height: 14, color: 'var(--color-text-tertiary)', pointerEvents: 'none' }} />
                </div>
              ) : (
                <input className="form-input" placeholder="模型名称..." value={modelName} onChange={e => setModelName(e.target.value)} />
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
          <div className="tool-panel-header"><span className="tool-panel-title">模型设置</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 温度</label>
                <input className="form-input" type="number" min="0" max="2" step="0.1" value={temperature} onChange={e => setTemperature(e.target.value)} placeholder="默认 0.2" />
              </div>
              <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Hash style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 最大 Tokens</label>
                <input className="form-input" type="number" min="-1" max="8192" step="1" value={maxTokens} onChange={e => setMaxTokens(e.target.value)} placeholder="-1 为不限制" />
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
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 打标中...</> : <><Play style={{ width: 18, height: 18 }} /> 开始 LLM 打标</>}
          </button>
          {processing && (
            <button className="btn btn-lg" style={{ height: 48, background: 'rgba(248,113,113,0.1)', color: '#f87171', border: '1px solid rgba(248,113,113,0.3)' }}
              onClick={() => invoke('cancel_llm_tagging')}>
              <StopCircle style={{ width: 18, height: 18 }} />
            </button>
          )}
        </div>

        <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasErr} onClearLogs={clearLogs} />
      </div>
    </div>
  );
}
