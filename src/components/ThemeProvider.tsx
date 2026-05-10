import { createContext, useContext, useState, useEffect, useCallback, ReactNode } from 'react';

type ThemeMode = 'dark' | 'light' | 'system';

interface AppSettingsContextType {
  mode: ThemeMode;
  setMode: (m: ThemeMode) => void;
  resolved: 'dark' | 'light';
  monitorInterval: number;
  setMonitorInterval: (ms: number) => void;
  cycleThemeWithRipple: (x: number, y: number) => void;
}

const AppSettingsContext = createContext<AppSettingsContextType>({
  mode: 'dark', setMode: () => {}, resolved: 'dark',
  monitorInterval: 3000, setMonitorInterval: () => {},
  cycleThemeWithRipple: () => {},
});

export function useTheme() { return useContext(AppSettingsContext); }
export function useAppSettings() { return useContext(AppSettingsContext); }

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeRaw] = useState<ThemeMode>(() => {
    return (localStorage.getItem('theme') as ThemeMode) || 'system';
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
    const el = document.documentElement;
    el.setAttribute('data-theme', resolved);
  }, [resolved]);

  // 水滴波纹切换主题 (View Transitions API)
  const cycleThemeWithRipple = useCallback((x: number, y: number) => {
    const next: Record<string, ThemeMode> = { dark: 'light', light: 'system', system: 'dark' };
    const nextMode = next[mode];
    const nextResolved = nextMode === 'system' ? (systemDark ? 'dark' : 'light') : nextMode;

    // 如果实际解析的主题没变，只切换模式不做动画
    if (nextResolved === resolved) {
      setMode(nextMode);
      return;
    }

    const isDarkening = nextResolved === 'dark';
    const maxDist = Math.hypot(
      Math.max(x, window.innerWidth - x),
      Math.max(y, window.innerHeight - y)
    );

    // 设置 CSS 自定义属性供动画使用
    document.documentElement.style.setProperty('--ripple-x', `${x}px`);
    document.documentElement.style.setProperty('--ripple-y', `${y}px`);
    document.documentElement.style.setProperty('--ripple-r', `${maxDist}px`);

    const doc = document as any;
    // 检查 View Transitions API 是否可用
    if (!doc.startViewTransition) {
      // 不支持时直接切换
      setMode(nextMode);
      return;
    }

    // 标记方向，CSS 会根据这个类来决定动画方向
    document.documentElement.classList.add(isDarkening ? 'theme-darkening' : 'theme-lightening');

    const transition = doc.startViewTransition(() => {
      document.documentElement.setAttribute('data-theme', nextResolved);
      setMode(nextMode);
    });

    transition.finished.then(() => {
      document.documentElement.classList.remove('theme-darkening', 'theme-lightening');
      document.documentElement.style.removeProperty('--ripple-x');
      document.documentElement.style.removeProperty('--ripple-y');
      document.documentElement.style.removeProperty('--ripple-r');
    });
  }, [mode, resolved, systemDark, setMode]);

  return (
    <AppSettingsContext.Provider value={{ mode, setMode, resolved, monitorInterval, setMonitorInterval, cycleThemeWithRipple }}>
      {children}
    </AppSettingsContext.Provider>
  );
}
