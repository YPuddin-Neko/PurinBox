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
    <div className="ui-page-hero">
      <div
        className="ui-page-icon-badge"
        style={{
          color,
          background: `color-mix(in srgb, ${color} 12%, transparent)`,
        }}
      >
        <Icon className="ui-page-icon" />
      </div>
      <div className="ui-page-title-block">
        <h1 className="page-title">{title}</h1>
        {subtitle !== undefined && <p className="page-subtitle">{subtitle}</p>}
      </div>
    </div>
  );

  if (actions === undefined) {
    return <div className="page-header" style={style}>{heading}</div>;
  }

  return (
    <div className="page-header ui-page-header-split" style={style}>
      {heading}
      <div className="ui-page-actions">{actions}</div>
    </div>
  );
}
