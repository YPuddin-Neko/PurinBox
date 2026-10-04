import type { ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';
import PageHeader from './PageHeader';
import './ui.css';

export interface ToolPageLayoutProps {
  /** 以下四项同 PageHeader */
  icon: LucideIcon;
  color: string;
  title: ReactNode;
  subtitle?: ReactNode;
  /** 右栏（360px），通常是 <ProcessButton> 与 <ProgressLog>，其后可接结果面板 */
  aside: ReactNode;
  /** 左栏：路径与参数面板 */
  children: ReactNode;
  /** 两栏间距，默认 var(--space-6) */
  gap?: string;
}

/**
 * 处理页骨架：页头 + 左参数、右执行与日志的两栏。
 *
 * 例：
 * ```tsx
 * <ToolPageLayout icon={Crop} color="#34d399" title={t('crop.title')} subtitle={t('crop.subtitle')}
 *   aside={<>
 *     <ProcessButton {...task.buttonProps} onStart={handleProcess} disabled={!inputPath} cancelCommand="cancel_crop" />
 *     <ProgressLog {...task.progressLogProps} />
 *   </>}>
 *   <PathFields ... />
 * </ToolPageLayout>
 * ```
 */
export default function ToolPageLayout({ icon, color, title, subtitle, aside, children, gap }: ToolPageLayoutProps) {
  return (
    <div className="page">
      <PageHeader icon={icon} color={color} title={title} subtitle={subtitle} />
      <div className="ui-tool-grid" style={gap ? { gap } : undefined}>
        <div className="ui-tool-col">{children}</div>
        <div className="ui-tool-col">{aside}</div>
      </div>
    </div>
  );
}
