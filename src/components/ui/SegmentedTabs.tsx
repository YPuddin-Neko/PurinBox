import type { CSSProperties, ReactNode } from 'react';
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
  return (
    <div role="tablist" className={className ? `ui-seg ${className}` : 'ui-seg'} style={style}>
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
