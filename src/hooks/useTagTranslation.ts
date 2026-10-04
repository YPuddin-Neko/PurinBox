import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../utils/tauriRuntime';

type TranslationResult = { translations: { source: string; translated: string }[] };
// 翻译进度事件不带请求 ID：三个常驻编辑器的翻译请求排队依次执行
let translationQueue: Promise<unknown> = Promise.resolve();

export function useTagTranslation() {
  const [translations, setTranslations] = useState<Record<string, string>>({});
  const [translating, setTranslating] = useState(false);
  const [translateProgress, setTranslateProgress] = useState<{ current: number; total: number } | null>(null);
  const mounted = useRef(true);
  const busy = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; clearTimeout(timer.current); }; }, []);
  const translate = useCallback(async (tags: string[], mode: 'tags' | 'text' = 'tags') => {
    if (!tags.length || busy.current) return undefined;
    busy.current = true;
    setTranslating(true);
    clearTimeout(timer.current);
    const run = async () => {
      if (!mounted.current) return undefined;
      let unlisten: (() => void) | undefined;
      try {
        setTranslateProgress({ current: 0, total: tags.length });
        unlisten = await listen<{ current: number; total: number }>('translate-progress', ({ payload }) => {
          if (mounted.current) setTranslateProgress(payload);
        });
        const value = (key: string, fallback = '') => localStorage.getItem(key) || fallback;
        const result = await invoke<TranslationResult>('translate_tags', {
          tags, targetLang: value('translate_target_lang', 'zh-CN'), provider: value('translate_provider', 'google'),
          baiduAppid: value('baidu_appid'), baiduKey: value('baidu_key'), youdaoAppKey: value('youdao_app_key'),
          youdaoAppSecret: value('youdao_app_secret'), bingKey: value('bing_key'), bingRegion: value('bing_region'),
          ...(mode === 'text' ? { translateMode: 'text' } : {}),
        });
        if (mounted.current) {
          setTranslations(prev => ({ ...prev, ...Object.fromEntries(result.translations.filter(v => v.translated).map(v => [v.source, v.translated])) }));
          setTranslateProgress({ current: tags.length, total: tags.length });
          timer.current = setTimeout(() => { if (mounted.current) setTranslateProgress(null); }, 3000);
        }
        return result;
      } catch (error) {
        if (mounted.current) setTranslateProgress(null);
        throw error;
      } finally { unlisten?.(); }
    };
    const pending = translationQueue.then(run, run);
    translationQueue = pending.catch(() => {});
    try { return await pending; }
    finally { busy.current = false; if (mounted.current) setTranslating(false); }
  }, []);
  return { translations, translating, translateProgress, translate };
}
