import { useId, useState, type CSSProperties, type MouseEvent, type ReactNode, type Ref } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ChevronLeft, ChevronRight, Image as ImageIcon, Languages, Loader2, Save, type LucideIcon } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { useTagStats } from '../hooks/useTagStats';
import type { useTagTranslation } from '../hooks/useTagTranslation';
import ThumbImage from './ThumbImage';
import ImageLightbox from './ImageLightbox';
import '../styles/tags.css';

/** 标签编辑三种模式共用的面板标题栏：图标、标题、标题后的附加信息与右侧操作 */
export function TagPanelHeader({ icon: Icon, color, title, extra, children }: {
  icon: LucideIcon;
  color: string;
  title: ReactNode;
  /** 标题后的计数、文件名等 */
  extra?: ReactNode;
  /** 右侧的按钮 */
  children?: ReactNode;
}) {
  return (
    <div className="tag-panel-header">
      <div className="tag-panel-heading">
        <Icon style={{ width: 14, height: 14, color }} />
        <span className="tag-panel-title">{title}</span>
        {extra}
      </div>
      {children}
    </div>
  );
}

/** 面板标题栏里保存当前图片的按钮 */
export function PanelSaveButton({ saving, disabled, onClick }: { saving: boolean; disabled: boolean; onClick: () => void }) {
  const { t } = useTranslation();
  return (
    <button className="btn btn-primary tag-header-btn" disabled={disabled || saving} onClick={onClick}>
      {saving
        ? <Loader2 style={{ width: 10, height: 10, animation: 'spin 1s linear infinite' }} />
        : <Save style={{ width: 10, height: 10 }} />} {t('common.save')}
    </button>
  );
}

/** 标签统计标题栏的「翻译标签」与「公共/全部」切换；设置里没开翻译时翻译按钮禁用 */
export function TagStatsActions({ stats, translation, disabled, onError, spinner = false }: {
  stats: Pick<ReturnType<typeof useTagStats>, 'tagStats' | 'tagListMode' | 'setTagListMode'>;
  translation: Pick<ReturnType<typeof useTagTranslation>, 'translations' | 'translating' | 'translate'>;
  /** 没有图片时两个按钮都禁用 */
  disabled: boolean;
  onError: (message: string) => void;
  /** 翻译中把翻译按钮的图标换成转圈 */
  spinner?: boolean;
}) {
  const { t } = useTranslation();
  const { tagStats, tagListMode, setTagListMode } = stats;
  const { translations, translating, translate } = translation;
  const handleTranslate = async () => {
    if (localStorage.getItem('translate_enabled') !== 'true') return;
    try { await translate(tagStats.map(([tag]) => tag)); }
    catch (error) { onError(`${t('tagEditor.translateFail')}: ${String(error)}`); }
  };
  return <>
    <button className="btn btn-ghost btn-sm tag-icon-btn" title={t('tagEditor.translateTags')} onClick={handleTranslate}
      disabled={disabled || localStorage.getItem('translate_enabled') !== 'true' || translating}
      style={{ color: Object.keys(translations).length > 0 ? '#60a5fa' : undefined }}>
      {spinner && translating
        ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} />
        : <Languages style={{ width: 12, height: 12 }} />}
    </button>
    <button className="btn btn-ghost btn-sm tag-text-btn" onClick={() => setTagListMode(m => m === 'all' ? 'common' : 'all')} disabled={disabled}>
      {tagListMode === 'all' ? t('tagEditor.commonLabel') : t('tagEditor.allLabel')}
    </button>
  </>;
}

/** 面板之间的拖动分隔条；axis x 调宽度，y 调高度 */
export function ResizeHandle({ axis, onMouseDown }: { axis: 'x' | 'y'; onMouseDown: (event: MouseEvent) => void }) {
  const { t } = useTranslation();
  return (
    <div role="separator" aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
      className={`tag-resize-handle is-${axis}`} onMouseDown={onMouseDown}
      title={t(axis === 'x' ? 'tagEditor.dragWidth' : 'tagEditor.dragHeight')}>
      <span />
    </div>
  );
}

