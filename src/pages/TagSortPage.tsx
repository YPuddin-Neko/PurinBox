import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowUpDown, FolderOpen, FolderOutput, Play, Loader2, Globe, Key, Bot,
  RefreshCw, ChevronDown, MessageSquare, StopCircle, Timer, Layers,
  CheckCircle2, XCircle, Info, ScrollText, Trash2, AlertTriangle, Save
} from 'lucide-react';
import { LogEntry, getTimeStr } from '../components/ProgressLog';
import { useTaskQueue } from '../components/TaskContext';

interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const defaultPrompt = `请对以下tags进行排序，按顺序：角色数量(例如:1girl等)→ 角色名字+特征 → 服装 → 表情细节 → 服装细节 → 视角 → 动作场景 → 其他。

重要规则：
1. 只重新排列现有tags的顺序
2. 不要添加任何新tag，不要删除任何原始tag
3. 只返回tag，逗号分隔

需要排序的tags: {tags}

排序后的tags:`;

export default function TagSortPage() {
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [preset, setPreset] = useState('openai');
  const [customEndpoint, setCustomEndpoint] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [modelName, setModelName] = useState('');
  const [modelList, setModelList] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchMsg, setFetchMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [prompt, setPrompt] = useState(defaultPrompt);
  const [intervalSec, setIntervalSec] = useState('-1');
  const [concurrency, setConcurrency] = useState('1');
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const [successCnt, setSuccessCnt] = useState(0);
  const [failCnt, setFailCnt] = useState(0);
  const [warnCnt, setWarnCnt] = useState(0);
  const [saveMsg, setSaveMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [startTime, setStartTime] = useState<number>(0);
  const [elapsed, setElapsed] = useState('');
  const [errorFiles, setErrorFiles] = useState<string[]>([]);
  const [warnFiles, setWarnFiles] = useState<string[]>([]);

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

  // 监听进度事件
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    listen<ProgressPayload>('tag-sort-progress', (e) => {
      if (cancelled) return;
      const p = e.payload;
      setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') {
        setIsDone(true);
        setProcessing(false);
      }
      if (p.status === 'error') {
        setHasErr(true);
        setFailCnt(c => c + 1);
        // 提取文件名
        const m = p.message.match(/\[\u9519\u8bef\] ([^:]+)/);
        if (m) setErrorFiles(prev => [...prev, m[1]]);
      }
      if (p.status === 'success') {
        setSuccessCnt(c => c + 1);
        if (p.message.includes('⚠')) {
          setWarnCnt(c => c + 1);
          const m = p.message.match(/\[\u5b8c\u6210\] ([^ ]+)/);
          if (m) setWarnFiles(prev => [...prev, m[1]]);
        }
      }
      setLogs(prev => [...prev, {
        time: getTimeStr(),
        message: p.message,
        status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'],
      }]);
    }).then(fn => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // 获取模型列表
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

  // 开始排序
  const handleStart = async () => {
    if (!inputPath || !outputPath || !endpoint || !modelName) return;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    setSuccessCnt(0); setFailCnt(0); setWarnCnt(0); setErrorFiles([]); setWarnFiles([]);
    setStartTime(Date.now()); setElapsed('');
    addTask('tag-sort', '标签排序');
    const sec = parseFloat(intervalSec);
    const intervalMs = sec < 0 ? -1 : Math.round(sec * 1000);
    const threads = Math.max(1, parseInt(concurrency) || 1);
    setLogs([{ time: getTimeStr(), message: `开始标签排序 | 模型: ${modelName} | 并发: ${threads} | 间隔: ${sec < 0 ? '无' : intervalSec + 's'}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_tag_sorting', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          api_endpoint: endpoint,
          api_key: apiKey,
          model_name: modelName,
          prompt: prompt,
          temperature: 1.0,
          max_tokens: -1,
          request_interval_ms: intervalMs,
          concurrency: threads,
        },
      });
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasErr(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); setSuccessCnt(0); setFailCnt(0); setWarnCnt(0); setErrorFiles([]); setWarnFiles([]); setElapsed(''); }, []);

  // 耗时计时器
  useEffect(() => {
    if (!processing || startTime === 0) return;
    const timer = setInterval(() => {
      const sec = Math.floor((Date.now() - startTime) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(m > 0 ? `${m}分${s}秒` : `${s}秒`);
    }, 1000);
    return () => clearInterval(timer);
  }, [processing, startTime]);

  // 完成后输出问题文件摘要
  useEffect(() => {
    if (!isDone) return;
    // 计算最终耗时
    if (startTime > 0) {
      const sec = Math.floor((Date.now() - startTime) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(m > 0 ? `${m}分${s}秒` : `${s}秒`);
    }
    // 输出问题文件
    if (errorFiles.length > 0) {
      setLogs(p => [...p, { time: getTimeStr(), message: `❌ 失败文件: ${errorFiles.join(', ')}`, status: 'error' }]);
    }
    if (warnFiles.length > 0) {
      setLogs(p => [...p, { time: getTimeStr(), message: `⚠️ 标签异常文件: ${warnFiles.join(', ')}`, status: 'info' }]);
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
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <ArrowUpDown style={{ width: 28, height: 28, color: '#f59e0b' }} />
          <h1 className="page-title">标签排序</h1>
        </div>
        <p className="page-subtitle">使用大语言模型对标签文件进行排序</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
        {/* 左栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">路径设置</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOpen style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 输入目录（含 .txt 标签文件）
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择标签文件所在文件夹..." value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setInputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOutput style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 输出目录
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择排序后输出文件夹..." value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setOutputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
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
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Timer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 请求间隔（秒）</label>
                  <input className="form-input" type="number" min="-1" max="120" step="1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)}
                    title="-1 表示无间隔" />
                </div>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Layers style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> 并发线程数</label>
                  <input className="form-input" type="number" min="1" max="32" step="1" value={concurrency} onChange={e => setConcurrency(e.target.value)}
                    title="同时处理的文件数" />
                </div>
              </div>
            </div>
          </div>


        </div>

        {/* 右栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 提示词 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">排序提示词</span>
              <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={() => setPrompt(defaultPrompt)}>恢复默认</button>
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> Prompt
                <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', fontWeight: 400 }}>（使用 {'{tags}'} 表示标签占位符）</span>
              </label>
              <textarea className="form-input" rows={8} value={prompt} onChange={e => setPrompt(e.target.value)}
                style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
          </div>

          {/* 操作按钮 */}
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <button className="btn btn-primary btn-lg" style={{ flex: 1, height: 48 }} onClick={handleStart}
              disabled={processing || !inputPath || !outputPath || !endpoint || !modelName}>
              {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 排序中...</> : <><Play style={{ width: 18, height: 18 }} /> 开始排序</>}
            </button>
            {processing && (
              <button className="btn btn-lg" style={{ height: 48, background: 'rgba(248,113,113,0.1)', color: '#f87171', border: '1px solid rgba(248,113,113,0.3)' }}
                onClick={() => invoke('cancel_tag_sorting')}>
                <StopCircle style={{ width: 18, height: 18 }} />
              </button>
            )}
          </div>

          {/* 自定义进度日志 */}
          <div className="progress-section">
            <div className="progress-header">
              <span className="progress-label">{isDone ? '处理完成' : '处理进度'}</span>
              <span className="progress-percent">{Math.round(progress)}%</span>
            </div>
            <div className="progress-bar-lg">
              <div className={`progress-fill-lg ${isDone ? (hasErr ? 'has-error' : 'done') : ''}`} style={{ width: `${progress}%` }} />
            </div>
            <div className="progress-count">{pCur} / {pTot} 个文件</div>

            <div className="log-panel" style={{ marginTop: 'var(--space-4)' }}>
              <div className="log-panel-header">
                <div className="log-panel-title"><ScrollText style={{ width: 14, height: 14 }} /> 处理日志</div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', fontSize: 12 }}>
                  {elapsed && <span style={{ color: 'var(--color-text-tertiary)' }}>⏱ {elapsed}</span>}
                  {successCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#4ade80' }}><CheckCircle2 style={{ width: 12, height: 12 }} /> {successCnt}</span>}
                  {failCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#f87171' }}><XCircle style={{ width: 12, height: 12 }} /> {failCnt}</span>}
                  {warnCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#fbbf24' }}><AlertTriangle style={{ width: 12, height: 12 }} /> {warnCnt}</span>}
                  <span className="log-panel-count">{logs.length} 条</span>
                  <button className="btn btn-ghost btn-sm" onClick={clearLogs} style={{ padding: '2px 6px' }} title="清空日志"><Trash2 style={{ width: 12, height: 12 }} /></button>
                </div>
              </div>

              <div className="log-content" ref={logContainerRef} onScroll={handleLogScroll}>
                {logs.length === 0 ? (
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--color-text-tertiary)', fontSize: 12 }}>暂无日志</div>
                ) : logs.map((log, i) => (
                  <div key={i} className={`log-entry ${i === logs.length - 1 ? 'log-entry-new' : ''}`}>
                    <span className="log-entry-time">{log.time}</span>
                    {statusIcon(log.status)}
                    <span className={`log-entry-message ${log.status}`}
                      style={log.message.includes('⚠') ? { color: '#fbbf24' } : undefined}>{log.message}</span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
