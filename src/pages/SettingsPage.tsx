import { Settings, Info, Check, Activity, Languages, Trash2, Eye, EyeOff, ExternalLink, Loader2, Zap, FolderOpen, RotateCcw, Globe, Save, AlertTriangle, RefreshCw as RefreshIcon, Database, Download, Upload, X } from 'lucide-react';
import { useTheme } from '../components/ThemeProvider';
import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import { ConfirmModal, AlertModal } from '../components/Modal';
import CustomSelect from '../components/CustomSelect';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { getVersion } from '@tauri-apps/api/app';
import SystemMonitor from '../components/SystemMonitor';

export default function SettingsPage() {
  const { t } = useTranslation();
  const { monitorInterval, setMonitorInterval } = useTheme();

  const intervalOptions = [
    { value: 1000, label: t('settings.monitorSec', { n: 1 }), desc: t('settings.monitorRealtime') },
    { value: 2000, label: t('settings.monitorSec', { n: 2 }), desc: t('settings.monitorFast') },
    { value: 3000, label: t('settings.monitorSec', { n: 3 }), desc: t('settings.monitorDefault') },
    { value: 5000, label: t('settings.monitorSec', { n: 5 }), desc: t('settings.monitorSave') },
    { value: 10000, label: t('settings.monitorSec', { n: 10 }), desc: t('settings.monitorLow') },
    { value: 0, label: t('settings.monitorOff'), desc: t('settings.monitorNone') },
  ];

  const providerOptions = [
    { value: 'google', label: t('settings.providerGoogle') },
    { value: 'bing', label: t('settings.providerBing') },
    { value: 'baidu', label: t('settings.providerBaidu') },
    { value: 'youdao', label: t('settings.providerYoudao') },
  ];
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

  const [cacheStats, setCacheStats] = useState<{ total: number; db_size_bytes: number; zh_cn: number; ja: number; ko: number } | null>(null);
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

  // 标签数据库
  const [tagDbStats, setTagDbStats] = useState<{ total_tags: number; translated_tags: number; db_size_bytes: number; has_data: boolean; source_file: string; import_date: string } | null>(null);
  const [tagDbDownloading, setTagDbDownloading] = useState(false);
  const [tagDbTranslating, setTagDbTranslating] = useState(false);
  const [translateHover, setTranslateHover] = useState(false);
  const [tagDbProgress, setTagDbProgress] = useState('');
  const [tagDbClearConfirm, setTagDbClearConfirm] = useState(false);
  const [tagDbLatest, setTagDbLatest] = useState('');
  const [tagDbChecking, setTagDbChecking] = useState(false);

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
    try { setCacheStats(await invoke<{ total: number; db_size_bytes: number; zh_cn: number; ja: number; ko: number }>('get_translation_cache_stats')); } catch (e) { console.error(e); }
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

  const loadTagDbStats = useCallback(async () => {
    try {
      const targetLang = localStorage.getItem('translate_target_lang') || 'zh-CN';
      setTagDbStats(await invoke<{ total_tags: number; translated_tags: number; db_size_bytes: number; has_data: boolean; source_file: string; import_date: string }>('get_tag_db_stats', { targetLang }));
    } catch (e) { console.error(e); }
  // Empty deps is intentional: localStorage is read fresh each call, no React state dependency needed
  }, []);

  const handleDownloadTagDb = async () => {
    // 如果已有数据，先检查是否有新版本
    if (tagDbStats?.has_data) {
      setTagDbDownloading(true); setTagDbProgress(t('settings.checkingUpdate'));
      try {
        const latest = await invoke<string>('check_tag_db_update');
        setTagDbLatest(latest);
        if (latest === tagDbStats.source_file) {
          setTagDbProgress(t('settings.alreadyLatestVersion'));
          setTagDbDownloading(false);
          return;
        }
      } catch (e: any) {
        setTagDbProgress(`${t('settings.checkFailed')}: ${e?.message || e}`);
        setTagDbDownloading(false);
        return;
      }
    } else {
      setTagDbDownloading(true);
    }
    setTagDbProgress(t('settings.downloading'));
    try { await invoke('download_danbooru_tags'); await loadTagDbStats(); } catch (e: any) { setTagDbProgress(`${t('common.failed')}: ${e?.message || e}`); }
    finally { setTagDbDownloading(false); }
  };

  const handleTranslateTagDb = async () => {
    setTagDbTranslating(true); setTagDbProgress(t('settings.translating'));
    try { await invoke('translate_tag_db', { targetLang: localStorage.getItem('translate_target_lang') || 'zh-CN' }); await loadTagDbStats(); } catch (e: any) { setTagDbProgress(`${t('common.failed')}: ${e?.message || e}`); }
    finally { setTagDbTranslating(false); }
  };

  const handleClearTagDb = async () => {
    try { await invoke('clear_tag_db'); await loadTagDbStats(); setTagDbProgress(''); } catch (e: any) { console.error(e); }
  };

  useEffect(() => {
    loadCacheStats(); loadCachePath(); loadTagDbStats();
    getVersion().then(v => setAppVersion(v)).catch(() => {});
    loadProxySettings(); loadPythonInfo();
    // 恢复后端忙碌状态
    invoke<[boolean, boolean]>('is_tag_db_busy').then(([downloading, translating]) => {
      setTagDbDownloading(downloading);
      setTagDbTranslating(translating);
    }).catch(() => {});
    const unlisten = listen<{ status: string; message: string; current: number; total: number }>('tag-db-progress', (e) => {
      setTagDbProgress(e.payload.message);
      if (e.payload.status === 'translating') {
        loadTagDbStats();
        loadCacheStats();
      }
      if (e.payload.status === 'done') {
        setTagDbDownloading(false);
        setTagDbTranslating(false);
        loadTagDbStats();
        loadCacheStats();
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

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
      setProxySaveMsg({ text: t('settings.proxySaved'), ok: true });
    } catch (e: any) { setProxySaveMsg({ text: e?.message || String(e), ok: false }); }
    finally { setProxySaving(false); setTimeout(() => setProxySaveMsg(null), 3000); }
  };

  const loadCachePath = async () => {
    try { setCachePath(await invoke<string>('get_cache_path')); } catch (e) { console.error(e); }
  };

  const handleChangeCachePath = async () => {
    const selected = await open({ directory: true, title: t('settings.selectCacheDir') });
    if (selected && typeof selected === 'string') {
      try {
        await invoke('set_cache_path', { path: selected });
        setCachePath(selected);
        await loadCacheStats();
      } catch (e: any) { setAlertMsg(`${t('settings.setCachePathFailed')}: ${e?.message || e}`); }
    }
  };

  const handleResetCachePath = async () => {
    try {
      await invoke<string>('set_cache_path', { path: '' });
      setCachePath(await invoke<string>('get_cache_path'));
      await loadCacheStats();
    } catch (e: any) { setAlertMsg(`${t('settings.resetFailed')}: ${e?.message || e}`); }
  };

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  };



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
            <h1 className="page-title">{t('settings.title')}</h1>
          </div>
          <p className="page-subtitle">{t('settings.aboutDesc')}</p>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* System Monitor - 仅在监控开启时显示 */}
          {monitorInterval > 0 && <SystemMonitor />}



          {/* Monitor Interval */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Activity style={{ width: 16, height: 16, color: '#4ade80' }} />
                <span className="tool-panel-title">{t('settings.monitor')}</span>
              </div>
            </div>
            <div>
              <label className="form-label" style={{ marginBottom: 8 }}>{t('settings.monitorInterval')}</label>
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
            </div>
          </div>

          {/* Proxy */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Globe style={{ width: 16, height: 16, color: '#f59e0b' }} />
                <span className="tool-panel-title">{t('settings.proxy')}</span>
              </div>
              <div style={{ display: 'flex', gap: 6 }}>
                {proxySaveMsg && <span style={{ fontSize: 10, color: proxySaveMsg.ok ? '#4ade80' : '#f87171' }}>{proxySaveMsg.text}</span>}
                <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={handleSaveProxy} disabled={proxySaving}>
                  <Save style={{ width: 12, height: 12 }} /> {proxySaving ? t('settings.proxySaving') : t('common.save')}
                </button>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* 启用开关 + LLM 开关 + 代理类型 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 10 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 12px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.proxyEnabled')}</div>
                    <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>{t('settings.proxy')}</div>
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
                    <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.proxyLlm')}</div>
                    <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>{t('settings.proxyLlmDesc')}</div>
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
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 5 }}>{t('settings.proxyType')}</div>
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
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('settings.proxyHost')}</label>
                  <input className="form-input" placeholder="127.0.0.1" value={proxyHost} onChange={e => setProxyHost(e.target.value)} style={{ height: 32 }} />
                </div>
                <div style={{ width: 90 }}>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('settings.proxyPort')}</label>
                  <input className="form-input" type="number" placeholder="7890" value={proxyPort} onChange={e => setProxyPort(Number(e.target.value))} style={{ height: 32 }} />
                </div>
              </div>

              {/* 认证（可选） */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('settings.proxyUserOptional')}</label>
                  <input className="form-input" placeholder={t('settings.proxyNoAuthHint')} value={proxyUser} onChange={e => setProxyUser(e.target.value)} style={{ height: 32 }} />
                </div>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('settings.proxyPassOptional')}</label>
                  <div style={{ position: 'relative' }}>
                    <input className="form-input" type={showProxyPass ? 'text' : 'password'} placeholder={t('settings.proxyNoAuthHint')} value={proxyPass} onChange={e => setProxyPass(e.target.value)} style={{ paddingRight: 32, height: 32 }} />
                    <button onClick={() => setShowProxyPass(!showProxyPass)} style={{ position: 'absolute', right: 6, top: '50%', transform: 'translateY(-50%)', background: 'none', border: 'none', cursor: 'pointer', color: 'var(--color-text-tertiary)', display: 'flex', padding: 2 }}>
                      {showProxyPass ? <EyeOff style={{ width: 13, height: 13 }} /> : <Eye style={{ width: 13, height: 13 }} />}
                    </button>
                  </div>
                </div>
              </div>

              <p style={{ fontSize: 10, color: 'var(--color-text-tertiary)', lineHeight: 1.6, margin: 0 }}>
                {t('settings.proxyDesc')}
              </p>
            </div>
          </div>

          {/* Translation */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Languages style={{ width: 16, height: 16, color: '#60a5fa' }} />
                <span className="tool-panel-title">{t('settings.translation')}</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {/* 开关 + 供应商 同一行 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, padding: '12px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.enableTranslation')}</div>
                    <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 1 }}>{t('settings.enableTranslationDesc')}</div>
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
                    <label style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.translationProvider')}</label>
                    <button onClick={handleTestTranslation} disabled={testing}
                      style={{ fontSize: 10, color: testResult ? (testResult.ok ? '#4ade80' : '#f87171') : '#60a5fa', background: 'none', border: 'none', cursor: testing ? 'wait' : 'pointer', display: 'flex', alignItems: 'center', gap: 3, padding: 0 }}>
                      {testing ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} /> : <Zap style={{ width: 10, height: 10 }} />}
                      {testing ? t('settings.testing') : testResult ? (testResult.ok ? testResult.msg : t('common.failed')) : t('settings.testTranslation')}
                    </button>
                  </div>
                  <CustomSelect value={provider} onChange={v => { changeProvider(v); setTestResult(null); }} options={providerOptions} compact />
                </div>
              </div>

              {/* 目标语言 */}
              <div style={{ padding: '12px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.targetLanguage')}</div>
                    <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 1 }}>{t('settings.targetLanguageDesc')}</div>
                  </div>
                  <CustomSelect value={targetLang}
                    onChange={v => { setTargetLang(v); localStorage.setItem('translate_target_lang', v); loadTagDbStats(); }}
                    options={[
                      { value: 'zh-CN', label: t('settings.langZhCN') },
                      { value: 'ja', label: t('settings.langJa') },
                      { value: 'ko', label: t('settings.langKo') },
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
                    {t('settings.providerBaidu')} API
                    <LinkButton href="https://fanyi-api.baidu.com" text={t('settings.applyLink')} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{t('settings.baiduAppId')}</label>
                    <input className="form-input" value={baiduAppid} onChange={e => saveLS('baidu_appid', e.target.value, setBaiduAppid)}
                      placeholder={t('settings.baiduAppIdPlaceholder')} style={{ fontSize: 12, height: 32 }} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{t('settings.baiduKey')}</label>
                    <SecretInput value={baiduKey} onChange={v => saveLS('baidu_key', v, setBaiduKey)}
                      placeholder={t('settings.baiduKeyPlaceholder')} show={showBaiduKey} onToggle={() => setShowBaiduKey(!showBaiduKey)} />
                  </div>
                </div>
              )}

              {provider === 'youdao' && (
                <div style={{ padding: '14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 10 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                    {t('settings.providerYoudao')} API
                    <LinkButton href="https://ai.youdao.com" text={t('settings.applyLink')} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{t('settings.youdaoAppKey')}</label>
                    <input className="form-input" value={youdaoAppKey} onChange={e => saveLS('youdao_app_key', e.target.value, setYoudaoAppKey)}
                      placeholder={t('settings.youdaoAppKeyPlaceholder')} style={{ fontSize: 12, height: 32 }} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{t('settings.youdaoAppSecret')}</label>
                    <SecretInput value={youdaoAppSecret} onChange={v => saveLS('youdao_app_secret', v, setYoudaoAppSecret)}
                      placeholder={t('settings.youdaoAppSecretPlaceholder')} show={showYoudaoKey} onToggle={() => setShowYoudaoKey(!showYoudaoKey)} />
                  </div>
                </div>
              )}

              {provider === 'bing' && (
                <div style={{ padding: '14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 10 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                    {t('settings.providerBing')} API
                    <LinkButton href="https://portal.azure.com/#create/Microsoft.CognitiveServicesTextTranslation" text={t('settings.applyLink')} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{t('settings.bingKey')}</label>
                    <SecretInput value={bingKey} onChange={v => saveLS('bing_key', v, setBingKey)}
                      placeholder={t('settings.bingKeyPlaceholder')} show={showBingKey} onToggle={() => setShowBingKey(!showBingKey)} />
                  </div>
                  <div>
                    <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{t('settings.bingRegion')}</label>
                    <input className="form-input" value={bingRegion} onChange={e => saveLS('bing_region', e.target.value, setBingRegion)}
                      placeholder={t('settings.bingRegionPlaceholder')} style={{ fontSize: 12, height: 32 }} />
                  </div>
                </div>
              )}

              {/* 缓存路径 */}
              <div style={{ padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.cachePath')}</div>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <button className="btn btn-secondary" onClick={handleChangeCachePath}
                      style={{ fontSize: 10, height: 24, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <FolderOpen style={{ width: 10, height: 10 }} />{t('settings.cacheModify')}
                    </button>
                    <button className="btn btn-secondary" onClick={handleResetCachePath} title={t('settings.cacheReset')}
                      style={{ fontSize: 10, height: 24, padding: '0 6px', display: 'flex', alignItems: 'center' }}>
                      <RotateCcw style={{ width: 10, height: 10 }} />
                    </button>
                  </div>
                </div>
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', wordBreak: 'break-all', lineHeight: 1.5, background: 'var(--color-bg-secondary)', padding: '6px 8px', borderRadius: 4 }}>
                  {cachePath || t('common.loading')}
                </div>
              </div>

              <div style={{ padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.translationCache')}</div>
                    <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                      {cacheStats ? `${t('settings.cacheRecords', { count: cacheStats.total })} · ${formatSize(cacheStats.db_size_bytes)}` : t('common.loading')}
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    <button className="btn btn-ghost btn-sm" title={t('settings.exportCsv')} onClick={async () => {
                      try {
                        const { save } = await import('@tauri-apps/plugin-dialog');
                        const path = await save({ title: t('settings.exportCsv'), defaultPath: 'translations.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
                        if (path) {
                          const count = await invoke<number>('export_translation_csv', { path });
                          alert(t('settings.exportSuccess', { count }));
                        }
                      } catch (e: any) { alert(t('settings.exportFailed') + ': ' + e); }
                    }} style={{ fontSize: 10, height: 26, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <Download style={{ width: 11, height: 11 }} /> {t('common.export')}
                    </button>
                    <button className="btn btn-ghost btn-sm" title={t('settings.importCsv')} onClick={async () => {
                      try {
                        const { open: dialogOpen } = await import('@tauri-apps/plugin-dialog');
                        const path = await dialogOpen({ title: t('settings.importCsv'), filters: [{ name: 'CSV', extensions: ['csv'] }] });
                        if (path) {
                          const [imported, skipped, errors] = await invoke<[number, number, string]>('import_translation_csv', { path });
                          let msg = t('settings.importSuccess', { imported });
                          if (skipped > 0) msg += t('settings.importSkipped', { skipped });
                          if (errors) msg += `\n\n${errors}`;
                          alert(msg);
                          loadCacheStats();
                        }
                      } catch (e: any) { alert(t('settings.importFailed') + ': ' + e); }
                    }} style={{ fontSize: 10, height: 26, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <Upload style={{ width: 11, height: 11 }} /> {t('common.import')}
                    </button>
                    <button className="btn btn-secondary" onClick={handleClearCache} disabled={clearing || !cacheStats || cacheStats.total === 0}
                      style={{ fontSize: 10, height: 26, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3, color: '#f87171' }}>
                      <Trash2 style={{ width: 11, height: 11 }} />
                      {clearing ? t('settings.clearing') : t('settings.clearCache')}
                    </button>
                  </div>
                </div>
                {/* 各语言翻译统计 */}
                {cacheStats && (
                  <div style={{ display: 'flex', gap: 8 }}>
                    {[{ label: t('settings.langZhCN'), count: cacheStats.zh_cn, color: '#f87171' },
                      { label: t('settings.langJa'), count: cacheStats.ja, color: '#60a5fa' },
                      { label: t('settings.langKo'), count: cacheStats.ko, color: '#34d399' }].map(l => (
                      <div key={l.label} style={{ flex: 1, padding: '6px 10px', borderRadius: 6, background: `${l.color}08`, border: `1px solid ${l.color}20` }}>
                        <div style={{ fontSize: 9, color: l.color, fontWeight: 600, marginBottom: 2 }}>{l.label}</div>
                        <div style={{ fontSize: 13, fontWeight: 800, color: l.count > 0 ? 'var(--color-text-primary)' : 'var(--color-text-tertiary)' }}>{l.count.toLocaleString()}</div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* 标签数据库 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Database style={{ width: 16, height: 16, color: '#a78bfa' }} />
                <span className="tool-panel-title">{t('settings.tagDatabase')}</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', lineHeight: 1.6 }}>
                {t('settings.tagDatabaseDesc')}{t('settings.tagDbSource')}: <a href="#" onClick={(e) => { e.preventDefault(); window.open('https://github.com/DraconicDragon/dbr-e621-lists-archive', '_blank'); }} style={{ color: 'var(--color-accent-primary)', textDecoration: 'none', cursor: 'pointer' }}>DraconicDragon/dbr-e621-lists-archive</a>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.tagData')}</div>
                  <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    {tagDbStats ? (tagDbStats.has_data
                      ? `${t('settings.tagCount', { count: tagDbStats.total_tags.toLocaleString() })} · ${t('settings.translatedCount', { count: tagDbStats.translated_tags.toLocaleString() })} · ${formatSize(tagDbStats.db_size_bytes)}`
                      : t('settings.notDownloaded')) : t('common.loading')}
                  </div>
                  {tagDbStats?.has_data && tagDbStats.source_file && (
                    <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 3, display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span>{t('settings.versionLabel')}: {(() => {
                        const m = tagDbStats.source_file.match(/danbooru_(\d{4}-\d{2}-\d{2})/);
                        return m ? m[1] : tagDbStats.source_file;
                      })()}</span>
                      {tagDbStats.import_date && (
                        <span>· {t('settings.importedAt')} {new Date(parseInt(tagDbStats.import_date) * 1000).toLocaleDateString()}</span>
                      )}
                      {tagDbLatest && tagDbLatest !== tagDbStats.source_file && (
                        <span style={{ color: '#fbbf24', fontWeight: 600 }}>· {t('settings.newVersionAvailable')}: {(() => {
                          const m = tagDbLatest.match(/danbooru_(\d{4}-\d{2}-\d{2})/);
                          return m ? m[1] : tagDbLatest;
                        })()}</span>
                      )}
                      {tagDbLatest && tagDbLatest === tagDbStats.source_file && (
                        <span style={{ color: '#4ade80' }}>· {t('settings.alreadyLatest')}</span>
                      )}
                    </div>
                  )}
                </div>
                <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                  {tagDbStats?.has_data && (
                    <button className="btn btn-ghost btn-sm" title={t('settings.checkUpdate')} disabled={tagDbDownloading || tagDbTranslating || tagDbChecking}
                      onClick={async () => {
                        setTagDbChecking(true);
                        try { setTagDbLatest(await invoke<string>('check_tag_db_update')); } catch (e: any) { setTagDbProgress(`${t('settings.checkFailed')}: ${e?.message || e}`); }
                        finally { setTagDbChecking(false); }
                      }}
                      style={{ width: 28, height: 28, padding: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                      <RefreshIcon style={{ width: 12, height: 12, animation: tagDbChecking ? 'spin 1s linear infinite' : undefined, transition: 'transform 0.2s' }} />
                    </button>
                  )}
                  <button className="btn btn-primary" onClick={handleDownloadTagDb}
                    disabled={tagDbDownloading || tagDbTranslating || (!!tagDbLatest && tagDbLatest === tagDbStats?.source_file)}
                    style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4 }}>
                    {tagDbDownloading ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <Download style={{ width: 12, height: 12 }} />}
                    {tagDbDownloading ? t('settings.downloading') : (tagDbStats?.has_data ? t('common.update') : t('common.download'))}
                  </button>
                  {tagDbStats?.has_data && (
                    <button className="btn btn-secondary"
                      onClick={tagDbTranslating ? async () => { await invoke('cancel_tag_db_download'); } : handleTranslateTagDb}
                      disabled={tagDbDownloading}
                      onMouseEnter={() => setTranslateHover(true)}
                      onMouseLeave={() => setTranslateHover(false)}
                      style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4,
                        ...(tagDbTranslating && translateHover ? { color: '#f87171', borderColor: 'rgba(248,113,113,0.3)' } : {})
                      }}>
                      {tagDbTranslating
                        ? (translateHover
                          ? <><X style={{ width: 12, height: 12 }} /> {t('settings.cancelTranslate')}</>
                          : <><Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> {t('settings.translating')}</>)
                        : <><Languages style={{ width: 12, height: 12 }} /> {t('settings.translate')}</>}
                    </button>
                  )}
                  {tagDbStats?.has_data && (
                    <button className="btn btn-secondary" onClick={() => setTagDbClearConfirm(true)} disabled={tagDbDownloading || tagDbTranslating}
                      style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4, color: '#f87171' }}>
                      <Trash2 style={{ width: 12, height: 12 }} /> {t('settings.clearData')}
                    </button>
                  )}
                </div>
              </div>
              {tagDbProgress && (
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', padding: '4px 8px', borderRadius: 4, background: 'var(--color-bg-secondary)' }}>
                  {tagDbProgress}
                </div>
              )}
            </div>
          </div>

          {/* 高级设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <AlertTriangle style={{ width: 16, height: 16, color: '#fbbf24' }} />
                <span className="tool-panel-title">{t('settings.advanced')}</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* Python 环境信息 */}
              <div style={{ padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div>
                    <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>{t('settings.pythonEnv')}</div>
                    <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                      {pythonInfo === null ? t('common.loading') : pythonInfo.available
                        ? <><span style={{ color: '#4ade80' }}>✓</span> {pythonInfo.version}</>
                        : <span style={{ color: '#f87171' }}>{t('settings.pythonNotInstalled')}</span>
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
                  <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>{t('settings.resetPythonEnv')}</div>
                  <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>{t('settings.resetConfirmMsg')}</div>
                </div>
                <button className="btn" onClick={() => setResetPythonConfirmOpen(true)} disabled={resettingPython}
                  style={{ background: 'rgba(248,113,113,0.1)', color: '#f87171', border: '1px solid rgba(248,113,113,0.3)', fontSize: 12, padding: '6px 14px', gap: 6 }}>
                  {resettingPython ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <RotateCcw style={{ width: 14, height: 14 }} />}
                  {t('settings.cacheReset')}
                </button>
              </div>
            </div>
          </div>

          {/* About */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Info style={{ width: 16, height: 16, color: 'var(--color-text-tertiary)' }} />
                <span className="tool-panel-title">{t('settings.about')}</span>
              </div>
            </div>
            <div style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)', lineHeight: 1.8 }}>
              <p><strong>PurinBox</strong> · v{appVersion}</p>
              <p>Tauri 2 + React + TypeScript</p>
              <p style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <a href="https://github.com/YPuddin-Neko/PurinBox" target="_blank" rel="noreferrer"
                  style={{ color: '#60a5fa', textDecoration: 'none', display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  GitHub <ExternalLink style={{ width: 12, height: 12 }} />
                </a>
              </p>
              {/* Update check */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 8, padding: '10px 14px', borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.checkForUpdate')}</div>
                  <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    {updateChecking ? t('settings.checkingForUpdate')
                      : updateError ? `${t('settings.updateCheckFailed')}: ${updateError}`
                      : updateResult
                        ? updateResult.has_update
                          ? <span style={{ color: '#ef4444' }}>{t('settings.hasUpdate', { version: updateResult.latest_version })}</span>
                          : <span style={{ color: '#4ade80' }}>{t('settings.noUpdate')}</span>
                        : t('settings.checkForUpdate')}
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 6 }}>
                  {updateResult?.has_update && updateResult.release_url && (
                    <a href={updateResult.release_url} target="_blank" rel="noreferrer"
                      className="btn btn-primary" style={{ fontSize: 11, height: 28, padding: '0 12px', textDecoration: 'none', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <ExternalLink style={{ width: 11, height: 11 }} /> {t('settings.goToDownload')}
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
                    {updateChecking ? t('settings.checkingForUpdate') : t('settings.checkForUpdate')}
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
        title={t('settings.clearCache')}
        message={t('settings.clearConfirmMsg')}
        confirmText={t('settings.clearCache')}
        variant="warning"
      />
      <ConfirmModal
        open={tagDbClearConfirm}
        onClose={() => setTagDbClearConfirm(false)}
        onConfirm={handleClearTagDb}
        title={t('settings.clearConfirmTitle')}
        message={t('settings.clearConfirmMsg')}
        confirmText={t('settings.clearData')}
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
            setAlertMsg(t('settings.resetSuccess'));
            loadPythonInfo();
          } catch (e: any) {
            setAlertMsg(`${t('settings.resetFailed')}: ${e}`);
          } finally {
            setResettingPython(false);
          }
        }}
        title={t('settings.resetConfirmTitle')}
        message={t('settings.resetConfirmMsg')}
        confirmText={t('settings.cacheReset')}
        variant="error"
      />
      <AlertModal
        open={!!alertMsg}
        onClose={() => setAlertMsg('')}
        title={t('common.notice')}
        message={alertMsg}
      />
    </>
  );
}
