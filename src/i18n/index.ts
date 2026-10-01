import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import zhCN from './locales/zh-CN';
import en from './locales/en';
import ja from './locales/ja';

const savedLang = localStorage.getItem('app_language') || 'zh-CN';

i18n
  .use(initReactI18next)
  .init({
    resources: {
      'zh-CN': { translation: zhCN },
      en: { translation: en },
      ja: { translation: ja },
    },
    lng: savedLang,
    fallbackLng: 'zh-CN',
    interpolation: {
      escapeValue: false, // React 已自动转义
    },
  });

export default i18n;

export function changeLanguage(lang: string) {
  i18n.changeLanguage(lang);
  localStorage.setItem('app_language', lang);
}

export const availableLanguages = ['zh-CN', 'en', 'ja'];
