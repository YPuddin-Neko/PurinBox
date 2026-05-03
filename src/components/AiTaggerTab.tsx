import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Play, Loader2, Cpu, Zap, Download, Plus, Check, RefreshCw, ChevronDown, Trash2, Search, FileUp, X } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';
import { useTaskQueue } from './TaskContext';

interface ModelInfo { id: string; name: string; description: string; input_size: number; is_builtin: boolean; is_downloaded: boolean; repo_id: string; input_format: string; }
interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }
interface OnnxModelInfo { input_size: number; input_format: string; input_shape: number[]; channels: number; }
interface DownloadPayload { filename: string; downloaded: number; total: number; percent: number; speed_mbps: number; status: string; message: string; }

const cats = [
  { key: 'general', label: '通用标签', default: true },
  { key: 'character', label: '角色标签', default: true },
  { key: 'rating', label: '评级标签', default: false },
  { key: 'artist', label: '作者标签', default: false },
  { key: 'copyright', label: '版权标签', default: false },
  { key: 'meta', label: '元信息标签', default: false },
];

export default function AiTaggerTab() {
  const [inputPath, setInputPath] = useState('');
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('');
  const [genTh, setGenTh] = useState(0.55);
  const [charTh, setCharTh] = useState(0.85);
  const [enabled, setEnabled] = useState<Set<string>>(new Set(cats.filter(c => c.default).map(c => c.key)));
  const [useGpu, setUseGpu] = useState(false);
  const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
  const gpuSupported = !isMac; // macOS 无 NVIDIA GPU，不支持 GPU 加速
  const [cudaOk, setCudaOk] = useState<boolean | null>(null);
  const [cudaChecking, setCudaChecking] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  // 下载进度
  const [dlProgress, setDlProgress] = useState<DownloadPayload | null>(null);
  // 导入模型
  const [showAdd, setShowAdd] = useState(false);
  const [nName, setNName] = useState('');
  const [nModelPath, setNModelPath] = useState('');
  const [nTagsPath, setNTagsPath] = useState('');
  const [nSize, setNSize] = useState(448);
  const [nFormat, setNFormat] = useState('NHWC');
  const [detecting, setDetecting] = useState(false);
  const [importing, setImporting] = useState(false);

  const load = useCallback(async () => {
    try { const l = await invoke<ModelInfo[]>('get_tagger_models'); setModels(l); if (l.length > 0 && !selectedModel) setSelectedModel(l[0].id); } catch {}
  }, [selectedModel]);
  useEffect(() => {
    load();
  }, []);

  // 打标进度事件
  useEffect(() => {
    let active = true;
    const unlistenPromise = listen<ProgressPayload>('tagger-progress', (e) => {
      if (!active) return;
      const p = e.payload; setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasErr(true);
      if (p.status === 'download-progress') {
        // 原地更新最后一行（进度条效果）
        setLogs(prev => {
          if (prev.length > 0 && prev[prev.length - 1].status === 'info' && prev[prev.length - 1].message.startsWith('⬇')) {
            return [...prev.slice(0, -1), { time: getTimeStr(), message: p.message, status: 'info' }];
          }
          return [...prev, { time: getTimeStr(), message: p.message, status: 'info' }];
        });
      } else {
        setLogs(prev => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'] }]);
      }
    });
    return () => { active = false; unlistenPromise.then(fn => fn()); };
  }, []);

  // 下载进度事件（独立，不刷日志）
  useEffect(() => {
    let active = true;
    const unlistenPromise = listen<DownloadPayload>('tagger-download', (e) => {
      if (!active) return;
      const d = e.payload;
      if (d.status === 'done' || d.status === 'cancelled' || d.status === 'error') {
        setDlProgress(null);
        if (d.status === 'error') {
          setLogs(p => [...p, { time: getTimeStr(), message: `下载失败: ${d.message}`, status: 'error' }]);
        }
      } else {
        setDlProgress(d);
      }
    });
    return () => { active = false; unlistenPromise.then(fn => fn()); };
  }, []);

  const { addTask, updateTask } = useTaskQueue();
  const cur = models.find(m => m.id === selectedModel);

  const handleCancel = async () => {
    try { await invoke('cancel_tagging'); } catch {}
    try { await invoke('cancel_tagger_download'); } catch {}
    setDlProgress(null);
    setProcessing(false);
    setLogs(p => [...p, { time: getTimeStr(), message: '已取消', status: 'info' }]);
    updateTask('tagger', { status: 'cancelled' });
  };

  const handleStart = async () => {
    if (!inputPath || !selectedModel || enabled.size === 0) return;
    // 如果正在打标，先取消上一次
    if (processing) {
      await handleCancel();
      await new Promise(r => setTimeout(r, 300));
    }
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    setLogs([{ time: getTimeStr(), message: `开始打标 | 模型: ${cur?.name} | 硬件: ${useGpu ? 'GPU' : 'CPU'}`, status: 'info' }]);
    addTask('tagger', `AI 打标 - ${cur?.name || '未知'}`);
    try {
      await invoke<ProcessResult>('start_tagging', { options: { input_path: inputPath, model_id: selectedModel, general_threshold: genTh, character_threshold: charTh, enabled_categories: Array.from(enabled), use_gpu: useGpu } });
      updateTask('tagger', { status: 'done' });
      await load();
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]);
      setHasErr(true); setIsDone(true);
      updateTask('tagger', { status: 'error', message: String(e) });
    }
    finally { setProcessing(false); }
  };

  const browseOnnx = async () => {
    const f = await open({ multiple: false, filters: [{ name: 'ONNX Model', extensions: ['onnx'] }] });
    if (f) setNModelPath(f as string);
  };
  const browseTags = async () => {
    const f = await open({ multiple: false, filters: [{ name: '标签文件', extensions: ['csv', 'json'] }] });
    if (f) setNTagsPath(f as string);
  };

  const autoDetect = async () => {
    if (!nModelPath) { setLogs(p => [...p, { time: getTimeStr(), message: '请先选择模型文件 (.onnx)', status: 'error' }]); return; }
    setDetecting(true);
    try {
      const info = await invoke<OnnxModelInfo>('detect_onnx_model_info', { modelPath: nModelPath });
      setNSize(info.input_size);
      setNFormat(info.input_format);
      setLogs(p => [...p, { time: getTimeStr(), message: `✓ 自动检测成功 | 输入尺寸: ${info.input_size}px | 通道格式: ${info.input_format} | 形状: [${info.input_shape.join(', ')}]`, status: 'success' }]);
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `自动检测失败: ${String(e)}`, status: 'error' }]);
    }
    setDetecting(false);
  };

  const handleImport = async () => {
    if (!nName || !nModelPath || !nTagsPath) {
      setLogs(p => [...p, { time: getTimeStr(), message: '请填写名称并选择模型文件和标签文件', status: 'error' }]);
      return;
    }
    setImporting(true);
    try {
      await invoke<string>('import_local_tagger_model', { name: nName, modelPath: nModelPath, tagsPath: nTagsPath, inputSize: nSize, inputFormat: nFormat });
      setLogs(p => [...p, { time: getTimeStr(), message: `✓ 模型 "${nName}" 导入成功`, status: 'success' }]);
      setShowAdd(false); setNName(''); setNModelPath(''); setNTagsPath(''); setNSize(448); setNFormat('NHWC');
      await load();
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `导入失败: ${String(e)}`, status: 'error' }]);
    }
    setImporting(false);
  };

  const handleDelete = async (id: string, name: string) => {
    if (!confirm(`确认删除自定义模型 "${name}" 吗？\n模型文件也会被删除。`)) return;
    try {
      await invoke('remove_custom_tagger_model', { id });
      setLogs(p => [...p, { time: getTimeStr(), message: `已删除模型: ${name}`, status: 'info' }]);
      if (selectedModel === id) setSelectedModel('');
      await load();
    } catch (e: any) {
      setLogs(p => [...p, { time: getTimeStr(), message: `删除失败: ${String(e)}`, status: 'error' }]);
    }
  };

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

        {/* 模型选择 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">打标模型</span>
            <button className="btn btn-ghost btn-sm" onClick={() => setShowAdd(!showAdd)}>
              {showAdd ? '✕ 关闭' : <><Plus style={{ width: 14, height: 14 }} /> 导入模型</>}
            </button>
          </div>

          {/* 导入本地模型面板 */}
          {showAdd && (
            <div style={{ padding: 'var(--space-3)', borderRadius: 'var(--radius-md)', background: 'rgba(124,92,252,0.04)', border: '1px solid rgba(124,92,252,0.15)', marginBottom: 'var(--space-3)', display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* 名称 */}
              <div>
                <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>名称：</label>
                <input className="form-input" placeholder="例如: My Custom Tagger" value={nName} onChange={e => setNName(e.target.value)} style={{ width: '100%' }} />
              </div>
              {/* 模型文件 */}
              <div>
                <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>模型文件 (.onnx)：</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择 .onnx 文件..." value={nModelPath} onChange={e => setNModelPath(e.target.value)} style={{ flex: 1 }} readOnly />
                  <button className="btn btn-secondary btn-sm" onClick={browseOnnx}><FileUp style={{ width: 14, height: 14 }} /> 浏览...</button>
                </div>
              </div>
              {/* 标签映射 */}
              <div>
                <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>标签映射 (.csv / .json)：</label>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <input className="form-input" placeholder="选择标签文件..." value={nTagsPath} onChange={e => setNTagsPath(e.target.value)} style={{ flex: 1 }} readOnly />
                  <button className="btn btn-secondary btn-sm" onClick={browseTags}><FileUp style={{ width: 14, height: 14 }} /> 浏览...</button>
                </div>
              </div>
              {/* 通道格式 + 尺寸 + 自动检测 */}
              <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'flex-end' }}>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>输入通道：</label>
                  <div style={{ position: 'relative' }}>
                    <select className="form-input" value={nFormat} onChange={e => setNFormat(e.target.value)} style={{ width: 90, appearance: 'none', paddingRight: 24, cursor: 'pointer' }}>
                      <option value="NHWC">NHWC</option>
                      <option value="NCHW">NCHW</option>
                    </select>
                    <ChevronDown style={{ position: 'absolute', right: 6, top: '50%', transform: 'translateY(-50%)', width: 12, height: 12, color: 'var(--color-text-tertiary)', pointerEvents: 'none' }} />
                  </div>
                </div>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>输入尺寸：</label>
                  <input className="form-input" type="number" value={nSize} onChange={e => setNSize(Number(e.target.value))} style={{ width: 80 }} />
                </div>
                <button className="btn btn-secondary btn-sm" onClick={autoDetect} disabled={detecting || !nModelPath} style={{ height: 34, whiteSpace: 'nowrap' }}>
                  {detecting ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Search style={{ width: 14, height: 14 }} />} 自动识别
                </button>
                <button className="btn btn-primary btn-sm" onClick={handleImport} disabled={importing || !nName || !nModelPath || !nTagsPath} style={{ height: 34 }}>
                  {importing ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Plus style={{ width: 14, height: 14 }} />} 添加
                </button>
              </div>
              <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', lineHeight: 1.5 }}>
                💡 <b>输入通道</b>：NHWC = [批次,高,宽,通道数]（TensorFlow 风格），NCHW = [批次,通道数,高,宽]（PyTorch 风格）。选错会导致结果异常，建议使用「自动识别」。
              </div>
            </div>
          )}

          <div style={{ position: 'relative' }}>
            <select className="form-input" value={selectedModel} onChange={e => setSelectedModel(e.target.value)}
              style={{ width: '100%', appearance: 'none', paddingRight: 32, cursor: 'pointer' }}>
              {models.map(m => (
                <option key={m.id} value={m.id}>{m.name} {m.is_downloaded ? '✓' : '⬇'}</option>
              ))}
            </select>
            <ChevronDown style={{ position: 'absolute', right: 10, top: '50%', transform: 'translateY(-50%)', width: 16, height: 16, color: 'var(--color-text-tertiary)', pointerEvents: 'none' }} />
          </div>
          {cur && (
            <div style={{ marginTop: 'var(--space-2)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', display: 'flex', gap: 'var(--space-3)', alignItems: 'center' }}>
              <span>{cur.description}</span>
              <span style={{ padding: '1px 6px', borderRadius: 'var(--radius-full)', fontSize: 10, background: cur.is_downloaded ? 'rgba(74,222,128,0.1)' : 'rgba(251,191,36,0.1)', color: cur.is_downloaded ? '#4ade80' : '#fbbf24' }}>
                {cur.is_downloaded ? '已下载' : '待下载'}
              </span>
              <span style={{ padding: '1px 6px', borderRadius: 'var(--radius-full)', fontSize: 10, background: 'rgba(124,92,252,0.1)', color: '#a78bfa' }}>
                {cur.input_format} · {cur.input_size}px
              </span>
              {!cur.is_builtin && (
                <button className="btn btn-ghost btn-sm" onClick={() => handleDelete(cur.id, cur.name)}
                  style={{ marginLeft: 'auto', padding: '2px 6px', color: '#f87171' }}>
                  <Trash2 style={{ width: 12, height: 12 }} /> 删除
                </button>
              )}
            </div>
          )}
        </div>

        {/* 标签分类 + 阈值 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">标签分类与阈值</span></div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 6, marginBottom: 'var(--space-3)' }}>
            {cats.map(c => {
              const on = enabled.has(c.key);
              return (
                <div key={c.key} onClick={() => setEnabled(p => { const n = new Set(p); on ? n.delete(c.key) : n.add(c.key); return n; })} style={{
                  display: 'flex', alignItems: 'center', gap: 6, padding: '6px 10px', borderRadius: 'var(--radius-sm)',
                  border: `1px solid ${on ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                  background: on ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)', cursor: 'pointer', transition: 'all 0.15s',
                }}>
                  <div style={{ width: 14, height: 14, borderRadius: 3, minWidth: 14, border: `2px solid ${on ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, background: on ? 'var(--color-accent-primary)' : 'transparent', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    {on && <Check style={{ width: 9, height: 9, color: '#fff' }} />}
                  </div>
                  <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{c.label}</span>
                </div>
              );
            })}
          </div>
          <div style={{ display: 'flex', gap: 'var(--space-4)' }}>
            <div style={{ flex: 1 }}>
              <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ fontSize: 12 }}>通用阈值</span><span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{genTh.toFixed(2)}</span>
              </label>
              <input type="range" min="0.05" max="1" step="0.01" value={genTh} onChange={e => setGenTh(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            <div style={{ flex: 1 }}>
              <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ fontSize: 12 }}>角色阈值</span><span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{charTh.toFixed(2)}</span>
              </label>
              <input type="range" min="0.05" max="1" step="0.01" value={charTh} onChange={e => setCharTh(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
          </div>
        </div>

        {/* 硬件设置 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">硬件设置</span></div>

          <div style={{ marginBottom: 'var(--space-3)', fontSize: 11, color: 'var(--color-text-secondary)', padding: '6px 10px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-elevated)', lineHeight: 1.6 }}>
            推理引擎: Python onnxruntime<br/>
            <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>{isMac ? 'macOS: CPU 推理' : 'Windows: CUDA 加速 (需 onnxruntime-gpu)'}</span>
          </div>

          {/* CPU / GPU 切换 */}
          <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
            <div onClick={() => setUseGpu(false)} style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '8px', borderRadius: 'var(--radius-md)', border: `1px solid ${!useGpu ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: !useGpu ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)', cursor: 'pointer' }}>
              <Cpu style={{ width: 16, height: 16, color: !useGpu ? '#60a5fa' : 'var(--color-text-tertiary)' }} />
              <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>CPU</span>
            </div>
            <div onClick={() => { if (gpuSupported) setUseGpu(true); }} style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '8px', borderRadius: 'var(--radius-md)', border: `1px solid ${useGpu ? 'rgba(74,222,128,0.5)' : 'var(--color-border)'}`, background: useGpu ? 'rgba(74,222,128,0.06)' : 'var(--color-bg-input)', cursor: gpuSupported ? 'pointer' : 'not-allowed', opacity: gpuSupported ? 1 : 0.4 }}>
              <Zap style={{ width: 16, height: 16, color: useGpu ? '#4ade80' : 'var(--color-text-tertiary)' }} />
              <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>GPU</span>
              {!gpuSupported && <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>(不可用)</span>}
            </div>
          </div>
          {useGpu && (
            <div style={{ marginTop: 'var(--space-3)', display: 'flex', flexDirection: 'column', gap: 8 }}>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <button className="btn btn-secondary btn-sm" onClick={async () => { setCudaChecking(true); try { const [ok] = await invoke<[boolean, string]>('check_cuda_available'); setCudaOk(ok); } catch(e: any) { setCudaOk(false); setLogs(p => [...p, { time: getTimeStr(), message: `GPU 检测异常: ${String(e)}`, status: 'error' }]); } setCudaChecking(false); }} disabled={cudaChecking} style={{ whiteSpace: 'nowrap' }}>
                  {cudaChecking ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 14, height: 14 }} />} 检测 GPU
                </button>
              </div>
              {cudaOk !== null && <div style={{ fontSize: 11, color: cudaOk ? '#4ade80' : '#f87171', padding: '6px 10px', borderRadius: 'var(--radius-sm)', background: cudaOk ? 'rgba(74,222,128,0.06)' : 'rgba(248,113,113,0.06)', lineHeight: 1.6 }}>{cudaOk ? '✓ GPU 加速可用' : '✗ GPU 加速不可用 — 详情请查看日志'}</div>}
            </div>
          )}
        </div>
      </div>

      {/* 右栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
          <button className="btn btn-primary btn-lg" style={{ flex: 1, height: 48 }} onClick={handleStart}
            disabled={!inputPath || !selectedModel || enabled.size === 0}>
            {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 打标中...</> :
              cur && !cur.is_downloaded ? <><Download style={{ width: 18, height: 18 }} /> 下载并打标</> : <><Play style={{ width: 18, height: 18 }} /> 开始打标</>}
          </button>
          {processing && (
            <button className="btn btn-secondary btn-lg" style={{ height: 48, color: '#f87171' }} onClick={handleCancel}>
              <X style={{ width: 18, height: 18 }} /> 取消
            </button>
          )}
        </div>

        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 'var(--font-size-sm)' }}>
            {[['模型', cur?.name || '未选择', '#f59e0b'], ['状态', cur?.is_downloaded ? '已下载' : '首次使用自动下载', cur?.is_downloaded ? '#4ade80' : '#fbbf24'], ['格式', cur ? `${cur.input_format} · ${cur.input_size}px` : '-', '#a78bfa'], ['分类', `${enabled.size} 类`, 'var(--color-text-primary)'], ['硬件', useGpu ? 'GPU' : 'CPU', useGpu ? '#4ade80' : '#60a5fa'], ['通用阈值', genTh.toFixed(2), '#f59e0b'], ['角色阈值', charTh.toFixed(2), '#f59e0b']].map(([k, v, c]) => (
              <div key={k as string} style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ color: 'var(--color-text-tertiary)' }}>{k}</span>
                <span style={{ fontWeight: 600, color: c as string }}>{v}</span>
              </div>
            ))}
          </div>
        </div>

        {/* 下载进度条 */}
        {dlProgress && (
          <div className="tool-panel" style={{ padding: 'var(--space-4)' }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <Download style={{ width: 14, height: 14, color: '#60a5fa', animation: 'pulse 1.5s infinite' }} />
                <span style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>下载模型</span>
              </div>
              <button onClick={async () => {
                  try { await invoke('cancel_tagger_download'); } catch {}
                  try { await invoke('cancel_gpu_runtime_download'); } catch {}
                  setDlProgress(null);
                }}
                style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '3px 8px', borderRadius: 'var(--radius-sm)', border: '1px solid rgba(248,113,113,0.3)', background: 'rgba(248,113,113,0.06)', color: '#f87171', fontSize: 11, fontWeight: 600, cursor: 'pointer' }}>
                <X style={{ width: 12, height: 12 }} /> 取消
              </button>
            </div>
            <div style={{ marginBottom: 6 }}>
              <div style={{ height: 6, borderRadius: 3, background: 'var(--color-border)', overflow: 'hidden' }}>
                <div style={{ height: '100%', borderRadius: 3, background: 'linear-gradient(90deg, #7c5cfc, #60a5fa)', width: `${dlProgress.percent}%`, transition: 'width 0.3s ease' }} />
              </div>
            </div>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--color-text-tertiary)' }}>
              <span>{dlProgress.message}</span>
              <span style={{ fontWeight: 700, fontFamily: 'monospace', color: '#60a5fa' }}>{dlProgress.percent.toFixed(1)}%</span>
            </div>
          </div>
        )}

        {logs.length > 0 && (
          <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasErr} onClearLogs={() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); }} />
        )}
      </div>
    </div>
  );
}
