import { Settings, Palette, Info, Sun, Moon, Monitor, Check, Activity, Languages, Trash2, Eye, EyeOff, ExternalLink, Loader2, Zap, FolderOpen, RotateCcw } from 'lucide-react';
import { useTheme } from '../components/ThemeProvider';
import { useState, useEffect } from 'react';
import { ConfirmModal, AlertModal } from '../components/Modal';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { getVersion } from '@tauri-apps/api/app';

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
  const [alertMsg, setAlertMsg] = useState('');

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

  useEffect(() => { loadCacheStats(); loadCachePath(); getVersion().then(v => setAppVersion(v)).catch(() => {}); }, []);

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
                  <select className="form-input" value={provider} onChange={e => { changeProvider(e.target.value); setTestResult(null); }}
                    style={{ fontSize: 12, height: 30, cursor: 'pointer', background: 'var(--color-bg-secondary)', color: 'var(--color-text-primary)', border: '1px solid var(--color-border)', borderRadius: 'var(--radius-sm)', padding: '0 10px', width: '100%' }}>
                    {providerOptions.map(opt => (
                      <option key={opt.value} value={opt.value}>{opt.label}</option>
                    ))}
                  </select>
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
      <AlertModal
        open={!!alertMsg}
        onClose={() => setAlertMsg('')}
        title="错误"
        message={alertMsg}
      />
    </>
  );
}
