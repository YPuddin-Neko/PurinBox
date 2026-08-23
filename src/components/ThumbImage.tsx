import { useEffect, useState, CSSProperties } from 'react';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { ensureAssetScope } from '../utils/assetScope';
import { hasTauriRuntime } from '../utils/tauriRuntime';

// 已解析的缩略图 URL（path|maxEdge → asset URL），进程生命周期内有效
const resolvedCache = new Map<string, string>();
// 进行中的解析，按键去重，避免同图重复 invoke
const pendingCache = new Map<string, Promise<string>>();

// 并发闸：同屏几十个格子同时请求会让后端同时解码几十张大图
const MAX_CONCURRENT = 4;
let activeCount = 0;
const waitQueue: (() => void)[] = [];

function acquireSlot(): Promise<void> {
  if (activeCount < MAX_CONCURRENT) {
    activeCount++;
    return Promise.resolve();
  }
  return new Promise(resolve => waitQueue.push(() => { activeCount++; resolve(); }));
}

function releaseSlot() {
  activeCount--;
  const next = waitQueue.shift();
  if (next) next();
}

function resolveThumb(path: string, maxEdge: number): Promise<string> {
  const key = `${path}|${maxEdge}`;
  const hit = resolvedCache.get(key);
  if (hit) return Promise.resolve(hit);
  let pending = pendingCache.get(key);
  if (!pending) {
    pending = (async () => {
      await acquireSlot();
      try {
        // 放行原图所在目录：缩略图失败的兜底直显、以及同目录的灯箱原图都依赖它
        await ensureAssetScope(path, { file: true });
        const thumbPath = await invoke<string>('get_image_thumbnail', { path, maxEdge });
        const url = convertFileSrc(thumbPath);
        resolvedCache.set(key, url);
        return url;
      } finally {
        releaseSlot();
        pendingCache.delete(key);
      }
    })();
    pendingCache.set(key, pending);
  }
  return pending;
}

interface ThumbImageProps {
  /** 原图绝对路径 */
  path: string;
  /** 缩略图最长边（默认 384，网格格子够用；中等预览建议 1024） */
  maxEdge?: number;
  alt?: string;
  style?: CSSProperties;
  className?: string;
  draggable?: boolean;
  onClick?: () => void;
  onDragStart?: (e: React.DragEvent<HTMLElement>) => void;
}

/**
 * 缩略图 <img>：经后端 get_image_thumbnail 生成/命中缓存后加载，
 * 替代 convertFileSrc(原图) 直出，避免 WebView 解码全尺寸大图导致卡顿。
 * 生成失败或缩略图不可用时回退加载原图；大图查看（Lightbox）请继续用原图。
 */
export default function ThumbImage({ path, maxEdge = 384, alt, style, className, draggable, onClick, onDragStart }: ThumbImageProps) {
  const key = `${path}|${maxEdge}`;
  const [src, setSrc] = useState<string>(() => resolvedCache.get(key) ?? '');

  useEffect(() => {
    const cached = resolvedCache.get(key);
    if (cached) {
      setSrc(cached);
      return;
    }
    if (!hasTauriRuntime()) {
      setSrc(convertFileSrc(path));
      return;
    }
    let alive = true;
    setSrc('');
    resolveThumb(path, maxEdge)
      .then(url => { if (alive) setSrc(url); })
      .catch(() => { if (alive) setSrc(convertFileSrc(path)); });
    return () => { alive = false; };
  }, [key, path, maxEdge]);

  if (!src) {
    // 占位块保持格子布局稳定，避免加载完成时跳动
    return <div className={className} style={{ background: 'var(--color-bg-tertiary)', ...style }} onClick={onClick} />;
  }
  return (
    <img
      src={src}
      alt={alt}
      style={style}
      className={className}
      draggable={draggable}
      loading="lazy"
      decoding="async"
      onClick={onClick}
      onDragStart={onDragStart}
      onError={() => {
        // 缓存文件可能被外部清理：失效并回退原图
        resolvedCache.delete(key);
        setSrc(convertFileSrc(path));
      }}
    />
  );
}
