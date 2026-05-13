import { Settings, Palette, Info, Sun, Moon, Monitor, Check, Activity, Languages, Trash2, Eye, EyeOff, ExternalLink, Loader2, Zap, FolderOpen, RotateCcw, Globe, Save, AlertTriangle, RefreshCw as RefreshIcon } from 'lucide-react';
import { useTheme } from '../components/ThemeProvider';
import { useState, useEffect } from 'react';
import { ConfirmModal, AlertModal } from '../components/Modal';
import CustomSelect from '../components/CustomSelect';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { getVersion } from '@tauri-apps/api/app';
import SystemMonitor from '../components/SystemMonitor';

const intervalOptions = [
  { value: 1000, label: '1 秒', desc: '实时' },
  { value: 2000, label: '2 秒', desc: '较快' },
  { value: 3000, label: '3 秒', desc: '默认' },
  { value: 5000, label: '5 秒', desc: '节能' },
  { value: 10000, label: '10 秒', desc: '低频' },
  { value: 0, label: '关闭', desc: '不检测' },
];

const providerOptions = [
  { value: 'google', label: 'Google 翻译' },
  { value: 'bing', label: '微软必应翻译' },
  { value: 'baidu', label: '百度翻译' },
  { value: 'youdao', label: '有道翻译' },
];

