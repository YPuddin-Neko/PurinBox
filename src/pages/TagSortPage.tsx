import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowUpDown, FolderOpen, FolderOutput, Loader2, Globe, Key, Bot,
  RefreshCw, MessageSquare, Timer, Layers,
  CheckCircle2, XCircle, Info, ScrollText, Trash2, AlertTriangle, Save, Thermometer,
  Eye, EyeOff
} from 'lucide-react';
import { LogEntry, getTimeStr } from '../components/ProgressLog';
import { useTaskQueue } from '../components/TaskContext';
import { useTranslation } from 'react-i18next';
import CustomSelect from '../components/CustomSelect';
import ProcessButton from '../components/ProcessButton';

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
  const { t } = useTranslation();
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
  const [temperature, setTemperature] = useState('0');
  const [topP, setTopP] = useState('0');
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
  const [showKey, setShowKey] = useState(false);

  const PRESETS: Record<string, { label: string; url: string }> = {
    openai: { label: 'OpenAI', url: 'https://api.openai.com/v1/' },
    gemini: { label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai/' },
    deepseek: { label: 'DeepSeek', url: 'https://api.deepseek.com/v1/' },
    custom: { label: t('tagSort.customLabel'), url: '' },
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
      setSaveMsg({ text: t('tagSort.configSaved'), ok: true });
    } catch (e: any) {
      setSaveMsg({ text: `${t('tagSort.saveFailed')}: ${String(e)}`, ok: false });
    }
    setTimeout(() => setSaveMsg(null), 2000);
  };

  // 监听进度事件
  useEffect(() => {
    let cancelled = false;
    const listenPromise = listen<ProgressPayload>('tag-sort-progress', (e) => {
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
    });
    return () => { cancelled = true; listenPromise.then(fn => fn()); };
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
      setFetchMsg({ text: t('tagSort.fetchOk', { n: models.length }), ok: true });
    } catch (e: any) {
      setFetchMsg({ text: `${t('tagSort.fetchFail')}: ${String(e)}`, ok: false });
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
    addTask('tag-sort', t('tagSort.taskName'));
    const sec = parseFloat(intervalSec);
    const intervalMs = sec < 0 ? -1 : Math.round(sec * 1000);
    const threads = Math.max(1, parseInt(concurrency) || 1);
    setLogs([{ time: getTimeStr(), message: t('tagSort.startMsg', { model: modelName, threads, interval: sec < 0 ? t('tagSort.noInterval') : intervalSec + 's' }), status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_tag_sorting', {
        options: {
          input_path: inputPath,
          output_path: outputPath,
          api_endpoint: endpoint,
          api_key: apiKey,
          model_name: modelName,
          prompt: prompt,
          temperature: parseFloat(temperature) || 0,
          max_tokens: -1,
          request_interval_ms: intervalMs,
          concurrency: threads,
          top_p: parseFloat(topP) || 0,
        },
      });
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('pages.errorPrefix')}: ${String(e)}`, status: 'error' }]);
      setHasErr(true); setIsDone(true);
    } finally { setProcessing(false); }
  };

  const clearLogs = useCallback(() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); setSuccessCnt(0); setFailCnt(0); setWarnCnt(0); setErrorFiles([]); setWarnFiles([]); setElapsed(''); }, []);
  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);

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

  // 完成后输出问题文件摘要
  useEffect(() => {
    if (!isDone) return;
    // 计算最终耗时
    if (startTime > 0) {
      const sec = Math.floor((Date.now() - startTime) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(m > 0 ? `${m}m${s}s` : `${s}s`);
    }
    // 输出问题文件
    if (errorFiles.length > 0) {
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('tagSort.failedFiles')}: ${errorFiles.join(', ')}`, status: 'error' }]);
    }
    if (warnFiles.length > 0) {
      setLogs(p => [...p, { time: getTimeStr(), message: `${t('tagSort.warnFiles')}: ${warnFiles.join(', ')}`, status: 'info' }]);
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
          <h1 className="page-title">{t('tagSort.title')}</h1>
        </div>
        <p className="page-subtitle">{t('tagSort.subtitle')}</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)' }}>
        {/* 左栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 路径设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('pages.pathSettings')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOpen style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.inputDir')}
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('tagSort.inputPlaceholder')} value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setInputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <FolderOutput style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.outputDir')}
                </label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder={t('tagSort.outputPlaceholder')} value={outputPath} onChange={e => setOutputPath(e.target.value)} style={{ flex: 1 }} />
                  <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setOutputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
                </div>
              </div>
            </div>
          </div>

          {/* API 设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('tagSort.apiSettings')}</span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                {saveMsg && <span style={{ fontSize: 11, color: saveMsg.ok ? '#4ade80' : '#f87171' }}>{saveMsg.ok ? '✓' : '✗'} {saveMsg.text}</span>}
                <button className="btn btn-ghost btn-sm" onClick={handleSaveConfig} style={{ padding: '2px 8px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}>
                  <Save style={{ width: 12, height: 12 }} /> {t('tagSort.saveConfig')}
                </button>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div className="form-group" style={{ marginBottom: 0 }}>
                <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Globe style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.apiEndpoint')}</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  {Object.entries(PRESETS).map(([key, { label }]) => (
                    <button key={key} className={`btn btn-sm ${preset === key ? 'btn-primary' : 'btn-secondary'}`}
                      onClick={() => setPreset(key)} style={{ flex: 1, fontSize: 11 }}>{label}</button>
                  ))}
                </div>
                {preset === 'custom' && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginTop: 6 }}>
                    <span style={{ fontSize: 11, color: 'var(--color-text-secondary)' }}>{t('tagSort.apiAddress')}</span>
                    <span title={t('tagSort.openaiFormatOnly')} style={{ cursor: 'help', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: '50%', fontSize: 9, fontWeight: 700, color: 'var(--color-text-tertiary)', border: '1px solid var(--color-border)' }}>?</span>
                  </div>
                )}
                {preset === 'custom' && (
                  <input className="form-input" placeholder={t('tagSort.customPlaceholder')} value={customEndpoint}
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
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Bot style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.modelLabel')}</span>
                  <button className="btn btn-ghost btn-sm" onClick={handleFetchModels} disabled={fetchingModels || !endpoint} style={{ padding: '2px 8px', fontSize: 11 }}>
                    {fetchingModels ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 12, height: 12 }} />} {t('tagSort.fetchModels')}
                  </button>
                </label>
                {modelList.length > 0 ? (
                  <CustomSelect value={modelName} onChange={v => setModelName(v)}
                    options={modelList.map(m => ({ value: m, label: m }))} />
                ) : (
                  <input className="form-input" placeholder={t('tagSort.modelPlaceholder')} value={modelName} onChange={e => setModelName(e.target.value)} />
                )}
                {fetchMsg && (
                  <div style={{ fontSize: 11, marginTop: 4, color: fetchMsg.ok ? '#4ade80' : '#f87171' }}>
                    {fetchMsg.ok ? '✓' : '✗'} {fetchMsg.text}
                  </div>
                )}
              </div>
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Timer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.interval')}</label>
                  <input className="form-input" type="number" min="-1" max="120" step="1" value={intervalSec} onChange={e => setIntervalSec(e.target.value)}
                    title={t('tagSort.intervalTip')} />
                </div>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Layers style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.concurrency')}</label>
                  <input className="form-input" type="number" min="1" max="32" step="1" value={concurrency} onChange={e => setConcurrency(e.target.value)}
                    title={t('tagSort.concurrencyTip')} />
                </div>
              </div>
              <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
                <div className="form-group" style={{ marginBottom: 0, flex: 1 }}>
                  <label className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> {t('tagSort.temperature')}</span>
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
            </div>
          </div>


        </div>

        {/* 右栏 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {/* 提示词 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <span className="tool-panel-title">{t('tagSort.promptTitle')}</span>
              <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={() => setPrompt(defaultPrompt)}>{t('tagSort.resetDefault')}</button>
            </div>
            <div className="form-group" style={{ marginBottom: 0 }}>
              <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> Prompt
                <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', fontWeight: 400 }}>（{t('tagSort.promptHint')}）</span>
              </label>
              <textarea className="form-input" rows={8} value={prompt} onChange={e => setPrompt(e.target.value)}
                style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
            </div>
          </div>

          {/* 操作按钮 */}
          <ProcessButton processing={processing} onStart={handleStart}
            disabled={!inputPath || !outputPath || !endpoint || !modelName}
            cancelCommand="cancel_tag_sorting" startText={t('tagSort.startSort')} processingText={t('tagSort.sorting')}
            onCancelLog={addCancelLog} />

          {/* 自定义进度日志 */}
          <div className="progress-section">
            <div className="progress-header">
              <span className="progress-label">{isDone ? t('tagSort.progressDone') : t('tagSort.progressLabel')}</span>
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
            <div className="progress-count">{pCur} / {pTot} {t('tagSort.fileCount')}</div>

            <div className="log-panel" style={{ marginTop: 'var(--space-4)' }}>
              <div className="log-panel-header">
                <div className="log-panel-title"><ScrollText style={{ width: 14, height: 14 }} /> {t('tagSort.logTitle')}</div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', fontSize: 12 }}>
                  {elapsed && <span style={{ color: 'var(--color-text-tertiary)' }}>⏱ {elapsed}</span>}
                  {successCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#4ade80' }}><CheckCircle2 style={{ width: 12, height: 12 }} /> {successCnt}</span>}
                  {failCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#f87171' }}><XCircle style={{ width: 12, height: 12 }} /> {failCnt}</span>}
                  {warnCnt > 0 && <span style={{ display: 'flex', alignItems: 'center', gap: 3, color: '#fbbf24' }}><AlertTriangle style={{ width: 12, height: 12 }} /> {warnCnt}</span>}
                  <span className="log-panel-count">{t('tagSort.logCount', { n: logs.length })}</span>
                  <button className="btn btn-ghost btn-sm" onClick={clearLogs} style={{ padding: '2px 6px' }} title={t('tagSort.logTitle')}><Trash2 style={{ width: 12, height: 12 }} /></button>
                </div>
              </div>

              <div className="log-content" ref={logContainerRef} onScroll={handleLogScroll}>
                {logs.length === 0 ? (
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--color-text-tertiary)', fontSize: 12 }}>{t('tagSort.noLogs')}</div>
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
