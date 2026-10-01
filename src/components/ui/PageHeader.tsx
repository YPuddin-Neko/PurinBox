import type { CSSProperties, ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';
import './ui.css';

export interface PageHeaderProps {
  icon: LucideIcon;
  /** 图标颜色 */
  color: string;
  title: ReactNode;
  subtitle?: ReactNode;
  /** 标题右侧的操作区（如工作流工具栏） */
  actions?: ReactNode;
  style?: CSSProperties;
}

export default function PageHeader({ icon: Icon, color, title, subtitle, actions, style }: PageHeaderProps) {
  const heading = (
    <>
      <div className="ui-page-heading">
        <Icon className="ui-page-icon" style={{ color }} />
        <h1 className="page-title">{title}</h1>
      </div>
      {subtitle !== undefined && <p className="page-subtitle">{subtitle}</p>}
    </>
  );

  if (actions === undefined) {
    return <div className="page-header" style={style}>{heading}</div>;
  }

  return (
    <div className="page-header ui-page-header-split" style={style}>
      <div>{heading}</div>
      {actions}
    </div>
  );
}
