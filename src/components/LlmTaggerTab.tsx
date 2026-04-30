import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Play, Loader2, Globe, Key, MessageSquare, Bot } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const defaultSystemPrompt = `You are an expert image tagger for AI training datasets. Analyze the given image and output descriptive tags separated by commas. Focus on: subject, actions, composition, style, colors, lighting, and quality. Output ONLY tags, no explanation.`;
const defaultUserPrompt = `Please analyze this image and provide descriptive tags for AI training. Output comma-separated tags only.`;

export default function LlmTaggerTab() {
  const [inputPath, setInputPath] = useState('');
  const [endpoint, setEndpoint] = useState('https://api.openai.com/v1/');
  const [apiKey, setApiKey] = useState('');
  const [modelName, setModelName] = useState('gpt-4o');
  const [sysPrompt, setSysPrompt] = useState(defaultSystemPrompt);
  const [userPrompt, setUserPrompt] = useState(defaultUserPrompt);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);

  useEffect(() => {
    let u: UnlistenFn | null = null;
    listen<ProgressPayload>('llm-tagger-progress', (e) => {
      const p = e.payload; setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasErr(true);
      setLogs(prev => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'] }]);
    }).then(fn => { u = fn; });
    return () => { u?.(); };
  }, []);

  const handleStart = async () => {
    if (!inputPath || !endpoint || !modelName) return;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    setLogs([{ time: getTimeStr(), message: `开始 LLM 打标 | 模型: ${modelName} | API: ${endpoint}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_llm_tagging', {
        options: { input_path: inputPath, api_endpoint: endpoint, api_key: apiKey, model_name: modelName, system_prompt: sysPrompt, user_prompt: userPrompt },
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
          <div className="tool-panel-header"><span className="tool-panel-title">API 设置</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Globe style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> API 端点</label>
              <input className="form-input" placeholder="https://api.openai.com/v1/" value={endpoint} onChange={e => setEndpoint(e.target.value)} />
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Key style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> API Key</label>
              <input className="form-input" type="password" placeholder="sk-..." value={apiKey} onChange={e => setApiKey(e.target.value)} />
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Bot style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 模型名称</label>
              <input className="form-input" placeholder="gpt-4o / gpt-4o-mini / ..." value={modelName} onChange={e => setModelName(e.target.value)} />
            </div>
          </div>
        </div>

        {/* Prompt 设置 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">Prompt 设置</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> System Prompt</label>
              <textarea className="form-input" rows={4} value={sysPrompt} onChange={e => setSysPrompt(e.target.value)} style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> User Prompt</label>
              <textarea className="form-input" rows={3} value={userPrompt} onChange={e => setUserPrompt(e.target.value)} style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
          </div>
        </div>
      </div>

      {/* 右栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleStart}
          disabled={processing || !inputPath || !endpoint || !modelName}>
          {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 打标中...</> : <><Play style={{ width: 18, height: 18 }} /> 开始 LLM 打标</>}
        </button>

        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">使用说明</span></div>
          <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', lineHeight: 1.8 }}>
            <p>📡 <strong>API 端点</strong>：支持任何兼容 OpenAI 格式的 API，包括：</p>
            <ul style={{ paddingLeft: 18, margin: '4px 0' }}>
              <li>OpenAI 官方 API</li>
              <li>Azure OpenAI</li>
              <li>本地部署 (Ollama, vLLM, LMStudio 等)</li>
              <li>第三方中转 API</li>
            </ul>
            <p style={{ marginTop: 6 }}>🤖 <strong>模型要求</strong>：必须支持 Vision（图片识别）能力</p>
            <p style={{ marginTop: 4 }}>💡 <strong>Prompt</strong>：根据需求调整 Prompt 可以控制标签的风格和内容</p>
          </div>
        </div>

        {(logs.length > 0 || processing) && (
          <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasErr} onClearLogs={clearLogs} />
        )}
      </div>
    </div>
  );
}