export interface ToolSidebarItem {
  icon: LucideIcon;
  /** 按钮的提示与无障碍名称，同一侧栏内不重复 */
  label: string;
  onClick: () => void;
  disabled?: boolean;
  color?: string;
}

/** 标签统计旁的竖排工具按钮 */
export function ToolSidebar({ items }: { items: readonly ToolSidebarItem[] }) {
  return (
    <div className="tag-tool-sidebar">
      {items.map(item => (
        <button key={item.label} type="button" className="btn btn-ghost" title={item.label} aria-label={item.label}
          disabled={item.disabled} onClick={item.onClick} style={{ color: item.color }}>
          <item.icon style={{ width: 14, height: 14 }} />
        </button>
      ))}
    </div>
  );
}

/** 单选的一组选项按钮（批量弹窗的目标字段、添加位置、应用范围） */
export function OptionChips<T extends string>({ legend, options, value, onChange, color }: {
  legend: string;
  options: readonly { value: T; label: string; disabled?: boolean }[];
  value: T;
  onChange: (value: T) => void;
  /** 选中项的颜色，默认紫色 */
  color?: string;
}) {
  const name = useId();
  return (
    <fieldset className="tag-batch-fieldset">
      <legend>{legend}</legend>
      <div className="tag-batch-options" style={color ? { '--option-color': color } as CSSProperties : undefined}>
        {options.map(option => (
          <label key={option.value} className="tag-option">
            <input type="radio" name={name} checked={value === option.value} disabled={option.disabled}
              onChange={() => onChange(option.value)} />
            {option.label}
          </label>
        ))}
      </div>
    </fieldset>
  );
}

/** 图片预览面板：文件名、上一张/下一张、缩略图，点击图片看大图 */
export function ImagePreviewPane({ path, filename, index, total, onPrev, onNext, badge, style, ref }: {
  /** 当前选中的图片；未选中时不传 */
  path?: string;
  filename?: string;
  index: number;
  total: number;
  onPrev: () => void;
  onNext: () => void;
  /** 文件名后的标记，如 JSON 解析失败 */
  badge?: ReactNode;
  /** 外框尺寸：flex 比例或固定高度 */
  style?: CSSProperties;
  ref?: Ref<HTMLDivElement>;
}) {
  const { t } = useTranslation();
  const [zoomed, setZoomed] = useState(false);
  return (
    <div ref={ref} className="tag-card" style={style}>
      <TagPanelHeader icon={ImageIcon} color="#7c5cfc" title={t('tagEditor.preview')}
        extra={<>{filename && <span className="tag-panel-note">{filename}</span>}{badge}</>}>
        {total > 0 && (
          <div className="tag-preview-nav">
            <button type="button" className="btn btn-ghost btn-sm" onClick={onPrev} disabled={index <= 0}>
              <ChevronLeft style={{ width: 14, height: 14 }} />
            </button>
            <span>{index + 1}/{total}</span>
            <button type="button" className="btn btn-ghost btn-sm" onClick={onNext} disabled={index >= total - 1}>
              <ChevronRight style={{ width: 14, height: 14 }} />
            </button>
          </div>
        )}
      </TagPanelHeader>
      <div className="tag-preview-body">
        {path ? (
          <ThumbImage path={path} maxEdge={1024} alt={filename} draggable={false} onClick={() => setZoomed(true)}
            style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain', cursor: 'zoom-in' }} />
        ) : (
          <div className="tag-preview-empty">
            <ImageIcon style={{ width: 56, height: 56, opacity: 0.2 }} />
            <span>{total === 0 ? t('tagEditor.loadToShow') : t('tagEditor.selectToPreview')}</span>
          </div>
        )}
      </div>
      {zoomed && path && <ImageLightbox src={convertFileSrc(path)} filename={filename ?? ''} onClose={() => setZoomed(false)} />}
    </div>
  );
}
