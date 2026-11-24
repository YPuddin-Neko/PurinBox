import { useId } from 'react';

interface PurinBrandIconProps {
  size?: number;
  className?: string;
}

/**
 * 基于 PurinBox 启动器图标重新设计的矢量品牌图标：
 * 奶油米白圆角底 + 斜掀盖等距甜品盒 + 焦糖淋面布丁 + 奶油樱桃 + 柔金星芒
 */
export function PurinBrandIcon({ size = 32, className }: PurinBrandIconProps) {
  const uid = useId().replace(/:/g, '');
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      className={className}
      aria-hidden="true"
      style={{ flexShrink: 0, display: 'block', filter: 'drop-shadow(0 1px 3px rgba(130, 82, 36, 0.16))' }}
    >
      <defs>
        <linearGradient id={`${uid}_bg`} x1="6" y1="4" x2="58" y2="60" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#FFFDF8" />
          <stop offset="100%" stopColor="#FBEFD9" />
        </linearGradient>
        <linearGradient id={`${uid}_lid`} x1="20" y1="10" x2="50" y2="30" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#FFF9EC" />
          <stop offset="100%" stopColor="#F2D7B1" />
        </linearGradient>
        <linearGradient id={`${uid}_boxL`} x1="15" y1="34" x2="32" y2="54" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#FFF7E8" />
          <stop offset="100%" stopColor="#F6DCBA" />
        </linearGradient>
        <linearGradient id={`${uid}_boxR`} x1="32" y1="34" x2="49" y2="54" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#F8E2C2" />
          <stop offset="100%" stopColor="#EBC597" />
        </linearGradient>
        <linearGradient id={`${uid}_pud`} x1="22" y1="28" x2="42" y2="42" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#FFEB8E" />
          <stop offset="100%" stopColor="#F4B038" />
        </linearGradient>
        <linearGradient id={`${uid}_car`} x1="22" y1="22" x2="42" y2="36" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#E78D29" />
          <stop offset="65%" stopColor="#BC5715" />
          <stop offset="100%" stopColor="#8B3A0C" />
        </linearGradient>
        <radialGradient id={`${uid}_chr`} cx="38%" cy="32%" r="65%">
          <stop offset="0%" stopColor="#FF7A85" />
          <stop offset="55%" stopColor="#DF3547" />
          <stop offset="100%" stopColor="#9E1B2B" />
        </radialGradient>
      </defs>

      {/* Squircle Base */}
      <rect x="2" y="2" width="60" height="60" rx="15" fill={`url(#${uid}_bg)`} stroke="#E6C9A5" strokeWidth="1.4" />

      {/* Soft Gold 4-Point Sparkles */}
      <path d="M11.5 16.5 L12.7 19.3 L15.5 20.5 L12.7 21.7 L11.5 24.5 L10.3 21.7 L7.5 20.5 L10.3 19.3 Z" fill="#DDB074" />
      <path d="M52 38 L52.9 40.1 L55 41 L52.9 41.9 L52 44 L51.1 41.9 L49 41 L51.1 40.1 Z" fill="#DDB074" />
      <circle cx="10.5" cy="30" r="1.4" fill="#E4C192" />
      <circle cx="53" cy="31" r="1.3" fill="#E4C192" />

      {/* Open Tilted Box Lid */}
      <path
        d="M21.4 15.4 C22.5 12.2 25.5 11.4 28.8 12.2 L44.4 16.4 C47.2 17.2 48.6 19.4 49.5 22.8 L51.1 28.5 C51.6 30.2 50.6 31.8 48.9 32.3 L32 28.2 L20.9 19.2 C19.8 18.3 20.4 16.8 21.4 15.4 Z"
        fill={`url(#${uid}_lid)`}
        stroke="#B88152"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path d="M23.5 18.6 L43.2 23.4 L46.2 29.8 L30.8 26.2 Z" fill="#DEC09A" opacity="0.58" />

      {/* Box Cavity Rim */}
      <path d="M16.5 34.5 L32 27 L47.5 34.5 L32 42.5 Z" fill="#EBD0AD" stroke="#B88152" strokeWidth="1.4" strokeLinejoin="round" />

      {/* Custard Pudding Body */}
      <path
        d="M21.2 37.8 L23.4 27.6 C24.4 24.2 39.6 24.2 40.6 27.6 L42.8 37.8 C38.5 41.2 25.5 41.2 21.2 37.8 Z"
        fill={`url(#${uid}_pud)`}
        stroke="#A76228"
        strokeWidth="1.3"
      />

      {/* Dripping Glossy Caramel Glaze */}
      <path
        d="M23.2 28.6 C24 24.2 40 24.2 40.8 28.6 C41.2 30.4 40 31.8 38.4 31.4 C37 31 36.6 33.4 35.4 35.6 C34.5 37.2 33 36.2 33 34.2 C33 32.4 31.4 31.8 29.8 32.6 C28.2 33.4 27 31.8 26.2 30.8 C25 29.8 23.4 30.2 23.2 28.6 Z"
        fill={`url(#${uid}_car)`}
        stroke="#8F3F12"
        strokeWidth="1.1"
      />
      <ellipse cx="27.2" cy="27.2" rx="2.4" ry="0.9" fill="#FFF7DF" opacity="0.55" transform="rotate(-14 27.2 27.2)" />

      {/* Whipped Cream Dollop */}
      <path
        d="M27.4 25.8 C27.2 23.4 29.2 22.2 32 22.2 C34.8 22.2 36.8 23.4 36.6 25.8 C36.4 27.4 34.4 28.1 32 28.1 C29.6 28.1 27.6 27.4 27.4 25.8 Z"
        fill="#FFFDF8"
        stroke="#C8A27A"
        strokeWidth="1.1"
      />

      {/* Glossy Cherry + Stem */}
      <circle cx="32" cy="21.4" r="3.4" fill={`url(#${uid}_chr)`} stroke="#8F1D2B" strokeWidth="1" />
      <ellipse cx="30.8" cy="20.2" rx="1.1" ry="0.7" fill="#FFFFFF" opacity="0.85" transform="rotate(-25 30.8 20.2)" />
      <path d="M32.2 18.2 C32.6 16.2 34.2 14.8 35.6 14.6" stroke="#6E351B" strokeWidth="1.35" strokeLinecap="round" />

      {/* Isometric Cream Front Box */}
      <path
        d="M16 34.8 L32 42.8 L32 54.6 L17.6 47.9 C16.5 47.4 16 46.3 16 45.0 Z"
        fill={`url(#${uid}_boxL)`}
        stroke="#B88152"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path
        d="M48 34.8 L32 42.8 L32 54.6 L46.4 47.9 C47.5 47.4 48 46.3 48 45.0 Z"
        fill={`url(#${uid}_boxR)`}
        stroke="#B88152"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />

      {/* Box Specular Highlights */}
      <path d="M18.5 39.2 L18.5 44.5" stroke="#FFFFFF" strokeWidth="1.6" strokeLinecap="round" opacity="0.75" />
      <path d="M32 44 L32 52.8" stroke="#FFFFFF" strokeWidth="1.3" strokeLinecap="round" opacity="0.55" />
    </svg>
  );
}

