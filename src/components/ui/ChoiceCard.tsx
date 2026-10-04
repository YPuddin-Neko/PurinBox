import type { KeyboardEvent, MouseEvent, ReactNode } from 'react';
import './ui.css';

export interface ChoiceCardProps {
  selected: boolean;
  /** 点击或空格/回车时调用；单选传 () => setValue(x)，多选传 () => toggle(x) */
  onSelect: () => void;
  /** 选中标记：radio 圆点 / check 勾选框；不传则不画标记 */
  indicator?: 'radio' | 'check';
  /** danger：选中时用红色边框和底色（如删除类操作） */
  tone?: 'accent' | 'danger';
  /** 较小的内边距与标记尺寸 */
  compact?: boolean;
  /** column：标记与内容纵向排列、水平居中（如图标 + 标题 + 说明的操作卡片） */
  layout?: 'row' | 'column';
  /** 标记所在行之下的内容；传了它卡片变成两段（标题行 + body） */
  body?: ReactNode;
  /** 与标记同一行的内容 */
  children?: ReactNode;
}

// 卡片里的这些控件自己处理点击，点它们不切换卡片
const NESTED_CONTROLS = 'input, textarea, select, button, a[href], label, [role="switch"], [role="tab"], [contenteditable="true"]';

/** 纯视觉的可选卡片：内部不放 input，点击事件只触发一次 onSelect */
export default function ChoiceCard({
  selected,
  onSelect,
  indicator,
  tone = 'accent',
  compact = false,
  layout = 'row',
  body,
  children,
}: ChoiceCardProps) {
  const handleClick = (e: MouseEvent<HTMLDivElement>) => {
    const control = e.target instanceof Element ? e.target.closest(NESTED_CONTROLS) : null;
    if (control && e.currentTarget.contains(control)) return;
    onSelect();
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault();
      onSelect();
    }
  };

  const classes = [
    'ui-choice',
    body === undefined ? 'ui-choice-inline' : '',
    selected ? 'is-selected' : '',
    compact ? 'is-compact' : '',
    layout === 'column' ? 'is-column' : '',
    tone === 'danger' ? 'is-danger' : '',
  ].filter(Boolean).join(' ');

  const mark = indicator && <span className={`ui-choice-indicator ui-choice-${indicator}`} aria-hidden="true" />;

  return (
    <div
      role={indicator === 'check' ? 'checkbox' : 'radio'}
      aria-checked={selected}
      tabIndex={0}
      className={classes}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
    >
      {body === undefined ? (
        <>{mark}{children}</>
      ) : (
        <>
          <div className="ui-choice-row">{mark}{children}</div>
          {body}
        </>
      )}
    </div>
  );
}
