import { useEffect, useRef, useState, type MouseEvent } from 'react';

/** 按下分隔条后跟随鼠标，松开或卸载时移除监听并恢复光标 */
function useDragSession() {
  const cleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanup.current?.(), []);
  return (event: MouseEvent, cursor: string, onMove: (e: globalThis.MouseEvent) => void) => {
    event.preventDefault();
    cleanup.current?.();
    const oldCursor = document.body.style.cursor;
    const stop = () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', stop);
      document.body.style.cursor = oldCursor;
      cleanup.current = null;
    };
    cleanup.current = stop;
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', stop);
    document.body.style.cursor = cursor;
  };
}

/** 拖动改变一块面板的像素宽度（axis x）或高度（axis y）；direction -1 表示分隔条在面板左侧/上方 */
export function useDragResize({ initial, min, max, axis = 'x', direction = 1, storageKey }: {
  initial: number; min: number; max: number; axis?: 'x' | 'y'; direction?: number; storageKey?: string;
}) {
  const [size, setSize] = useState(() => {
    const saved = storageKey ? Number(localStorage.getItem(storageKey) ?? initial) : initial;
    return Number.isFinite(saved) ? Math.max(min, Math.min(max, saved)) : initial;
  });
  const startDrag = useDragSession();
  const onMouseDown = (event: MouseEvent) => {
    const start = axis === 'x' ? event.clientX : event.clientY;
    startDrag(event, axis === 'x' ? 'col-resize' : 'row-resize', e => {
      const next = Math.max(min, Math.min(max, size + direction * ((axis === 'x' ? e.clientX : e.clientY) - start)));
      setSize(next);
      if (storageKey) { try { localStorage.setItem(storageKey, String(next)); } catch { /* 配额满等，忽略 */ } }
    });
  };
  return { size, onMouseDown };
}

/**
 * 上下两块按 flex 比例分高度时，分隔条拖动 delta 像素后的新比例（上 : 下）。
 * before / after 是按下时量出的两块高度，按像素换算，分隔条与鼠标同步移动。
 */
export function flexRatioAfterDrag(before: number, after: number, delta: number, min: number, max: number): number {
  const ratio = after - delta > 0 ? (before + delta) / (after - delta) : max;
  return Math.max(min, Math.min(max, ratio));
}

/** 上下两块按 flex 比例分高度（上块 flex: ratio，下块 flex: 1）的分隔条 */
export function useFlexRatioResize({ initial, min, max }: { initial: number; min: number; max: number }) {
  const [ratio, setRatio] = useState(initial);
  const beforeRef = useRef<HTMLDivElement>(null);
  const afterRef = useRef<HTMLDivElement>(null);
  const startDrag = useDragSession();
  const onMouseDown = (event: MouseEvent) => {
    const before = beforeRef.current?.getBoundingClientRect().height ?? 0;
    const after = afterRef.current?.getBoundingClientRect().height ?? 0;
    if (before <= 0 || after <= 0) return;
    const start = event.clientY;
    startDrag(event, 'row-resize', e => setRatio(flexRatioAfterDrag(before, after, e.clientY - start, min, max)));
  };
  return { ratio, beforeRef, afterRef, onMouseDown };
}
