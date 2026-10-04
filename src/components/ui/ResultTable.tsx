import type { ReactNode } from 'react';
import './ui.css';

export interface ResultTableColumn {
  label: ReactNode;
  /** 默认 left */
  align?: 'left' | 'center';
  /** 列宽（px） */
  width?: number;
  /** React key；label 不是字符串时传 */
  key?: string;
}

export interface ResultTableProps {
  title: ReactNode;
  /** 标题栏右侧的内容（如条数） */
  extra?: ReactNode;
  columns: readonly ResultTableColumn[];
  /** 表格行 */
  children: ReactNode;
  /** 面板底部、滚动区之外的内容（如 <Pager footer />） */
  footer?: ReactNode;
}

/**
 * 结果列表面板：标题栏 + 表格 + 底部栏。面板占满纵向 flex 父级的剩余高度，只有表格区滚动。
 *
 * 例：`<ResultTable title={t('sdMetadata.metaList')} columns={[{ label: '#', width: 30 }, { label: t('sdMetadata.filename') }]}
 *   footer={pages > 1 && <Pager footer page={page} pages={pages} onChange={setPage} />}>{rows}</ResultTable>`
 */
export default function ResultTable({ title, extra, columns, children, footer }: ResultTableProps) {
  return (
    <div className="ui-result-table">
      <div className="ui-result-table-head">
        <span className="ui-result-table-title">{title}</span>
        {extra}
      </div>
      <div className="ui-result-table-scroll">
        <table className="ui-result-table-grid">
          <thead>
            <tr>
              {columns.map((column, index) => (
                <th key={column.key ?? (typeof column.label === 'string' ? column.label : String(index))}
                  style={{ textAlign: column.align ?? 'left', width: column.width }}>
                  {column.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>{children}</tbody>
        </table>
      </div>
      {footer}
    </div>
  );
}
