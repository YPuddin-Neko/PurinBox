import { useLayoutEffect, type CSSProperties, type ReactNode } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import './ui.css';

export interface PagerProps {
  /** 当前页（从 0 开始）；越界时组件自己夹紧并通过 onChange 回写 */
  page: number;
  /** 总页数；小于 1 按 1 页处理 */
  pages: number;
  onChange: (page: number) => void;
  /** md（默认）：28px 按钮；sm：紧凑版 */
  size?: 'sm' | 'md';
  /** 列表底部的居中翻页栏（上边框 + 上下留白） */
  footer?: boolean;
  /** 自定义页码文本，参数为从 1 开始的页码；默认 "页码 / 总页数" */
  formatLabel?: (page: number, pages: number) => ReactNode;
  prevTitle?: string;
  nextTitle?: string;
  /** 翻页按钮之后的附加内容（如总条数） */
  children?: ReactNode;
  className?: string;
  style?: CSSProperties;
}

/** 把页码夹到 [0, pages - 1]；页面切片列表时也用它，避免删除数据后出现空页 */
export function clampPage(page: number, pages: number): number {
  const last = Number.isFinite(pages) && pages > 1 ? Math.floor(pages) - 1 : 0;
  if (!Number.isFinite(page) || page <= 0) return 0;
  return Math.min(Math.floor(page), last);
}

export default function Pager({
  page,
  pages,
  onChange,
  size = 'md',
  footer = false,
  formatLabel,
  prevTitle,
  nextTitle,
  children,
  className,
  style,
}: PagerProps) {
  const pageCount = Number.isFinite(pages) && pages > 1 ? Math.floor(pages) : 1;
  const current = clampPage(page, pageCount);

  // 数据变少（如删除分组）后页码越界：在绘制前回写夹紧后的页码
  useLayoutEffect(() => {
    if (current !== page) onChange(current);
  }, [current, page, onChange]);

  const classes = [
    'ui-pager',
    size === 'sm' ? 'is-sm' : '',
    footer ? 'is-footer' : '',
    className ?? '',
  ].filter(Boolean).join(' ');

  return (
    <div className={classes} style={style}>
      <button type="button" className="btn btn-ghost ui-pager-btn" title={prevTitle} aria-label={prevTitle}
        disabled={current <= 0} onClick={() => onChange(current - 1)}>
        <ChevronLeft />
      </button>
      <span className="ui-pager-label">
        {formatLabel ? formatLabel(current + 1, pageCount) : `${current + 1} / ${pageCount}`}
      </span>
      <button type="button" className="btn btn-ghost ui-pager-btn" title={nextTitle} aria-label={nextTitle}
        disabled={current >= pageCount - 1} onClick={() => onChange(current + 1)}>
        <ChevronRight />
      </button>
      {children}
    </div>
  );
}
