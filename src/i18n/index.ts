import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import zhCN from './locales/zh-CN';
import en from './locales/en';
import ja from './locales/ja';

// 从 localStorage 读取已保存的语言偏好
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

// 切换语言并持久化
export function changeLanguage(lang: string) {
  i18n.changeLanguage(lang);
  localStorage.setItem('app_language', lang);
}

// 可用语言列表
export const availableLanguages = [
  { value: 'zh-CN', label: '简体中文' },
  { value: 'en', label: 'English' },
  { value: 'ja', label: '日本語' },
];
