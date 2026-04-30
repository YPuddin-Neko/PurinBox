import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Play, Loader2, Cpu, Zap, Download, Plus, Check, RefreshCw, ChevronDown } from 'lucide-react';
import ProgressLog, { LogEntry, getTimeStr } from './ProgressLog';

interface ModelInfo { id: string; name: string; description: string; input_size: number; is_builtin: boolean; is_downloaded: boolean; repo_id: string; }
interface ProcessResult { success_count: number; fail_count: number; total: number; errors: string[]; }
interface ProgressPayload { current: number; total: number; filename: string; status: string; message: string; }

const cats = [
  { key: 'general', label: '通用标签', default: true },
  { key: 'character', label: '角色标签', default: true },
  { key: 'rating', label: '评级标签', default: true },
  { key: 'artist', label: '作者标签', default: false },
  { key: 'copyright', label: '版权标签', default: false },
  { key: 'meta', label: '元信息标签', default: false },
];

export default function AiTaggerTab() {
  const [inputPath, setInputPath] = useState('');
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('');
  const [genTh, setGenTh] = useState(0.35);
  const [charTh, setCharTh] = useState(0.85);
  const [enabled, setEnabled] = useState<Set<string>>(new Set(cats.filter(c => c.default).map(c => c.key)));
  const [useGpu, setUseGpu] = useState(false);
  const [cudaOk, setCudaOk] = useState<boolean | null>(null);
  const [cudaChecking, setCudaChecking] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pCur, setPCur] = useState(0);
  const [pTot, setPTot] = useState(0);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isDone, setIsDone] = useState(false);
  const [hasErr, setHasErr] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [nId, setNId] = useState(''); const [nName, setNName] = useState(''); const [nRepo, setNRepo] = useState(''); const [nSize, setNSize] = useState(448);

  const load = useCallback(async () => {
    try { const l = await invoke<ModelInfo[]>('get_tagger_models'); setModels(l); if (l.length > 0 && !selectedModel) setSelectedModel(l[0].id); } catch {}
  }, [selectedModel]);
  useEffect(() => { load(); }, []);

  useEffect(() => {
    let u: UnlistenFn | null = null;
    listen<ProgressPayload>('tagger-progress', (e) => {
      const p = e.payload; setPCur(p.current); setPTot(p.total);
      if (p.total > 0) setProgress((p.current / p.total) * 100);
      if (p.status === 'done') setIsDone(true);
      if (p.status === 'error') setHasErr(true);
      setLogs(prev => [...prev, { time: getTimeStr(), message: p.message, status: p.status === 'done' ? 'info' : p.status === 'processing' ? 'info' : p.status as LogEntry['status'] }]);
    }).then(fn => { u = fn; });
    return () => { u?.(); };
  }, []);

  const cur = models.find(m => m.id === selectedModel);

  const handleStart = async () => {
    if (!inputPath || !selectedModel || enabled.size === 0) return;
    setProcessing(true); setProgress(0); setPCur(0); setPTot(0); setIsDone(false); setHasErr(false);
    setLogs([{ time: getTimeStr(), message: `开始打标 | 模型: ${cur?.name} | GPU: ${useGpu ? '是' : '否'}`, status: 'info' }]);
    try {
      await invoke<ProcessResult>('start_tagging', { options: { input_path: inputPath, model_id: selectedModel, general_threshold: genTh, character_threshold: charTh, enabled_categories: Array.from(enabled), use_gpu: useGpu } });
      await load();
    } catch (e: any) { setLogs(p => [...p, { time: getTimeStr(), message: `错误: ${String(e)}`, status: 'error' }]); setHasErr(true); setIsDone(true); }
    finally { setProcessing(false); }
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

        {/* 模型选择 - 下拉 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">打标模型</span>
            <button className="btn btn-ghost btn-sm" onClick={() => setShowAdd(!showAdd)}><Plus style={{ width: 14, height: 14 }} /> 添加</button>
          </div>
          {showAdd && (
            <div style={{ padding: 'var(--space-3)', borderRadius: 'var(--radius-md)', background: 'rgba(245,158,11,0.04)', border: '1px solid rgba(245,158,11,0.12)', marginBottom: 'var(--space-3)', display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
              <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                <input className="form-input" placeholder="模型ID" value={nId} onChange={e => setNId(e.target.value)} style={{ flex: 1 }} />
                <input className="form-input" placeholder="名称" value={nName} onChange={e => setNName(e.target.value)} style={{ flex: 1 }} />
              </div>
              <input className="form-input" placeholder="HuggingFace Repo (user/model)" value={nRepo} onChange={e => setNRepo(e.target.value)} />
              <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'center' }}>
                <input className="form-input" type="number" value={nSize} onChange={e => setNSize(Number(e.target.value))} style={{ width: 100 }} />
                <button className="btn btn-primary btn-sm" onClick={async () => { if (!nId||!nName||!nRepo) return; try { await invoke('add_custom_tagger_model', { id: nId, name: nName, repoId: nRepo, inputSize: nSize }); setShowAdd(false); setNId(''); setNName(''); setNRepo(''); await load(); } catch(e:any) { setLogs(p => [...p, { time: getTimeStr(), message: String(e), status: 'error' }]); } }}>添加</button>
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
            </div>
          )}
        </div>

        {/* 标签分类 + 阈值 紧凑排列 */}
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

        {/* GPU */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">加速设置</span></div>
          <div style={{ display: 'flex', gap: 'var(--space-3)' }}>
            <div onClick={() => setUseGpu(false)} style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '8px', borderRadius: 'var(--radius-md)', border: `1px solid ${!useGpu ? 'var(--color-border-active)' : 'var(--color-border)'}`, background: !useGpu ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)', cursor: 'pointer' }}>
              <Cpu style={{ width: 16, height: 16, color: !useGpu ? '#60a5fa' : 'var(--color-text-tertiary)' }} />
              <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>CPU</span>
            </div>
            <div onClick={() => setUseGpu(true)} style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '8px', borderRadius: 'var(--radius-md)', border: `1px solid ${useGpu ? 'rgba(74,222,128,0.5)' : 'var(--color-border)'}`, background: useGpu ? 'rgba(74,222,128,0.06)' : 'var(--color-bg-input)', cursor: 'pointer' }}>
              <Zap style={{ width: 16, height: 16, color: useGpu ? '#4ade80' : 'var(--color-text-tertiary)' }} />
              <span style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>GPU</span>
            </div>
            <button className="btn btn-secondary btn-sm" onClick={async () => { setCudaChecking(true); try { const [ok, detail] = await invoke<[boolean, string]>('check_cuda_available'); setCudaOk(ok); setLogs(p => [...p, { time: getTimeStr(), message: detail, status: ok ? 'success' : 'error' }]); } catch(e: any) { setCudaOk(false); setLogs(p => [...p, { time: getTimeStr(), message: String(e), status: 'error' }]); } setCudaChecking(false); }} disabled={cudaChecking} style={{ whiteSpace: 'nowrap' }}>
              {cudaChecking ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <RefreshCw style={{ width: 14, height: 14 }} />} 检测
            </button>
          </div>
          {cudaOk !== null && <div style={{ marginTop: 6, fontSize: 11, color: cudaOk ? '#4ade80' : '#f87171', padding: '6px 10px', borderRadius: 'var(--radius-sm)', background: cudaOk ? 'rgba(74,222,128,0.06)' : 'rgba(248,113,113,0.06)', lineHeight: 1.6, whiteSpace: 'pre-line' }}>{cudaOk ? '✓ CUDA 可用' : '✗ CUDA 不可用 — 详情请查看日志'}</div>}
        </div>
      </div>

      {/* 右栏 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <button className="btn btn-primary btn-lg" style={{ width: '100%', height: 48 }} onClick={handleStart}
          disabled={processing || !inputPath || !selectedModel || enabled.size === 0}>
          {processing ? <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> 打标中...</> :
            cur && !cur.is_downloaded ? <><Download style={{ width: 18, height: 18 }} /> 下载并打标</> : <><Play style={{ width: 18, height: 18 }} /> 开始打标</>}
        </button>

        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">当前设置</span></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 'var(--font-size-sm)' }}>
            {[['模型', cur?.name || '未选择', '#f59e0b'], ['状态', cur?.is_downloaded ? '已下载' : '首次使用自动下载', cur?.is_downloaded ? '#4ade80' : '#fbbf24'], ['分类', `${enabled.size} 类`, 'var(--color-text-primary)'], ['加速', useGpu ? 'GPU CUDA' : 'CPU', useGpu ? '#4ade80' : '#60a5fa'], ['通用阈值', genTh.toFixed(2), '#f59e0b'], ['角色阈值', charTh.toFixed(2), '#f59e0b']].map(([k, v, c]) => (
              <div key={k as string} style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ color: 'var(--color-text-tertiary)' }}>{k}</span>
                <span style={{ fontWeight: 600, color: c as string }}>{v}</span>
              </div>
            ))}
          </div>
        </div>

        {(logs.length > 0 || processing) && (
          <ProgressLog progress={progress} current={pCur} total={pTot} logs={logs} isDone={isDone} hasError={hasErr} onClearLogs={() => { setLogs([]); setProgress(0); setIsDone(false); setHasErr(false); }} />
        )}
      </div>
    </div>
  );
}
