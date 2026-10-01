import { Settings, Info, Check, Activity, Languages, Trash2, Eye, EyeOff, ExternalLink, Loader2, Zap, FolderOpen, RotateCcw, Globe, Save, Terminal, RefreshCw as RefreshIcon, Database, Download, Upload, X, Play, KeyRound, FlaskConical } from 'lucide-react';
import { useAppSettings } from '../components/ThemeProvider';
import { useState, useEffect, useRef, type CSSProperties } from 'react';
import { useTranslation } from 'react-i18next';
import { hasTauriRuntime, listen } from '../utils/tauriRuntime';
import { ConfirmModal, AlertModal } from '../components/Modal';
import CustomSelect from '../components/CustomSelect';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { packageAppVersion, type UpdateCheckResult } from '../utils/appVersion';

import SystemMonitor from '../components/SystemMonitor';
import PageHeader from '../components/ui/PageHeader';
import Switch from '../components/ui/Switch';
import NumberInput from '../components/ui/NumberInput';
import SegmentedTabs from '../components/ui/SegmentedTabs';

// 密钥输入框组件（提升到模块顶层，避免每次重渲染重建组件类型导致输入丢焦点）
const SecretInput = ({ value, onChange, placeholder, show, onToggle, fontSize = 12 }: { value: string; onChange: (v: string) => void; placeholder?: string; show: boolean; onToggle: () => void; fontSize?: number }) => (
  <div style={{ position: 'relative' }}>
    <input className="form-input" type={show ? 'text' : 'password'} value={value} onChange={e => onChange(e.target.value)}
      placeholder={placeholder} style={{ fontSize, height: 32, paddingRight: 32 }} />
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
    fontSize: 10, color: '#60a5fa', display: 'inline-flex', alignItems: 'center', gap: 3,
  }}>
    {text} <ExternalLink style={{ width: 9, height: 9 }} />
  </a>
);

interface CacheStats { total: number; db_size_bytes: number; zh_cn: number; ja: number; ko: number }
interface PythonInfo { available: boolean; version: string; path: string }
interface TagDbStats { total_tags: number; translated_tags: number; db_size_bytes: number; has_data: boolean; source_file: string; import_date: string }
interface SaveMsg { text: string; ok: boolean }

/** 设置项卡片的公共底色与边框 */
const cardStyle: CSSProperties = { borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)' };

const formatTagDbVersion = (sourceFile: string) => sourceFile.match(/danbooru_(\d{4}-\d{2}-\d{2})/)?.[1] ?? sourceFile;

/** 保存按钮的状态：保存中 + 结果提示，提示 3 秒后清除 */
function useSaveFeedback() {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<SaveMsg | null>(null);
  const clearTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (clearTimer.current) clearTimeout(clearTimer.current);
  }, []);
  const run = async (action: () => Promise<unknown>, successKey: string) => {
    if (clearTimer.current) clearTimeout(clearTimer.current);
    setSaving(true); setMsg(null);
    try {
      await action();
      setMsg({ text: t(successKey), ok: true });
    } catch (e: any) {
      setMsg({ text: e?.message || String(e), ok: false });
    } finally {
      setSaving(false);
      clearTimer.current = setTimeout(() => setMsg(null), 3000);
    }
  };
  return { saving, msg, run };
}

const SaveButton = ({ saving, msg, disabled, onClick }: { saving: boolean; msg: SaveMsg | null; disabled: boolean; onClick: () => void }) => {
  const { t } = useTranslation();
  return (
    <div style={{ display: 'flex', gap: 6 }}>
      {msg && <span style={{ fontSize: 10, color: msg.ok ? '#4ade80' : '#f87171' }}>{msg.text}</span>}
      <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={onClick} disabled={saving || disabled}>
        <Save style={{ width: 12, height: 12 }} /> {saving ? t('settings.proxySaving') : t('common.save')}
      </button>
    </div>
  );
};

interface ProviderField {
  /** localStorage 键，同时作为 React key */
  storageKey: string;
  label: string;
  placeholder?: string;
  value: string;
  setValue: (v: string) => void;
  secret?: { show: boolean; toggle: () => void };
}

