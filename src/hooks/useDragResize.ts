import { useEffect, useRef, useState, type MouseEvent } from 'react';

export function useDragResize({ initial, min, max, axis = 'x', direction = 1, storageKey }: {
  initial: number; min: number; max: number; axis?: 'x' | 'y'; direction?: number; storageKey?: string;
}) {
  const [size, setSize] = useState(() => {
    const saved = storageKey ? Number(localStorage.getItem(storageKey) ?? initial) : initial;
    return Number.isFinite(saved) ? Math.max(min, Math.min(max, saved)) : initial;
  });
  const cleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanup.current?.(), []);
  const onMouseDown = (event: MouseEvent) => {
    event.preventDefault();
    cleanup.current?.();
    const start = axis === 'x' ? event.clientX : event.clientY;
    const oldCursor = document.body.style.cursor;
    const move = (e: globalThis.MouseEvent) => {
      const next = Math.max(min, Math.min(max, size + direction * ((axis === 'x' ? e.clientX : e.clientY) - start)));
      setSize(next);
      if (storageKey) { try { localStorage.setItem(storageKey, String(next)); } catch { /* Storage may be full. */ } }
    };
    const stop = () => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', stop);
      document.body.style.cursor = oldCursor;
      cleanup.current = null;
    };
    cleanup.current = stop;
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
    document.body.style.cursor = axis === 'x' ? 'col-resize' : 'row-resize';
  };
  return { size, onMouseDown };
}
