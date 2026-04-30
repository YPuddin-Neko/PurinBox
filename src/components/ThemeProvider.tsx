import { createContext, useContext, useState, useEffect, ReactNode } from 'react';

type ThemeMode = 'dark' | 'light' | 'system';

interface AppSettingsContextType {
  mode: ThemeMode;
  setMode: (m: ThemeMode) => void;
  resolved: 'dark' | 'light';
  monitorInterval: number;
  setMonitorInterval: (ms: number) => void;
}

const AppSettingsContext = createContext<AppSettingsContextType>({
  mode: 'dark', setMode: () => {}, resolved: 'dark',
  monitorInterval: 3000, setMonitorInterval: () => {},
});

export function useTheme() { return useContext(AppSettingsContext); }
export function useAppSettings() { return useContext(AppSettingsContext); }

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeRaw] = useState<ThemeMode>(() => {
    return (localStorage.getItem('theme') as ThemeMode) || 'dark';
  });

  const [monitorInterval, setMonitorIntervalRaw] = useState<number>(() => {
    const saved = localStorage.getItem('monitorInterval');
    return saved ? Number(saved) : 3000;
  });

  const [systemDark, setSystemDark] = useState(
    window.matchMedia('(prefers-color-scheme: dark)').matches
  );

  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  const setMode = (m: ThemeMode) => {
    setModeRaw(m);
    localStorage.setItem('theme', m);
  };

  const setMonitorInterval = (ms: number) => {
    setMonitorIntervalRaw(ms);
    localStorage.setItem('monitorInterval', String(ms));
  };

  const resolved = mode === 'system' ? (systemDark ? 'dark' : 'light') : mode;

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', resolved);
  }, [resolved]);

  return (
    <AppSettingsContext.Provider value={{ mode, setMode, resolved, monitorInterval, setMonitorInterval }}>
      {children}
    </AppSettingsContext.Provider>
  );
}