export default function SettingsPage() {
  const { t } = useTranslation();
  const {
    monitorInterval,
    setMonitorInterval,
    workflowEnabled,
    setWorkflowEnabled,
    hybridTaggerEnabled,
    setHybridTaggerEnabled,
  } = useAppSettings();
  const isDesktopRuntime = hasTauriRuntime();
  const desktopOnlyText = t('settings.desktopOnly');

  const intervalOptions = [
    { value: 1000, label: t('settings.monitorSec', { n: 1 }) },
    { value: 2000, label: t('settings.monitorSec', { n: 2 }) },
    { value: 3000, label: t('settings.monitorSec', { n: 3 }) },
    { value: 5000, label: t('settings.monitorSec', { n: 5 }) },
    { value: 10000, label: t('settings.monitorSec', { n: 10 }) },
    { value: 0, label: t('settings.monitorOff') },
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

  const [cacheStats, setCacheStats] = useState<CacheStats | null>(null);
  const [clearing, setClearing] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const [cachePath, setCachePath] = useState<string>('');
  const appVersion = packageAppVersion;
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [resetPythonConfirmOpen, setResetPythonConfirmOpen] = useState(false);
  const [resettingPython, setResettingPython] = useState(false);
  const [alertMsg, setAlertMsg] = useState('');
  const [pythonInfo, setPythonInfo] = useState<PythonInfo | null>(null);

  // 更新检查
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);
  const [updateError, setUpdateError] = useState('');

  // 标签数据库
  const [tagDbStats, setTagDbStats] = useState<TagDbStats | null>(null);
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
  const [proxyType, setProxyType] = useState<'http' | 'socks5'>('http');
  const [proxyHost, setProxyHost] = useState('127.0.0.1');
  const [proxyPort, setProxyPort] = useState(7890);
  const [proxyUser, setProxyUser] = useState('');
  const [proxyPass, setProxyPass] = useState('');
  const [showProxyPass, setShowProxyPass] = useState(false);
  const proxySave = useSaveFeedback();

  // Hugging Face
  const [huggingFaceToken, setHuggingFaceToken] = useState('');
  const [showHuggingFaceToken, setShowHuggingFaceToken] = useState(false);
  const huggingFaceSave = useSaveFeedback();

  const toggleTranslate = (val: boolean) => { setTranslateEnabled(val); localStorage.setItem('translate_enabled', String(val)); };
  const changeProvider = (val: string) => { setProvider(val); localStorage.setItem('translate_provider', val); };

  const saveLS = (key: string, val: string, setter: (v: string) => void) => { setter(val); localStorage.setItem(key, val); };

  const providerForms: Record<string, { name: string; applyUrl: string; fields: ProviderField[] }> = {
    baidu: {
      name: t('settings.providerBaidu'),
      applyUrl: 'https://fanyi-api.baidu.com',
      fields: [
        { storageKey: 'baidu_appid', label: t('settings.baiduAppId'), value: baiduAppid, setValue: setBaiduAppid },
        { storageKey: 'baidu_key', label: t('settings.baiduKey'), value: baiduKey, setValue: setBaiduKey,
          secret: { show: showBaiduKey, toggle: () => setShowBaiduKey(!showBaiduKey) } },
      ],
    },
    youdao: {
      name: t('settings.providerYoudao'),
      applyUrl: 'https://ai.youdao.com',
      fields: [
        { storageKey: 'youdao_app_key', label: t('settings.youdaoAppKey'), value: youdaoAppKey, setValue: setYoudaoAppKey },
        { storageKey: 'youdao_app_secret', label: t('settings.youdaoAppSecret'), value: youdaoAppSecret, setValue: setYoudaoAppSecret,
          secret: { show: showYoudaoKey, toggle: () => setShowYoudaoKey(!showYoudaoKey) } },
      ],
    },
    bing: {
      name: t('settings.providerBing'),
      applyUrl: 'https://portal.azure.com/#create/Microsoft.CognitiveServicesTextTranslation',
      fields: [
        { storageKey: 'bing_key', label: t('settings.bingKey'), placeholder: t('settings.bingKeyPlaceholder'), value: bingKey, setValue: setBingKey,
          secret: { show: showBingKey, toggle: () => setShowBingKey(!showBingKey) } },
        { storageKey: 'bing_region', label: t('settings.bingRegion'), placeholder: t('settings.bingRegionPlaceholder'), value: bingRegion, setValue: setBingRegion },
      ],
    },
  };
  const providerForm = providerForms[provider];

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
    try { setCacheStats(await invoke<CacheStats>('get_translation_cache_stats')); } catch (e) { console.error(e); }
  };

  const doClearCache = async () => {
    setClearing(true);
    try { await invoke('clear_translation_cache'); await loadCacheStats(); } catch (e) { console.error(e); } finally { setClearing(false); }
  };

  const loadPythonInfo = async () => {
    try { setPythonInfo(await invoke<PythonInfo>('get_python_env_info')); } catch { setPythonInfo(null); }
  };

  const runPythonAction = async (command: 'deploy_python_env' | 'reset_python_env', successKey: string, failKey: string) => {
    setResettingPython(true);
    try {
      await invoke(command);
      setAlertMsg(t(successKey));
      loadPythonInfo();
    } catch (e) {
      setAlertMsg(`${t(failKey)}: ${e}`);
    } finally {
      setResettingPython(false);
    }
  };

  // 目标语言从 localStorage 读：挂载时注册的 tag-db-progress 监听也会调用这里，读 state 会拿到旧值
  const loadTagDbStats = async () => {
    try {
      const lang = localStorage.getItem('translate_target_lang') || 'zh-CN';
      setTagDbStats(await invoke<TagDbStats>('get_tag_db_stats', { targetLang: lang }));
    } catch (e) { console.error(e); }
  };

  const handleDownloadTagDb = async () => {
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

    if (!hasTauriRuntime()) return;

    loadCacheStats(); loadCachePath(); loadTagDbStats();
    loadProxySettings(); loadHuggingFaceSettings(); loadPythonInfo();
    // 恢复后端忙碌状态
    invoke<[boolean, boolean]>('is_tag_db_busy').then(([downloading, translating]) => {
      setTagDbDownloading(downloading);
      setTagDbTranslating(translating);
    }).catch(() => {});
    let active = true;
    const unlisten = listen<{ status: string; message: string; current: number; total: number }>('tag-db-progress', (e) => {
      if (!active) return;
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
    return () => { active = false; unlisten.then(fn => fn()); };
  }, []);

  const loadProxySettings = async () => {
    try {
      const [enabled, llm, ptype, host, port, user, pass] = await invoke<[boolean, boolean, string, string, number, string, string]>('load_proxy_config');
      setProxyEnabled(enabled); setLlmProxy(llm); setProxyType(ptype === 'socks5' ? 'socks5' : 'http'); setProxyHost(host); setProxyPort(port); setProxyUser(user); setProxyPass(pass);
    } catch {}
  };

  const handleSaveProxy = () => {
    const port = Number.isFinite(proxyPort) && proxyPort > 0 ? Math.floor(proxyPort) : 0;
    if (port !== proxyPort) setProxyPort(port);
    proxySave.run(
      () => invoke('save_proxy_config', { enabled: proxyEnabled, llmProxy, proxyType, host: proxyHost, port, username: proxyUser, password: proxyPass }),
      'settings.proxySaved',
    );
  };

  const loadHuggingFaceSettings = async () => {
    try {
      setHuggingFaceToken(await invoke<string>('load_huggingface_config'));
    } catch {}
  };

  const handleSaveHuggingFace = () => {
    huggingFaceSave.run(() => invoke('save_huggingface_config', { token: huggingFaceToken }), 'settings.huggingFaceSaved');
  };

  const loadCachePath = async () => {
    try { setCachePath(await invoke<string>('get_cache_path')); } catch (e) { console.error(e); }
  };

  const handleChangeCachePath = async () => {
    const selected = await open({ directory: true, title: t('settings.selectCacheDir') });
    if (selected && typeof selected === 'string') {
      try {
        setCachePath(await invoke<string>('set_cache_path', { path: selected }));
        await loadCacheStats();
      } catch (e: any) { setAlertMsg(`${t('settings.setCachePathFailed')}: ${e?.message || e}`); }
    }
  };

  const handleResetCachePath = async () => {
    try {
      // 传空串即重置为默认目录，返回值就是重置后的目录
      setCachePath(await invoke<string>('set_cache_path', { path: '' }));
      await loadCacheStats();
    } catch (e: any) { setAlertMsg(`${t('settings.resetFailed')}: ${e?.message || e}`); }
  };

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  };

  return (
    <>
    <div className="page" style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
      <div style={{ width: '100%', maxWidth: 640 }}>
        <PageHeader icon={Settings} color="var(--color-accent-primary)" title={t('settings.title')} subtitle={t('settings.aboutDesc')} />

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {monitorInterval > 0 && isDesktopRuntime && <SystemMonitor />}



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

          {/* Experimental Features */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <FlaskConical style={{ width: 16, height: 16, color: '#fbbf24' }} />
                <span className="tool-panel-title">{t('settings.experimental')}</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {[
                {
                  enabled: workflowEnabled,
                  setEnabled: setWorkflowEnabled,
                  title: t('settings.workflowToggle'),
                },
                {
                  enabled: hybridTaggerEnabled,
                  setEnabled: setHybridTaggerEnabled,
                  title: t('settings.hybridTaggerToggle'),
                },
              ].map((feature) => (
                <div key={feature.title} style={{
                  ...cardStyle,
                  display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16,
                  padding: '10px 12px',
                }}>
                  <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--color-text-primary)' }}>
                    {feature.title}
                    <span className="beta-badge">
                      Beta
                    </span>
                  </span>
                  <Switch checked={feature.enabled} onChange={feature.setEnabled} aria-label={feature.title} />
                </div>
              ))}
            </div>
          </div>

          {/* Proxy */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Globe style={{ width: 16, height: 16, color: '#f59e0b' }} />
                <span className="tool-panel-title">{t('settings.proxy')}</span>
              </div>
              <SaveButton saving={proxySave.saving} msg={proxySave.msg} disabled={!isDesktopRuntime} onClick={handleSaveProxy} />
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* 启用开关 + LLM 开关 + 代理类型 */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 10 }}>
                <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 12px' }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.proxyEnabled')}</div>
                  </div>
                  <Switch checked={proxyEnabled} onChange={setProxyEnabled} aria-label={t('settings.proxyEnabled')} />
                </div>
                <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 12px' }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.proxyLlm')}</div>
                  </div>
                  <Switch checked={llmProxy} onChange={setLlmProxy} aria-label={t('settings.proxyLlm')} />
                </div>
                <div style={{ ...cardStyle, padding: '10px 12px' }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-secondary)', marginBottom: 5 }}>{t('settings.proxyType')}</div>
                  <SegmentedTabs className="ui-seg-compact" value={proxyType} onChange={setProxyType} tabs={[
                    { id: 'http', label: 'HTTP' }, { id: 'socks5', label: 'SOCKS5' },
                  ]} />
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
                  <NumberInput min={0} max={65535} integer placeholder="7890" value={proxyPort} onChange={setProxyPort} style={{ height: 32 }} />
                </div>
              </div>

              {/* 认证（可选） */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('settings.proxyUserOptional')}</label>
                  <input className="form-input" placeholder="" value={proxyUser} onChange={e => setProxyUser(e.target.value)} style={{ height: 32 }} />
                </div>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('settings.proxyPassOptional')}</label>
                  <SecretInput fontSize={13} value={proxyPass} onChange={setProxyPass} placeholder=""
                    show={showProxyPass} onToggle={() => setShowProxyPass(!showProxyPass)} />
                </div>
              </div>
            </div>
          </div>

          {/* Tokens */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <KeyRound style={{ width: 16, height: 16, color: '#f97316' }} />
                <span className="tool-panel-title">{t('settings.tokenSettings')}</span>
              </div>
              <SaveButton saving={huggingFaceSave.saving} msg={huggingFaceSave.msg} disabled={!isDesktopRuntime} onClick={handleSaveHuggingFace} />
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div style={{ ...cardStyle, padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div>
                  <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.huggingFace')}</div>
                </div>
                <SecretInput
                  value={huggingFaceToken}
                  onChange={setHuggingFaceToken}
                  placeholder={isDesktopRuntime ? t('settings.huggingFaceTokenPlaceholder') : desktopOnlyText}
                  show={showHuggingFaceToken}
                  onToggle={() => setShowHuggingFaceToken(!showHuggingFaceToken)}
                />
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
                  <LinkButton href="https://huggingface.co/settings/tokens" text={t('settings.huggingFaceTokenLink')} />
                </div>
              </div>
              <p style={{ fontSize: 10, color: 'var(--color-text-tertiary)', lineHeight: 1.6, margin: 0 }}>
                {t('settings.huggingFaceDesc')}
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
                <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, padding: '12px 14px' }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.enableTranslation')}</div>
                  </div>
                  <Switch checked={translateEnabled} onChange={toggleTranslate} aria-label={t('settings.enableTranslation')} />
                </div>
                <div style={{ ...cardStyle, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 6, padding: '12px 14px' }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <label style={{ fontSize: 12, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.translationProvider')}</label>
                    <button onClick={handleTestTranslation} disabled={testing || !isDesktopRuntime}
                      style={{ fontSize: 10, color: testResult ? (testResult.ok ? '#4ade80' : '#f87171') : '#60a5fa', background: 'none', border: 'none', cursor: testing ? 'wait' : 'pointer', display: 'flex', alignItems: 'center', gap: 3, padding: 0 }}>
                      {testing ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} /> : <Zap style={{ width: 10, height: 10 }} />}
                      {testing ? t('settings.testing') : testResult ? (testResult.ok ? testResult.msg : t('common.failed')) : t('settings.testTranslation')}
                    </button>
                  </div>
                  <CustomSelect value={provider} onChange={v => { changeProvider(v); setTestResult(null); }} options={providerOptions} compact />
                </div>
              </div>

              {/* 目标语言 */}
              <div style={{ ...cardStyle, padding: '12px 14px' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--color-text-primary)' }}>{t('settings.targetLanguage')}</div>
                  </div>
                  <CustomSelect value={targetLang}
                    onChange={v => { setTargetLang(v); localStorage.setItem('translate_target_lang', v); if (isDesktopRuntime) loadTagDbStats(); }}
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
              {providerForm && (
                <div key={provider} style={{ ...cardStyle, padding: '14px', display: 'flex', flexDirection: 'column', gap: 10 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--color-text-primary)', display: 'flex', alignItems: 'center', gap: 8 }}>
                    {providerForm.name} API
                    <LinkButton href={providerForm.applyUrl} text={t('settings.applyLink')} />
                  </div>
                  {providerForm.fields.map(f => (
                    <div key={f.storageKey}>
                      <label style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 3, display: 'block' }}>{f.label}</label>
                      {f.secret ? (
                        <SecretInput value={f.value} onChange={v => saveLS(f.storageKey, v, f.setValue)}
                          placeholder={f.placeholder} show={f.secret.show} onToggle={f.secret.toggle} />
                      ) : (
                        <input className="form-input" value={f.value} onChange={e => saveLS(f.storageKey, e.target.value, f.setValue)}
                          placeholder={f.placeholder} style={{ fontSize: 12, height: 32 }} />
                      )}
                    </div>
                  ))}
                </div>
              )}

              {/* 缓存路径 */}
              <div style={{ ...cardStyle, padding: '10px 14px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.cachePath')}</div>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <button className="btn btn-secondary" onClick={handleChangeCachePath} disabled={!isDesktopRuntime}
                      style={{ fontSize: 10, height: 24, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <FolderOpen style={{ width: 10, height: 10 }} />{t('settings.cacheModify')}
                    </button>
                    <button className="btn btn-secondary" onClick={handleResetCachePath} disabled={!isDesktopRuntime} title={t('settings.cacheReset')}
                      style={{ fontSize: 10, height: 24, padding: '0 6px', display: 'flex', alignItems: 'center' }}>
                      <RotateCcw style={{ width: 10, height: 10 }} />
                    </button>
                  </div>
                </div>
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', wordBreak: 'break-all', lineHeight: 1.5, background: 'var(--color-bg-secondary)', padding: '6px 8px', borderRadius: 4 }}>
                  {cachePath || (isDesktopRuntime ? t('common.loading') : desktopOnlyText)}
                </div>
              </div>

              <div style={{ ...cardStyle, padding: '10px 14px' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                  <div>
                    <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.translationCache')}</div>
                    <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                      {cacheStats ? `${t('settings.cacheRecords', { count: cacheStats.total })} · ${formatSize(cacheStats.db_size_bytes)}` : (isDesktopRuntime ? t('common.loading') : desktopOnlyText)}
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    <button className="btn btn-ghost btn-sm" title={t('settings.exportCsv')} disabled={!isDesktopRuntime} onClick={async () => {
                      try {
                        const path = await save({ title: t('settings.exportCsv'), defaultPath: 'translations.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
                        if (path) {
                          const count = await invoke<number>('export_translation_csv', { path });
                          setAlertMsg(t('settings.exportSuccess', { count }));
                        }
                      } catch (e: any) { setAlertMsg(t('settings.exportFailed') + ': ' + e); }
                    }} style={{ fontSize: 10, height: 26, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <Download style={{ width: 11, height: 11 }} /> {t('common.export')}
                    </button>
                    <button className="btn btn-ghost btn-sm" title={t('settings.importCsv')} disabled={!isDesktopRuntime} onClick={async () => {
                      try {
                        const path = await open({ title: t('settings.importCsv'), filters: [{ name: 'CSV', extensions: ['csv'] }] });
                        if (path) {
                          const [imported, skipped, errors] = await invoke<[number, number, string]>('import_translation_csv', { path });
                          let msg = t('settings.importSuccess', { imported });
                          if (skipped > 0) msg += t('settings.importSkipped', { skipped });
                          if (errors) msg += `\n\n${errors}`;
                          setAlertMsg(msg);
                          loadCacheStats();
                        }
                      } catch (e: any) { setAlertMsg(t('settings.importFailed') + ': ' + e); }
                    }} style={{ fontSize: 10, height: 26, padding: '0 8px', display: 'flex', alignItems: 'center', gap: 3 }}>
                      <Upload style={{ width: 11, height: 11 }} /> {t('common.import')}
                    </button>
                    <button className="btn btn-secondary" onClick={() => setClearConfirmOpen(true)} disabled={clearing || !cacheStats || cacheStats.total === 0 || !isDesktopRuntime}
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
                {t('settings.tagDbSource')}: <a href="https://github.com/DraconicDragon/dbr-e621-lists-archive" target="_blank" rel="noreferrer" style={{ color: 'var(--color-accent-primary)' }}>DraconicDragon/dbr-e621-lists-archive</a>
              </div>
              <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '10px 14px' }}>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.tagData')}</div>
                  <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    {tagDbStats ? (tagDbStats.has_data
                      ? `${t('settings.tagCount', { count: tagDbStats.total_tags.toLocaleString() })} · ${t('settings.translatedCount', { count: tagDbStats.translated_tags.toLocaleString() })} · ${formatSize(tagDbStats.db_size_bytes)}`
                      : t('settings.notDownloaded')) : (isDesktopRuntime ? t('common.loading') : desktopOnlyText)}
                  </div>
                  {tagDbStats?.has_data && tagDbStats.source_file && (
                    <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 3, display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span>{t('settings.versionLabel')}: {formatTagDbVersion(tagDbStats.source_file)}</span>
                      {tagDbStats.import_date && (
                        <span>· {t('settings.importedAt')} {new Date(parseInt(tagDbStats.import_date) * 1000).toLocaleDateString()}</span>
                      )}
                      {tagDbLatest && tagDbLatest !== tagDbStats.source_file && (
                        <span style={{ color: '#fbbf24', fontWeight: 600 }}>· {t('settings.newVersionAvailable')}: {formatTagDbVersion(tagDbLatest)}</span>
                      )}
                      {tagDbLatest && tagDbLatest === tagDbStats.source_file && (
                        <span style={{ color: '#4ade80' }}>· {t('settings.alreadyLatest')}</span>
                      )}
                    </div>
                  )}
                </div>
                <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                  {tagDbStats?.has_data && (
                    <button className="btn btn-ghost btn-sm" title={t('settings.checkUpdate')} disabled={tagDbDownloading || tagDbTranslating || tagDbChecking || !isDesktopRuntime}
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
                    disabled={tagDbDownloading || tagDbTranslating || (!!tagDbLatest && tagDbLatest === tagDbStats?.source_file) || !isDesktopRuntime}
                    style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4 }}>
                    {tagDbDownloading ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} /> : <Download style={{ width: 12, height: 12 }} />}
                    {tagDbDownloading ? t('settings.downloading') : (tagDbStats?.has_data ? t('common.update') : t('common.download'))}
                  </button>
                  {tagDbStats?.has_data && (
                    <button className="btn btn-secondary"
                      onClick={tagDbTranslating ? async () => { await invoke('cancel_tag_db_translation'); } : handleTranslateTagDb}
                      disabled={tagDbDownloading || !isDesktopRuntime}
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
                    <button className="btn btn-secondary" onClick={() => setTagDbClearConfirm(true)} disabled={tagDbDownloading || tagDbTranslating || !isDesktopRuntime}
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

          {/* 环境设置 */}
          <div className="tool-panel">
            <div className="tool-panel-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
                <Terminal style={{ width: 16, height: 16, color: '#38bdf8' }} />
                <span className="tool-panel-title">{t('settings.envSettings')}</span>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              {/* Python 环境信息 */}
              <div style={{ ...cardStyle, padding: '10px 14px' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div>
                    <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>{t('settings.pythonEnv')}</div>
                    <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                      {!isDesktopRuntime ? desktopOnlyText : pythonInfo === null ? t('common.loading') : pythonInfo.available
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

              {/* Python 操作按钮 */}
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <div>
                  <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>{pythonInfo?.available ? t('settings.resetPythonEnv') : t('settings.deployPythonEnv')}</div>
                  {!pythonInfo?.available && <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>{t('settings.deployPythonDesc')}</div>}
                </div>
                {pythonInfo?.available ? (
                  <button className="btn btn-danger" onClick={() => setResetPythonConfirmOpen(true)} disabled={resettingPython}
                    style={{ fontSize: 12, padding: '6px 14px', gap: 6 }}>
                    {resettingPython ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <RotateCcw style={{ width: 14, height: 14 }} />}
                    {t('settings.cacheReset')}
                  </button>
                ) : (
                  <button className="btn" onClick={() => runPythonAction('deploy_python_env', 'settings.deploySuccess', 'settings.deployFailed')}
                    disabled={resettingPython || !isDesktopRuntime}
                    style={{ background: 'rgba(74,222,128,0.1)', color: '#4ade80', border: '1px solid rgba(74,222,128,0.3)', fontSize: 12, padding: '6px 14px', gap: 6 }}>
                    {resettingPython ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Play style={{ width: 14, height: 14 }} />}
                    {isDesktopRuntime ? t('settings.deployEnv') : desktopOnlyText}
                  </button>
                )}
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
              <p style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <a href="https://github.com/YPuddin-Neko/PurinBox" target="_blank" rel="noreferrer"
                  style={{ color: '#60a5fa', display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  GitHub <ExternalLink style={{ width: 12, height: 12 }} />
                </a>
              </p>
              {/* Update check */}
              <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', gap: 10, marginTop: 8, padding: '10px 14px' }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-primary)' }}>{t('settings.checkForUpdate')}</div>
                  <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
                    {!isDesktopRuntime ? desktopOnlyText
                      : updateChecking ? t('settings.checkingForUpdate')
                      : updateError ? `${t('settings.updateCheckFailed')}: ${updateError}`
                      : updateResult
                        ? updateResult.has_update
                          ? <span style={{ color: '#ef4444' }}>{t('settings.hasUpdate', { version: updateResult.latest_version })}</span>
                          : <span style={{ color: '#4ade80' }}>{t('settings.noUpdate')}</span>
                        : null}
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 6 }}>
                  {updateResult?.has_update && updateResult.release_url && (
                    <a href={updateResult.release_url} target="_blank" rel="noreferrer"
                      className="btn btn-primary" style={{ fontSize: 11, height: 28, padding: '0 12px', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <ExternalLink style={{ width: 11, height: 11 }} /> {t('settings.goToDownload')}
                    </a>
                  )}
                  <button className="btn btn-secondary" disabled={updateChecking || !isDesktopRuntime}
                    onClick={async () => {
                      setUpdateChecking(true); setUpdateError(''); setUpdateResult(null);
                      try {
                        const r = await invoke<UpdateCheckResult>('check_for_updates');
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
        title={t('settings.clearCacheTitle')}
        message={t('settings.clearCacheConfirmMsg')}
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
        onConfirm={() => runPythonAction('reset_python_env', 'settings.resetSuccess', 'settings.resetFailed')}
        title={t('settings.resetConfirmTitle')}
        message={t('settings.resetConfirmDialog')}
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
