import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Loader2, Cpu, Gpu, Download, Plus, Check, RefreshCw, Trash2, Search, FileUp } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';
import ProcessButton from './ProcessButton';
import { useTaskQueue } from './TaskContext';
import { ConfirmModal } from './Modal';
import CustomSelect from './CustomSelect';

interface ModelInfo { id: string; name: string; description: string; input_size: number; is_builtin: boolean; is_downloaded: boolean; repo_id: string; input_format: string; supported_categories: string[]; }
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
  { key: 'quality', label: '质量标签', default: false },
  { key: 'model', label: '模型标签', default: false },
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
  const gpuSupported = true; // Windows: CUDA, macOS: CoreML (Neural Engine)
  const [cudaOk, setCudaOk] = useState<boolean | null>(null);
  const [cudaChecking, setCudaChecking] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  // 导入模型
  const [showAdd, setShowAdd] = useState(false);
  const [nName, setNName] = useState('');
  const [nModelPath, setNModelPath] = useState('');
  const [nTagsPath, setNTagsPath] = useState('');
  const [nSize, setNSize] = useState(448);
  const [nFormat, setNFormat] = useState('NHWC');
  const [detecting, setDetecting] = useState(false);
  const [importing, setImporting] = useState(false);
  // 其他设置
  const [excludeTags, setExcludeTags] = useState('');
  const [appendTags, setAppendTags] = useState('');
  const [appendPosition, setAppendPosition] = useState<'prepend' | 'append'>('append');
  const [replaceUnderscore, setReplaceUnderscore] = useState(true);
  const [outputFormat, setOutputFormat] = useState<'txt' | 'json'>('txt');
  const [jsonSimplified, setJsonSimplified] = useState(()=>localStorage.getItem('tagger_json_simplified')==='true');
  const [deleteConfirm, setDeleteConfirm] = useState<{ id: string; name: string } | null>(null);

  const load = useCallback(async () => {
    try { const l = await invoke<ModelInfo[]>('get_tagger_models'); setModels(l); if (l.length > 0 && !selectedModel) setSelectedModel(l[0].id); } catch {}
  }, [selectedModel]);
  useEffect(() => {
    load();
  }, []);

  // 切换模型时自动移除不支持的分类
  useEffect(() => {
    const curModel = models.find(m => m.id === selectedModel);
    if (!curModel) return;
    const supported = new Set(curModel.supported_categories);
    setEnabled(prev => {
      const next = new Set([...prev].filter(k => supported.has(k)));
      // 至少保留一个支持的分类
      if (next.size === 0) cats.filter(c => c.default && supported.has(c.key)).forEach(c => next.add(c.key));
      if (next.size === 0 && curModel.supported_categories.length > 0) next.add(curModel.supported_categories[0]);
      return next;
    });
  }, [selectedModel, models]);

  // 打标进度事件
  useEffect(() => {
    let active = true;
    const handler = (e: { payload: ProgressPayload }) => {
      if (!active) return;
      const p = e.payload; setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasErr(true);
      setLogs(prev => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'] }]);
    };
    const u1 = listen<ProgressPayload>('tagger-progress', handler);
    const u2 = listen<ProgressPayload>('python-env-progress', handler);
    return () => { active = false; u1.then(fn => fn()); u2.then(fn => fn()); };
  }, []);

  // 下载进度事件 → 更新日志中的下载条目
  useEffect(() => {
    let active = true;
    const handler = (e: { payload: DownloadPayload }) => {
      if (!active) return;
      const d = e.payload;
      if (d.status === 'done' || d.status === 'cancelled') {
        // 下载结束，移除进度条日志
        setLogs(p => p.filter(l => l.status !== 'download'));
      } else if (d.status === 'error') {
        setLogs(p => [...p.filter(l => l.status !== 'download'), { time: getTimeStr(), message: `下载失败: ${d.message}`, status: 'error' }]);
      } else {
        const avgSpeed = d.speed_mbps > 0 ? `${d.speed_mbps.toFixed(1)} MB/s` : '';
        setLogs(p => {
          const idx = p.findIndex(l => l.status === 'download');
          const entry: LogEntry = { time: getTimeStr(), message: d.message, status: 'download', dlPercent: d.percent, dlSpeed: avgSpeed };
          if (idx >= 0) {
            const next = [...p];
            next[idx] = entry;
            return next;
          }
          return [...p, entry];
        });
      }
    };
    const u1 = listen<DownloadPayload>('tagger-download', handler);
    const u2 = listen<DownloadPayload>('python-env-download', handler);
    return () => { active = false; u1.then(fn => fn()); u2.then(fn => fn()); };
  }, []);

  const { addTask, updateTask } = useTaskQueue();
  const cur = models.find(m => m.id === selectedModel);

  const addCancelLog = useCallback((msg: string) => setLogs(p => [...p, { time: getTimeStr(), message: msg, status: 'warning' as const }]), []);
  const doCancel = async () => {
    try { await invoke('cancel_tagging'); } catch {}
    try { await invoke('cancel_tagger_download'); } catch {}
    setProcessing(false);
    updateTask('tagger', { status: 'cancelled' });
  };

  const handleStart = async () => {
    if (!inputPath || !selectedModel || enabled.size === 0) return;
    // 如果正在打标，先取消上一次
    if (processing) {
      doCancel();
      await new Promise(r => setTimeout(r, 300));
    }
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    setLogs([{ time: getTimeStr(), message: `开始打标 | 模型: ${cur?.name} | 硬件: ${useGpu ? 'GPU' : 'CPU'}`, status: 'info' }]);
    addTask('tagger', `Tagger 打标 - ${cur?.name || '未知'}`);
    try {
      await invoke<ProcessResult>('start_tagging', { options: { input_path: inputPath, model_id: selectedModel, general_threshold: genTh, character_threshold: charTh, enabled_categories: Array.from(enabled), use_gpu: useGpu, exclude_tags: excludeTags, append_tags: appendTags, append_position: appendPosition, replace_underscore: replaceUnderscore, output_format: outputFormat, json_simplified: jsonSimplified } });
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
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)', alignItems: 'start' }}>
      {/* 左栏 - 所有设置 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        {/* 数据集路径 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">数据集路径</span></div>
          <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
            <input className="form-input" placeholder="选择图片文件夹..." value={inputPath} onChange={e => setInputPath(e.target.value)} style={{ flex: 1 }} />
            <button className="btn btn-secondary" onClick={async () => { const s = await open({ directory: true, multiple: false }); if (s) setInputPath(s as string); }}><FolderOpen style={{ width: 16, height: 16 }} /></button>
          </div>
        </div>

        {/* 打标模型 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">打标模型</span>
            <button className="btn btn-ghost btn-sm" onClick={() => setShowAdd(!showAdd)}>
              {showAdd ? '✕ 关闭' : <><Plus style={{ width: 14, height: 14 }} /> 导入模型</>}
            </button>
          </div>
          {showAdd && (
            <div style={{ padding: 'var(--space-3)', borderRadius: 'var(--radius-md)', background: 'rgba(124,92,252,0.04)', border: '1px solid rgba(124,92,252,0.15)', marginBottom: 'var(--space-3)', display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>名称：</label><input className="form-input" placeholder="例如: My Custom Tagger" value={nName} onChange={e => setNName(e.target.value)} style={{ width: '100%' }} /></div>
              <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>模型文件 (.onnx)：</label><div style={{ display: 'flex', gap: 'var(--space-2)' }}><input className="form-input" placeholder="选择 .onnx 文件..." value={nModelPath} onChange={e => setNModelPath(e.target.value)} style={{ flex: 1 }} readOnly /><button className="btn btn-secondary btn-sm" onClick={browseOnnx}><FileUp style={{ width: 14, height: 14 }} /> 浏览...</button></div></div>
              <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>标签映射 (.csv / .json)：</label><div style={{ display: 'flex', gap: 'var(--space-2)' }}><input className="form-input" placeholder="选择标签文件..." value={nTagsPath} onChange={e => setNTagsPath(e.target.value)} style={{ flex: 1 }} readOnly /><button className="btn btn-secondary btn-sm" onClick={browseTags}><FileUp style={{ width: 14, height: 14 }} /> 浏览...</button></div></div>
              <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'flex-end' }}>
                <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>输入通道：</label><CustomSelect value={nFormat} onChange={v => setNFormat(v)} options={[{ value: 'NHWC', label: 'NHWC' }, { value: 'NCHW', label: 'NCHW' }]} compact style={{ width: 90 }} /></div>
                <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>输入尺寸：</label><input className="form-input" type="number" value={nSize} onChange={e => setNSize(Number(e.target.value))} style={{ width: 80 }} /></div>
                <button className="btn btn-secondary btn-sm" onClick={autoDetect} disabled={detecting || !nModelPath} style={{ height: 34, whiteSpace: 'nowrap' }}>{detecting ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Search style={{ width: 14, height: 14 }} />} 自动识别</button>
                <button className="btn btn-primary btn-sm" onClick={handleImport} disabled={importing || !nName || !nModelPath || !nTagsPath} style={{ height: 34 }}>{importing ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Plus style={{ width: 14, height: 14 }} />} 添加</button>
              </div>
              <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', lineHeight: 1.5 }}>💡 <b>输入通道</b>：NHWC = [批次,高,宽,通道数]（TensorFlow），NCHW = [批次,通道数,高,宽]（PyTorch）。建议使用「自动识别」。</div>
            </div>
          )}
          <CustomSelect value={selectedModel} onChange={v => setSelectedModel(v)}
            options={models.map(m => ({ value: m.id, label: `${m.name} ${m.is_downloaded ? '✓' : '⬇'}` }))} />
          {cur && (
            <div style={{ marginTop: 'var(--space-2)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', display: 'flex', gap: 'var(--space-3)', alignItems: 'center' }}>
              <span>{cur.description}</span>
              <span style={{ padding: '1px 6px', borderRadius: 'var(--radius-full)', fontSize: 10, background: cur.is_downloaded ? 'rgba(74,222,128,0.1)' : 'rgba(251,191,36,0.1)', color: cur.is_downloaded ? '#4ade80' : '#fbbf24' }}>{cur.is_downloaded ? '已下载' : '待下载'}</span>
              <span style={{ padding: '1px 6px', borderRadius: 'var(--radius-full)', fontSize: 10, background: 'rgba(124,92,252,0.1)', color: '#a78bfa' }}>{cur.input_format} · {cur.input_size}px</span>
              {!cur.is_builtin && (<button className="btn btn-ghost btn-sm" onClick={() => setDeleteConfirm({ id: cur.id, name: cur.name })} style={{ marginLeft: 'auto', padding: '2px 6px', color: '#f87171' }}><Trash2 style={{ width: 12, height: 12 }} /> 删除</button>)}
            </div>
          )}
        </div>

        {/* 标签分类与阈值 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">标签分类与阈值</span></div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6, marginBottom: 'var(--space-3)' }}>
            {(()=>{ const curModel = models.find(m => m.id === selectedModel); const supported = new Set(curModel?.supported_categories || cats.map(c=>c.key)); return cats.map(c => { const on = enabled.has(c.key); const avail = supported.has(c.key); return (
              <div key={c.key} onClick={() => { if(!avail)return; setEnabled(p => { const n = new Set(p); on ? n.delete(c.key) : n.add(c.key); return n; }); }} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 10px', borderRadius: 'var(--radius-sm)', border: `1px solid ${!avail ? 'var(--color-border)' : on ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: !avail ? 'rgba(0,0,0,0.04)' : on ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)', cursor: avail ? 'pointer' : 'not-allowed', transition: 'all 0.15s', opacity: avail ? 1 : 0.35 }}>
                <div style={{ width: 14, height: 14, borderRadius: 3, minWidth: 14, border: `2px solid ${on && avail ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, background: on && avail ? 'var(--color-accent-primary)' : 'transparent', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{on && avail && <Check style={{ width: 9, height: 9, color: '#fff' }} />}</div>
                <span style={{ fontSize: 12, fontWeight: 600, color: avail ? 'var(--color-text-primary)' : 'var(--color-text-tertiary)' }}>{c.label}</span>
              </div>); }); })()}
          </div>
          <div style={{ display: 'flex', gap: 'var(--space-4)' }}>
            <div style={{ flex: 1 }}><label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}><span style={{ fontSize: 12 }}>通用阈值</span><span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{genTh.toFixed(2)}</span></label><input type="range" min="0.05" max="1" step="0.01" value={genTh} onChange={e => setGenTh(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} /></div>
            <div style={{ flex: 1 }}><label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}><span style={{ fontSize: 12 }}>角色阈值</span><span style={{ fontWeight: 700, color: '#f59e0b', fontFamily: 'monospace', fontSize: 12 }}>{charTh.toFixed(2)}</span></label><input type="range" min="0.05" max="1" step="0.01" value={charTh} onChange={e => setCharTh(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} /></div>
          </div>
        </div>

        {/* 硬件设置 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">硬件设置</span>
            {gpuSupported && (<button className="btn btn-secondary btn-sm" onClick={async () => { setCudaChecking(true); try { const [ok] = await invoke<[boolean, string]>('check_cuda_available'); setCudaOk(ok); } catch(e: any) { setCudaOk(false); setLogs(p => [...p, { time: getTimeStr(), message: `GPU 检测异常: ${String(e)}`, status: 'error' }]); } setCudaChecking(false); }} disabled={cudaChecking} style={{ whiteSpace: 'nowrap' }}>{cudaChecking ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 14, height: 14 }} />} {isMac ? '检测 CoreML' : '检测 CUDA'}</button>)}
          </div>
          {cudaOk !== null && <div style={{ fontSize: 11, color: cudaOk ? '#4ade80' : '#f87171', padding: '6px 10px', borderRadius: 'var(--radius-sm)', background: cudaOk ? 'rgba(74,222,128,0.06)' : 'rgba(248,113,113,0.06)', lineHeight: 1.6, marginBottom: 'var(--space-3)' }}>{cudaOk ? (isMac ? '✓ CoreML 可用，Neural Engine 加速已就绪' : '✓ CUDA 可用，GPU 加速已就绪') : (isMac ? '✗ CoreML 不可用 — 详情请查看日志' : '✗ CUDA 不可用 — 详情请查看日志')}</div>}
          <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
            <div onClick={() => setUseGpu(false)} style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '8px', borderRadius: 'var(--radius-md)', border: `1.5px solid ${!useGpu ? '#fbbf24' : 'var(--color-border)'}`, background: !useGpu ? 'rgba(251,191,36,0.07)' : 'var(--color-bg-input)', cursor: 'pointer' }}><Cpu style={{ width: 16, height: 16, color: !useGpu ? '#fbbf24' : 'var(--color-text-tertiary)' }} /><span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: !useGpu ? '#fbbf24' : 'var(--color-text-tertiary)' }}>CPU</span></div>
            <div onClick={() => { if (gpuSupported) setUseGpu(true); }} style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '8px', borderRadius: 'var(--radius-md)', border: `1.5px solid ${useGpu ? '#4ade80' : 'var(--color-border)'}`, background: useGpu ? 'rgba(74,222,128,0.07)' : 'var(--color-bg-input)', cursor: gpuSupported ? 'pointer' : 'not-allowed', opacity: gpuSupported ? 1 : 0.4 }}><Gpu style={{ width: 16, height: 16, color: useGpu ? '#4ade80' : 'var(--color-text-tertiary)' }} /><span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600, color: useGpu ? '#4ade80' : 'var(--color-text-tertiary)' }}>GPU</span>{!gpuSupported && <span style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>(不可用)</span>}</div>
          </div>
        </div>

        {/* 其他设置 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">其他设置</span></div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 10px', borderRadius: 'var(--radius-sm)', marginBottom: 'var(--space-3)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }} onClick={() => setReplaceUnderscore(!replaceUnderscore)}>
              <div style={{ width: 16, height: 16, borderRadius: 4, minWidth: 16, border: `2px solid ${replaceUnderscore ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)'}`, background: replaceUnderscore ? 'var(--color-accent-primary)' : 'transparent', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{replaceUnderscore && <Check style={{ width: 10, height: 10, color: '#fff' }} />}</div>
              <span style={{ fontSize: 12, fontWeight: 600 }}>替换下划线为空格</span>
            </div>
            <div style={{ flex: 1 }} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 11 }}>
              <span style={{ color: 'var(--color-text-tertiary)', fontWeight: 500 }}>输出格式</span>
              {(['txt', 'json'] as const).map(fmt => (
                <button key={fmt} onClick={() => setOutputFormat(fmt)} style={{ padding: '2px 10px', borderRadius: 'var(--radius-sm)', border: `1px solid ${outputFormat === fmt ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: outputFormat === fmt ? 'rgba(124,92,252,0.08)' : 'transparent', color: outputFormat === fmt ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', fontSize: 11, fontWeight: 600, cursor: 'pointer' }}>.{fmt}</button>
              ))}
              {outputFormat==='json'&&(
                <select className="form-input" value={jsonSimplified?'simplified':'full'} onChange={e=>{const v=e.target.value==='simplified';setJsonSimplified(v);localStorage.setItem('tagger_json_simplified',String(v));}} style={{fontSize:10,height:24,padding:'0 6px',width:'auto',marginLeft:2}}>
                  <option value="full">完整格式</option>
                  <option value="simplified">简化格式</option>
                </select>
              )}
            </div>
          </div>
          <div style={{ marginBottom: 'var(--space-3)' }}>
            <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>排除标签</label>
            <input className="form-input" placeholder="tag1, tag2, tag3 ..." value={excludeTags} onChange={e => setExcludeTags(e.target.value)} style={{ width: '100%' }} />
            <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>打标结果中将排除这些标签，多个用逗号分隔</div>
          </div>
          <div>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
              <label className="form-label" style={{ fontSize: 11, margin: 0 }}>额外追加标签</label>
              <div style={{ display: 'flex', gap: 4 }}>
                {(['prepend', 'append'] as const).map(pos => (<button key={pos} onClick={() => setAppendPosition(pos)} style={{ padding: '2px 8px', borderRadius: 'var(--radius-sm)', border: `1px solid ${appendPosition === pos ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: appendPosition === pos ? 'rgba(124,92,252,0.08)' : 'transparent', color: appendPosition === pos ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', fontSize: 10, fontWeight: 600, cursor: 'pointer' }}>{pos === 'prepend' ? '最前面' : '最后面'}</button>))}
              </div>
            </div>
            <input className="form-input" placeholder="tag1, tag2, tag3 ..." value={appendTags} onChange={e => setAppendTags(e.target.value)} style={{ width: '100%' }} />
            <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>打标完成后追加这些标签到每个文件，多个用逗号分隔</div>
          </div>
        </div>
      </div>

      {/* 右栏 - 操作 + 进度 + 日志 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <ProcessButton processing={processing} onStart={handleStart}
          disabled={!inputPath || !selectedModel || enabled.size === 0}
          cancelCommand="cancel_tagging" forceCancelCommand="cancel_tagging"
          startText={cur && !cur.is_downloaded ? '下载并打标' : '开始打标'}
          startIcon={cur && !cur.is_downloaded ? <Download style={{ width: 18, height: 18 }} /> : undefined}
          processingText="打标中..."
          onCancelLog={addCancelLog} />


        <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasErr} onClearLogs={() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); }} />
      </div>

      <ConfirmModal
        open={!!deleteConfirm}
        onClose={() => setDeleteConfirm(null)}
        onConfirm={() => { if (deleteConfirm) handleDelete(deleteConfirm.id, deleteConfirm.name); }}
        title="删除模型"
        message={`确认删除自定义模型 "${deleteConfirm?.name}" 吗？\n模型文件也会被删除。`}
        confirmText="删除"
        variant="error"
      />
    </div>
  );
}

