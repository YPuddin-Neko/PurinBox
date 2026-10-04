import type { ReactNode } from 'react';
import './ui.css';

export interface StatChipItem {
  label: ReactNode;
  value: ReactNode;
  /** 数值颜色；可点击时也用于边框和箭头 */
  color: string;
  /** 传了才可点击：按钮语义、带色边框和右侧箭头（如打开未匹配文件列表） */
  onClick?: () => void;
  /** React key；label 不是字符串时传 */
  key?: string;
}

export interface StatChipsProps {
  items: readonly StatChipItem[];
  /** sm（默认）：16px 数值、8px 10px 留白；md：18px 数值、10px 14px 留白 */
  size?: 'sm' | 'md';
}

/**
 * 结果区顶部的统计条，每项等宽。
 *
 * 例：`<StatChips items={[
 *   { label: t('dedupRename.totalA'), value: totalA, color: '#60a5fa' },
 *   { label: t('dedupRename.unmatchA'), value: unmatchedA.length, color: '#f59e0b',
 *     onClick: unmatchedA.length > 0 ? () => setUnmatchModal('a') : undefined },
 * ]} />`
 */
export default function StatChips({ items, size = 'sm' }: StatChipsProps) {
  return (
    <div className={size === 'md' ? 'ui-stats is-md' : 'ui-stats'}>
      {items.map((item, index) => {
        const key = item.key ?? (typeof item.label === 'string' ? item.label : String(index));
        const content = (
          <>
            <span className="ui-stat-value" style={{ color: item.color }}>{item.value}</span>
            <span className="ui-stat-label">{item.label}</span>
            {item.onClick && <span className="ui-stat-more" style={{ color: item.color }} aria-hidden="true">↗</span>}
          </>
        );
        return item.onClick ? (
          <button key={key} type="button" className="ui-stat is-clickable" onClick={item.onClick}
            style={{ borderColor: `color-mix(in srgb, ${item.color} 20%, transparent)` }}>
            {content}
          </button>
        ) : (
          <div key={key} className="ui-stat">{content}</div>
        );
      })}
    </div>
  );
}