export default function SettingsPage() {
  const { mode, setMode, monitorInterval, setMonitorInterval } = useTheme();
  const [translateEnabled, setTranslateEnabled] = useState(() => localStorage.getItem('translate_enabled') === 'true');
  const [provider, setProvider] = useState(() => localStorage.getItem('translate_provider') || 'google');
  const [targetLang, setTargetLang] = useState(() => localStorage.getItem('translate_target_lang') || 'zh-CN');
  // Baidu
  const [baiduAppid, setBaiduAppid] = useState(() => localStorage.getItem('baidu_appid') || '');
  const [baiduKey, setBaiduKey] = useState(() => localStorage.getItem('baidu_key') || '');
  const [showBaiduKey, setShowBaiduKey] = useState(false);
  // Youdao
  const [youdaoAppKey, setYoudaoAppKey] = useState(() => localStorage.getItem('youdao_app_key') || '');
  const [youdaoAppSecret, setYoudaoAppSecret] = useState(() => localStorage.getItem('youdao_app_secret') || '');
  const [showYoudaoKey, setShowYoudaoKey] = useState(false);
  // Bing
  const [bingKey, setBingKey] = useState(() => localStorage.getItem('bing_key') || '');
  const [bingRegion, setBingRegion] = useState(() => localStorage.getItem('bing_region') || '');
  const [showBingKey, setShowBingKey] = useState(false);

  const [cacheStats, setCacheStats] = useState<{ total: number; db_size_bytes: number } | null>(null);
  const [clearing, setClearing] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const [cachePath, setCachePath] = useState<string>('');
  const [appVersion, setAppVersion] = useState('');
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [resetPythonConfirmOpen, setResetPythonConfirmOpen] = useState(false);
  const [resettingPython, setResettingPython] = useState(false);
  const [alertMsg, setAlertMsg] = useState('');
  const [pythonInfo, setPythonInfo] = useState<{ available: boolean; version: string; path: string } | null>(null);

  // 更新检查
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<{ has_update: boolean; latest_version: string; release_url: string } | null>(null);
  const [updateError, setUpdateError] = useState('');

  // 代理设置
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [llmProxy, setLlmProxy] = useState(false);
  const [proxyType, setProxyType] = useState('http');
  const [proxyHost, setProxyHost] = useState('127.0.0.1');
  const [proxyPort, setProxyPort] = useState(7890);
  const [proxyUser, setProxyUser] = useState('');
  const [proxyPass, setProxyPass] = useState('');
  const [showProxyPass, setShowProxyPass] = useState(false);
  const [proxySaving, setProxySaving] = useState(false);
  const [proxySaveMsg, setProxySaveMsg] = useState<{ text: string; ok: boolean } | null>(null);

  const toggleTranslate = (val: boolean) => { setTranslateEnabled(val); localStorage.setItem('translate_enabled', String(val)); };
  const changeProvider = (val: string) => { setProvider(val); localStorage.setItem('translate_provider', val); };

  const saveLS = (key: string, val: string, setter: (v: string) => void) => { setter(val); localStorage.setItem(key, val); };

  const handleTestTranslation = async () => {
    setTesting(true); setTestResult(null);
    try {
      const result = await invoke<string>('test_translation', {
        provider,
        baiduAppid: baiduAppid || '',
        baiduKey: baiduKey || '',
        youdaoAppKey: youdaoAppKey || '',
        youdaoAppSecret: youdaoAppSecret || '',
        bingKey: bingKey || '',
        bingRegion: bingRegion || '',
      });
      setTestResult({ ok: true, msg: result });
    } catch (e: any) {
      setTestResult({ ok: false, msg: e?.message || String(e) });
    } finally {
      setTesting(false);
    }
  };

  const loadCacheStats = async () => {
    try { setCacheStats(await invoke<{ total: number; db_size_bytes: number }>('get_translation_cache_stats')); } catch (e) { console.error(e); }
  };

  const handleClearCache = async () => {
    setClearConfirmOpen(true);
  };
  const doClearCache = async () => {
    setClearing(true);
    try { await invoke('clear_translation_cache'); await loadCacheStats(); } catch (e) { console.error(e); } finally { setClearing(false); }
  };

  const loadPythonInfo = async () => {
    try { setPythonInfo(await invoke<{ available: boolean; version: string; path: string }>('get_python_env_info')); } catch { setPythonInfo(null); }
  };

  useEffect(() => { loadCacheStats(); loadCachePath(); getVersion().then(v => setAppVersion(v)).catch(() => {}); loadProxySettings(); loadPythonInfo(); }, []);

  const loadProxySettings = async () => {
    try {
      const [enabled, llm, ptype, host, port, user, pass] = await invoke<[boolean, boolean, string, string, number, string, string]>('load_proxy_config');
      setProxyEnabled(enabled); setLlmProxy(llm); setProxyType(ptype); setProxyHost(host); setProxyPort(port); setProxyUser(user); setProxyPass(pass);
    } catch {}
  };

  const handleSaveProxy = async () => {
    setProxySaving(true); setProxySaveMsg(null);
    try {
      await invoke('save_proxy_config', { enabled: proxyEnabled, llmProxy, proxyType, host: proxyHost, port: proxyPort, username: proxyUser, password: proxyPass });
      setProxySaveMsg({ text: '保存成功', ok: true });
    } catch (e: any) { setProxySaveMsg({ text: e?.message || String(e), ok: false }); }
    finally { setProxySaving(false); setTimeout(() => setProxySaveMsg(null), 3000); }
  };

  const loadCachePath = async () => {
    try { setCachePath(await invoke<string>('get_cache_path')); } catch (e) { console.error(e); }
  };

  const handleChangeCachePath = async () => {
    const selected = await open({ directory: true, title: '选择翻译缓存目录' });
    if (selected && typeof selected === 'string') {
      try {
        await invoke('set_cache_path', { path: selected });
        setCachePath(selected);
        await loadCacheStats();
      } catch (e: any) { setAlertMsg(`设置缓存路径失败: ${e?.message || e}`); }
    }
  };

  const handleResetCachePath = async () => {
    try {
      await invoke<string>('set_cache_path', { path: '' });
      setCachePath(await invoke<string>('get_cache_path'));
      await loadCacheStats();
    } catch (e: any) { setAlertMsg(`重置失败: ${e?.message || e}`); }
  };

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  };

  const themeOptions = [
    { value: 'dark' as const, label: '深色模式', icon: <Moon style={{ width: 16, height: 16 }} />, desc: '深色背景，护眼模式' },
    { value: 'light' as const, label: '浅色模式', icon: <Sun style={{ width: 16, height: 16 }} />, desc: '浅色背景，明亮清晰' },
    { value: 'system' as const, label: '跟随系统', icon: <Monitor style={{ width: 16, height: 16 }} />, desc: '自动跟随操作系统设置' },
  ];

  // 密钥输入框组件
  const SecretInput = ({ value, onChange, placeholder, show, onToggle }: { value: string; onChange: (v: string) => void; placeholder: string; show: boolean; onToggle: () => void }) => (
    <div style={{ position: 'relative' }}>
      <input className="form-input" type={show ? 'text' : 'password'} value={value} onChange={e => onChange(e.target.value)}
        placeholder={placeholder} style={{ fontSize: 12, height: 32, paddingRight: 32 }} />
      <button onClick={onToggle} style={{
        position: 'absolute', right: 4, top: '50%', transform: 'translateY(-50%)',
        width: 24, height: 24, borderRadius: 4, border: 'none', background: 'none',
        cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: 'var(--color-text-tertiary)',
      }}>
        {show ? <EyeOff style={{ width: 12, height: 12 }} /> : <Eye style={{ width: 12, height: 12 }} />}
      </button>
    </div>
  );

  const LinkButton = ({ href, text }: { href: string; text: string }) => (
    <a href={href} target="_blank" rel="noreferrer" style={{
      fontSize: 10, color: '#60a5fa', textDecoration: 'none', display: 'inline-flex', alignItems: 'center', gap: 3,
    }}>
      {text} <ExternalLink style={{ width: 9, height: 9 }} />
    </a>
  );

  return (
    <>
    <div className="page" style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
      <div style={{ width: '100%', maxWidth: 640 }}>
        <div className="page-header">
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
            <Settings style={{ width: 28, height: 28, color: 'var(--color-text-secondary)' }} />
            <h1 className="page-title">设置</h1>
          </div>
          <p className="page-subtitle">配置工具箱的全局选项</p>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* System Monitor */}
          <SystemMonitor />

          {/* Appearance */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Palette style={{ width: 16, height: 16, color: 'var(--color-accent-primary)' }} />
                <span className="tool-panel-title">外观</span>
              </div>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 'var(--space-3)' }}>
              {themeOptions.map(opt => {
                const active = mode === opt.value;
                return (
                  <div key={opt.value} onClick={() => setMode(opt.value)} style={{
                    padding: 'var(--space-4)', borderRadius: 'var(--radius-md)',
                    border: `1.5px solid ${active ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
                    background: active ? 'rgba(124,92,252,0.06)' : 'var(--color-bg-input)',
                    cursor: 'pointer', transition: 'all 0.2s', position: 'relative',
                    display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 'var(--space-2)',
                  }}>
                    {active && (
                      <div style={{ position: 'absolute', top: 8, right: 8, width: 18, height: 18, borderRadius: '50%', background: 'var(--color-accent-primary)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                        <Check style={{ width: 11, height: 11, color: '#fff' }} />
                      </div>
                    )}
                    <div style={{ width: 36, height: 36, borderRadius: 'var(--radius-sm)', background: active ? 'rgba(124,92,252,0.12)' : 'rgba(255,255,255,0.04)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: active ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)' }}>
                      {opt.icon}
                    </div>
                    <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{opt.label}</span>
                    <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{opt.desc}</span>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Monitor Interval */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Activity style={{ width: 16, height: 16, color: '#4ade80' }} />
                <span className="tool-panel-title">系统监控</span>
              </div>
            </div>
            <div>
              <label className="form-label" style={{ marginBottom: 8 }}>检测间隔</label>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 'var(--space-2)' }}>
                {intervalOptions.map(opt => {
                  const active = monitorInterval === opt.value;
                  return (
                    <div key={opt.value} onClick={() => setMonitorInterval(opt.value)} style={{
                      padding: '8px 12px', borderRadius: 'var(--radius-sm)',
                      border: `1.5px solid ${active ? (opt.value === 0 ? 'rgba(248,113,113,0.5)' : 'var(--color-border-active)') : 'var(--color-border)'}`,
                      background: active ? (opt.value === 0 ? 'rgba(248,113,113,0.06)' : 'rgba(124,92,252,0.06)') : 'var(--color-bg-input)',
                      cursor: 'pointer', transition: 'all 0.15s',
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                    }}>
                      <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
                        <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{opt.label}</span>
                        <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>{opt.desc}</span>
                      </div>
                      {active && (
                        <div style={{ width: 16, height: 16, borderRadius: '50%', background: opt.value === 0 ? '#f87171' : 'var(--color-accent-primary)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                          <Check style={{ width: 10, height: 10, color: '#fff' }} />
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
              <p style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 8, lineHeight: 1.6 }}>
                控制顶栏和首页系统性能指标（CPU/RAM/GPU）的刷新频率。间隔越短数据越实时，但会略微增加系统开销。选择「关闭」将完全停止检测。
              </p>
            </div>
          </div>

          {/* Proxy */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Globe style={{ width: 16, height: 16, color: '#f59e0b' }} />
                <span className="tool-panel-title">网络代理</span>
              </div>
              <div style={{ display: 'flex', gap: 6 }}>
                {proxySaveMsg && <span style={{ fontSize: 10, color: proxySaveMsg.ok ? '#4ade80' : '#f87171' }}>{proxySaveMsg.text}</span>}
                <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={handleSaveProxy} disabled={proxySaving}>
                  <Save style={{ width: 12, height: 12 }} /> {proxySaving ? '保存中...' : '保存'}
                </button>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* 启用开关 + LLM 开关 + 代理类型 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 10 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 12px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>启用代理</div>
                    <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>全局网络代理</div>
                  </div>
                  <div onClick={() => setProxyEnabled(!proxyEnabled)} style={{
                    width: 36, height: 20, borderRadius: 10, cursor: 'pointer', transition: 'all 0.2s',
                    background: proxyEnabled ? 'var(--color-accent-primary)' : 'var(--color-border)',
                    position: 'relative',
                  }}>
                    <div style={{
                      width: 14, height: 14, borderRadius: '50%', background: '#fff',
                      position: 'absolute', top: 3,
                      left: proxyEnabled ? 19 : 3, transition: 'left 0.2s',
                    }} />
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 12px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>LLM 代理</div>
                    <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>允许LLM接口是否走代理</div>
                  </div>
                  <div onClick={() => setLlmProxy(!llmProxy)} style={{
                    width: 36, height: 20, borderRadius: 10, cursor: 'pointer', transition: 'all 0.2s',
                    background: llmProxy ? 'var(--color-accent-primary)' : 'var(--color-border)',
                    position: 'relative',
                  }}>
                    <div style={{
                      width: 14, height: 14, borderRadius: '50%', background: '#fff',
                      position: 'absolute', top: 3,
                      left: llmProxy ? 19 : 3, transition: 'left 0.2s',
                    }} />
                  </div>
                </div>
                <div style={{ padding: '10px 12px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 5 }}>代理类型</div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    {(['http', 'socks5'] as const).map(t => (
                      <button key={t} onClick={() => setProxyType(t)} style={{
                        flex: 1, padding: '4px 0', borderRadius: 'var(--radius-sm)', fontSize: 11, fontWeight: 600, cursor: 'pointer',
                        border: `1.5px solid ${proxyType === t ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                        background: proxyType === t ? 'rgba(124,92,252,0.08)' : 'transparent',
                        color: proxyType === t ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)',
                      }}>{t.toUpperCase()}</button>
                    ))}
                  </div>
                </div>
              </div>

              {/* 地址 + 端口 */}
              <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
                <div style={{ flex: 1 }}>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>代理地址</label>
                  <input className="form-input" placeholder="127.0.0.1" value={proxyHost} onChange={e => setProxyHost(e.target.value)} style={{ height: 32 }} />
                </div>
                <div style={{ width: 90 }}>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>端口</label>
                  <input className="form-input" type="number" placeholder="7890" value={proxyPort} onChange={e => setProxyPort(Number(e.target.value))} style={{ height: 32 }} />
                </div>
              </div>

              {/* 认证（可选） */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>用户名（可选）</label>
                  <input className="form-input" placeholder="无需认证留空" value={proxyUser} onChange={e => setProxyUser(e.target.value)} style={{ height: 32 }} />
                </div>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>密码（可选）</label>
                  <div style={{ position: 'relative' }}>
                    <input className="form-input" type={showProxyPass ? 'text' : 'password'} placeholder="无需认证留空" value={proxyPass} onChange={e => setProxyPass(e.target.value)} style={{ paddingRight: 32, height: 32 }} />
                    <button onClick={() => setShowProxyPass(!showProxyPass)} style={{ position: 'absolute', right: 6, top: '50%', transform: 'translateY(-50%)', background: 'none', border: 'none', cursor: 'pointer', color: 'var(--color-text-tertiary)', display: 'flex', padding: 2 }}>
                      {showProxyPass ? <EyeOff style={{ width: 13, height: 13 }} /> : <Eye style={{ width: 13, height: 13 }} />}
                    </button>
                  </div>
                </div>
              </div>

              <p style={{ fontSize: 10, color: 'var(--color-text-tertiary)', lineHeight: 1.6, margin: 0 }}>
                代理将应用于所有网络请求，包括翻译 API、LLM 接口、模型下载等。修改后请点击“保存”，新任务将自动使用新配置。
              </p>
            </div>
          </div>

          {/* Translation */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Languages style={{ width: 16, height: 16, color: '#60a5fa' }} />
                <span className="tool-panel-title">翻译设置</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {/* 开关 + 供应商 同一行 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, padding: '12px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>启用翻译</div>
                    <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 1 }}>启用后支持标签管理的 Tag 翻译功能</div>
                  </div>
                  <div onClick={() => toggleTranslate(!translateEnabled)} style={{
                    width: 40, height: 22, borderRadius: 11, cursor: 'pointer', transition: 'all 0.2s',
                    background: translateEnabled ? 'var(--color-accent-primary)' : 'var(--color-bg-tertiary, rgba(255,255,255,0.06))',
                    border: `1px solid ${translateEnabled ? 'var(--color-accent-primary)' : 'var(--color-border)'}`,
                    position: 'relative', flexShrink: 0,
                  }}>
                    <div style={{
                      width: 16, height: 16, borderRadius: '50%', background: '#fff',
                      position: 'absolute', top: 2, left: translateEnabled ? 21 : 2,
                      transition: 'left 0.2s', boxShadow: '0 1px 3px rgba(0,0,0,0.2)',
                    }} />
                  </div>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 6, padding: '12px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <label style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>供应商设置</label>
                    <button onClick={handleTestTranslation} disabled={testing}
                      style={{ fontSize: 10, color: testResult ? (testResult.ok ? '#4ade80' : '#f87171') : '#60a5fa', background: 'none', border: 'none', cursor: testing ? 'wait' : 'pointer', display: 'flex', alignItems: 'center', gap: 3, padding: 0 }}>
                      {testing ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} /> : <Zap style={{ width: 10, height: 10 }} />}
                      {testing ? '测试中...' : testResult ? (testResult.ok ? testResult.msg : '测试失败') : '测试可用性'}
                    </button>
                  </div>
                  <CustomSelect value={provider} onChange={v => { changeProvider(v); setTestResult(null); }} options={providerOptions} compact />
                </div>
              </div>

              {/* 目标语言 */}
              <div style={{ padding: '12px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>目标语言</div>
                    <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 1 }}>翻译结果的目标语言</div>
                  </div>
                  <CustomSelect value={targetLang}
                    onChange={v => { setTargetLang(v); localStorage.setItem('translate_target_lang', v); }}
                    options={[
                      { value: 'zh-CN', label: '中文' },
                      { value: 'en', label: 'English' },
                      { value: 'ja', label: '日本語' },
                      { value: 'ko', label: '한국어' },
                    ]}
                    compact
                    style={{ width: 120 }}
                  />
                </div>
              </div>

              {/* 供应商配置 */}
              {provider === 'baidu' && (
                <div style={{ padding: '14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 10 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                    百度翻译 API 配置
                    <LinkButton href="https://fanyi-api.baidu.com" text="申请密钥" />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>APP ID</label>
                    <input className="form-input" value={baiduAppid} onChange={e => saveLS('baidu_appid', e.target.value, setBaiduAppid)}
                      placeholder="输入百度翻译 APP ID" style={{ fontSize: 12, height: 32 }} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>密钥</label>
                    <SecretInput value={baiduKey} onChange={v => saveLS('baidu_key', v, setBaiduKey)}
                      placeholder="输入百度翻译密钥" show={showBaiduKey} onToggle={() => setShowBaiduKey(!showBaiduKey)} />
                  </div>
                </div>
              )}

              {provider === 'youdao' && (
                <div style={{ padding: '14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 10 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                    有道翻译 API 配置
                    <LinkButton href="https://ai.youdao.com" text="申请密钥" />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>应用 ID</label>
                    <input className="form-input" value={youdaoAppKey} onChange={e => saveLS('youdao_app_key', e.target.value, setYoudaoAppKey)}
                      placeholder="输入有道翻译应用 ID" style={{ fontSize: 12, height: 32 }} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>应用密钥</label>
                    <SecretInput value={youdaoAppSecret} onChange={v => saveLS('youdao_app_secret', v, setYoudaoAppSecret)}
                      placeholder="输入有道翻译应用密钥" show={showYoudaoKey} onToggle={() => setShowYoudaoKey(!showYoudaoKey)} />
                  </div>
                </div>
              )}

              {provider === 'bing' && (
                <div style={{ padding: '14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 10 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                    微软必应翻译 API 配置
                    <LinkButton href="https://portal.azure.com/#create/Microsoft.CognitiveServicesTextTranslation" text="申请密钥" />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>订阅密钥</label>
                    <SecretInput value={bingKey} onChange={v => saveLS('bing_key', v, setBingKey)}
                      placeholder="输入 Ocp-Apim-Subscription-Key" show={showBingKey} onToggle={() => setShowBingKey(!showBingKey)} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>区域（可选）</label>
                    <input className="form-input" value={bingRegion} onChange={e => saveLS('bing_region', e.target.value, setBingRegion)}
                      placeholder="如 eastasia、global 等，可留空" style={{ fontSize: 12, height: 32 }} />
                  </div>
                </div>
              )}

              {/* 缓存路径 */}
              <div style={{ padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>缓存路径</div>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <button className="btn btn-secondary" onClick={handleChangeCachePath}
                      style={{ fontSize: 10, height: 24, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <FolderOpen style={{ width: 10, height: 10 }} />修改
                    </button>
                    <button className="btn btn-secondary" onClick={handleResetCachePath} title="重置为默认路径"
                      style={{ fontSize: 10, height: 24, padding: '0 6px', display: 'flex', alignItems: 'center' }}>
                      <RotateCcw style={{ width: 10, height: 10 }} />
                    </button>
                  </div>
                </div>
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', wordBreak: 'break-all', lineHeight: 1.5, background: 'var(--color-bg-secondary)', padding: '6px 8px', borderRadius: 4 }}>
                  {cachePath || '加载中...'}
                </div>
              </div>

              {/* 缓存管理 */}
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>翻译缓存</div>
                  <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    {cacheStats ? `${cacheStats.total} 条记录 · ${formatSize(cacheStats.db_size_bytes)}` : '加载中...'}
                  </div>
                </div>
                <button className="btn btn-secondary" onClick={handleClearCache} disabled={clearing || !cacheStats || cacheStats.total === 0}
                  style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4, color: '#f87171' }}>
                  <Trash2 style={{ width: 12, height: 12 }} />
                  {clearing ? '清理中...' : '清空缓存'}
                </button>
              </div>
            </div>
          </div>

          {/* 高级设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <AlertTriangle style={{ width: 16, height: 16, color: '#fbbf24' }} />
                <span className="tool-panel-title">高级设置</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* Python 环境信息 */}
              <div style={{ padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div>
                    <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>Python 环境</div>
                    <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                      {pythonInfo === null ? '加载中...' : pythonInfo.available
                        ? <><span style={{ color: '#4ade80' }}>✓</span> {pythonInfo.version}</>
                        : <span style={{ color: '#f87171' }}>未检测到</span>
                      }
                    </div>
                    {pythonInfo?.available && (
                      <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2, wordBreak: 'break-all', lineHeight: 1.5 }}>
                        {pythonInfo.path}
                      </div>
                    )}
                  </div>
                </div>
              </div>

              {/* 重置 Python */}
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <div>
                  <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>重置 Python 环境</div>
                  <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>删除软件管理的 Python 虚拟环境和独立版本，下次使用时会自动重新配置</div>
                </div>
                <button className="btn" onClick={() => setResetPythonConfirmOpen(true)} disabled={resettingPython}
                  style={{ background: 'rgba(248,113,113,0.1)', color: '#f87171', border: '1px solid rgba(248,113,113,0.3)', fontSize: 12, padding: '6px 14px', gap: 6 }}>
                  {resettingPython ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <RotateCcw style={{ width: 14, height: 14 }} />}
                  重置
                </button>
              </div>
            </div>
          </div>

          {/* About */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Info style={{ width: 16, height: 16, color: 'var(--color-text-tertiary)' }} />
                <span className="tool-panel-title">关于</span>
              </div>
            </div>
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.8 }}>
              <p><strong>PurinBox</strong> · v{appVersion}</p>
              <p>基于 Tauri 2 + React + TypeScript 构建</p>
              <p style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <a href="https://github.com/YPuddin-Neko/PurinBox" target="_blank" rel="noreferrer"
                  style={{ color: '#60a5fa', textDecoration: 'none', display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  GitHub <ExternalLink style={{ width: 12, height: 12 }} />
                </a>
              </p>
              {/* Update check */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 8, padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>检查更新</div>
                  <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    {updateChecking ? '正在检查...'
                      : updateError ? `检查失败: ${updateError}`
                      : updateResult
                        ? updateResult.has_update
                          ? <span style={{ color: '#ef4444' }}>发现新版本 v{updateResult.latest_version}</span>
                          : <span style={{ color: '#4ade80' }}>已是最新版本</span>
                        : '点击检查是否有新版本'}
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 6 }}>
                  {updateResult?.has_update && updateResult.release_url && (
                    <a href={updateResult.release_url} target="_blank" rel="noreferrer"
                      className="btn btn-primary" style={{ fontSize: 11, height: 28, padding: '0 12px', textDecoration: 'none', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <ExternalLink style={{ width: 11, height: 11 }} /> 前往下载
                    </a>
                  )}
                  <button className="btn btn-secondary" disabled={updateChecking}
                    onClick={async () => {
                      setUpdateChecking(true); setUpdateError(''); setUpdateResult(null);
                      try {
                        const r = await invoke<{ has_update: boolean; latest_version: string; release_url: string }>('check_for_updates');
                        setUpdateResult(r);
                      } catch (e: any) { setUpdateError(String(e)); }
                      finally { setUpdateChecking(false); }
                    }}
                    style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4 }}>
                    {updateChecking ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <RefreshIcon style={{ width: 12, height: 12 }} />}
                    {updateChecking ? '检查中...' : '检查更新'}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

      <ConfirmModal
        open={clearConfirmOpen}
        onClose={() => setClearConfirmOpen(false)}
        onConfirm={doClearCache}
        title="清空缓存"
        message="确定清空所有翻译缓存？下次翻译将重新请求翻译接口。"
        confirmText="清空"
        variant="warning"
      />
      <ConfirmModal
        open={resetPythonConfirmOpen}
        onClose={() => setResetPythonConfirmOpen(false)}
        onConfirm={async () => {
          setResetPythonConfirmOpen(false);
          setResettingPython(true);
          try {
            await invoke('reset_python_env');
            setAlertMsg('Python 环境已重置');
            loadPythonInfo();
          } catch (e: any) {
            setAlertMsg(`重置失败: ${e}`);
          } finally {
            setResettingPython(false);
          }
        }}
        title="重置 Python 环境"
        message="确定重置 Python 环境？将删除虚拟环境和下载的独立 Python."
        confirmText="重置"
        variant="error"
      />
      <AlertModal
        open={!!alertMsg}
        onClose={() => setAlertMsg('')}
        title="提示"
        message={alertMsg}
      />
    </>
  );
}
