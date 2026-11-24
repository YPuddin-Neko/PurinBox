import { useCallback, useEffect, useLayoutEffect, useRef, type CSSProperties, type ReactNode } from 'react';
import './ui.css';

export interface SegmentedTab<T extends string> {
  id: T;
  label: ReactNode;
}

export interface SegmentedTabsProps<T extends string> {
  tabs: readonly SegmentedTab<T>[];
  /** 只从 tabs 推断 T，value 写错 id 会报类型错误 */
  value: NoInfer<T>;
  onChange: (id: T) => void;
  className?: string;
  /** 外边距由调用方给，如 { marginBottom: 'var(--space-4)' } */
  style?: CSSProperties;
}

export default function SegmentedTabs<T extends string>({ tabs, value, onChange, className, style }: SegmentedTabsProps<T>) {
  const listRef = useRef<HTMLDivElement>(null);
  const pillRef = useRef<HTMLSpanElement>(null);
  const hasVisiblePillRef = useRef(false);
  const lastListWidthRef = useRef(0);

  const syncPill = useCallback((instant: boolean) => {
    const list = listRef.current;
    const pill = pillRef.current;
    if (!list || !pill) return;
    const activeBtn = list.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]');
    if (!activeBtn || activeBtn.offsetWidth === 0) {
      pill.style.opacity = '0';
      hasVisiblePillRef.current = false;
      return;
    }

    const snap = instant || !hasVisiblePillRef.current;
    if (snap) {
      pill.style.transition = 'none';
      pill.style.width = `${activeBtn.offsetWidth}px`;
      pill.style.transform = `translate3d(${activeBtn.offsetLeft}px, 0, 0)`;
      pill.style.opacity = '1';
      pill.getBoundingClientRect();
      pill.style.transition = '';
    } else {
      pill.style.width = `${activeBtn.offsetWidth}px`;
      pill.style.transform = `translate3d(${activeBtn.offsetLeft}px, 0, 0)`;
      pill.style.opacity = '1';
    }
    hasVisiblePillRef.current = true;
    lastListWidthRef.current = list.clientWidth;
  }, []);

  // 值变化时以弹簧动画平滑滑移胶囊
  useLayoutEffect(() => {
    syncPill(false);
  }, [value, tabs.length, syncPill]);

  // 仅在容器首次从隐藏切为可见或窗口缩放改变宽度时对齐，绝不打断点击切换动画
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const ro = new ResizeObserver(() => {
      const w = list.clientWidth;
      if (w === 0) {
        hasVisiblePillRef.current = false;
        return;
      }
      if (w !== lastListWidthRef.current || !hasVisiblePillRef.current) {
        syncPill(true);
      }
    });
    ro.observe(list);
    return () => ro.disconnect();
  }, [syncPill]);

  return (
    <div ref={listRef} role="tablist" className={className ? `ui-seg ${className}` : 'ui-seg'} style={style}>
      <span ref={pillRef} className="ui-seg-pill" aria-hidden="true" />
      {tabs.map(tab => {
        const active = tab.id === value;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={active}
            className={active ? 'ui-seg-tab is-active' : 'ui-seg-tab'}
            onClick={() => onChange(tab.id)}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