/** 首页三大功能分类定制双色图标，与品牌图标柔陶质感统一 */
export function DomainArtIcon({ domain }: { domain: 'preprocess' | 'dataset' | 'advanced' }) {
  if (domain === 'preprocess') {
    return (
      <svg width="34" height="34" viewBox="0 0 34 34" fill="none" aria-hidden="true">
        <rect x="1" y="1" width="32" height="32" rx="10" fill="var(--dom-pre-bg)" />
        <path d="M11 8.5V21a2.2 2.2 0 0 0 2.2 2.2H25.5" stroke="var(--dom-pre)" strokeWidth="2.1" strokeLinecap="round" />
        <path d="M23 25.5V13a2.2 2.2 0 0 0-2.2-2.2H8.5" stroke="var(--dom-pre)" strokeWidth="2.1" strokeLinecap="round" opacity="0.58" />
        <circle cx="23.4" cy="10.6" r="2.2" fill="var(--dom-pre)" />
      </svg>
    );
  }
  if (domain === 'dataset') {
    return (
      <svg width="34" height="34" viewBox="0 0 34 34" fill="none" aria-hidden="true">
        <rect x="1" y="1" width="32" height="32" rx="10" fill="var(--dom-dat-bg)" />
        <path
          d="M17.2 8.8H10.4a1.6 1.6 0 0 0-1.6 1.6v6.8l8.1 8.1a2.1 2.1 0 0 0 3 0l5.4-5.4a2.1 2.1 0 0 0 0-3l-8.1-8.1Z"
          fill="var(--dom-dat)"
          fillOpacity="0.18"
          stroke="var(--dom-dat)"
          strokeWidth="2"
          strokeLinejoin="round"
        />
        <circle cx="13.2" cy="13.2" r="1.7" fill="var(--dom-dat)" />
      </svg>
    );
  }
  return (
    <svg width="34" height="34" viewBox="0 0 34 34" fill="none" aria-hidden="true">
      <rect x="1" y="1" width="32" height="32" rx="10" fill="var(--dom-adv-bg)" />
      <rect x="9" y="9" width="6.8" height="6.8" rx="2" fill="var(--dom-adv)" fillOpacity="0.22" stroke="var(--dom-adv)" strokeWidth="1.9" />
      <rect x="18.2" y="9" width="6.8" height="6.8" rx="2" stroke="var(--dom-adv)" strokeWidth="1.9" />
      <rect x="9" y="18.2" width="6.8" height="6.8" rx="2" stroke="var(--dom-adv)" strokeWidth="1.9" />
      <rect x="18.2" y="18.2" width="6.8" height="6.8" rx="2" fill="var(--dom-adv)" fillOpacity="0.22" stroke="var(--dom-adv)" strokeWidth="1.9" />
    </svg>
  );
}
