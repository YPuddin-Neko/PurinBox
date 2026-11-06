import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { hasTauriRuntime } from '../utils/tauriRuntime';

type LocalPaths = { python_path: string; detector_path: string; embedder_path: string };

export default function GbcEnvironment() {
  const { t } = useTranslation();
  const [paths, setPaths] = useState<LocalPaths>();
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState('');
  const refresh = () => invoke<LocalPaths>('get_gbc_local_paths').then(setPaths);
  useEffect(() => { if (hasTauriRuntime()) void refresh().catch(e => setError(String(e))); }, []);
  const install = async () => {
    setInstalling(true); setError('');
    try { await invoke('setup_gbc_environment'); await refresh(); }
    catch (e) { setError(String(e)); }
    finally { setInstalling(false); }
  };
  return <div>
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
      <div>
        <div style={{ fontSize: 'var(--font-size-sm)', fontWeight: 600 }}>{t('gbc.environment')}</div>
        <div style={{ fontSize: 11, color: 'var(--color-text-tertiary)', marginTop: 2 }}>
          {paths ? t(!paths.python_path ? 'gbc.notInstalled' : !paths.detector_path || !paths.embedder_path ? 'gbc.modelsMissing' : 'gbc.ready') : t('common.loading')}
        </div>
      </div>
      {(!paths?.python_path || installing) && <button className="btn btn-secondary btn-sm" disabled={installing || !hasTauriRuntime()} onClick={() => void install()}>{t(installing ? 'gbc.installing' : 'gbc.install')}</button>}
      {installing && <button className="btn btn-secondary btn-sm" onClick={() => void invoke('cancel_gbc_setup').catch(e => setError(String(e)))}>{t('gbc.cancelInstall')}</button>}
    </div>
    {error && <div role="status" style={{ fontSize: 11, color: 'var(--color-error)', marginTop: 6 }}>{error}</div>}
  </div>;
}
