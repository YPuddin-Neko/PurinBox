import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { ChevronDown, FolderOpen, ImageIcon } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import './ui.css';

/**
 * folder：按钮直接选文件夹；
 * folderOrImage：按钮弹出菜单，选文件夹或单张图片（后端支持单文件输入时用）
 */
export type PathPickMode = 'folder' | 'folderOrImage';

const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'tiff', 'tif', 'gif', 'psd'];

type MenuKeyAction = { focus: number } | { close: 'restoreFocus' | 'leave' } | null;

/**
 * 选择菜单里的按键：上下方向键在菜单项间移动（首尾相接）；Esc 关闭并把焦点还给触发按钮；
 * Tab 关闭菜单，焦点从触发按钮照常移到前后的控件。active 为获得焦点的菜单项序号，不在菜单项上时为 -1。
 */
export function menuKeyAction(key: string, active: number, count: number): MenuKeyAction {
  if (key === 'Escape') return { close: 'restoreFocus' };
  if (key === 'Tab') return { close: 'leave' };
  if ((key !== 'ArrowDown' && key !== 'ArrowUp') || count === 0) return null;
  const step = key === 'ArrowDown' ? 1 : -1;
  const from = active >= 0 ? active : step > 0 ? -1 : count;
  return { focus: (from + step + count) % count };
}

/** 路径输入框旁的选择按钮 */
function PathPickerButton({ onPick, mode, dialogTitle, size }: {
  onPick: (path: string) => void;
  mode: PathPickMode;
  /** 选文件夹时的对话框标题，也是 folder 模式按钮的无障碍名称 */
  dialogTitle: string;
  size: 'sm' | 'md';
}) {
  const { t } = useTranslation();
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const iconSize = size === 'sm' ? 14 : 16;

  useEffect(() => {
    if (!menuOpen) return;
    // 打开后焦点落在第一项，键盘用户才能接着用方向键选择
    menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    const onPointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setMenuOpen(false);
    };
    window.addEventListener('pointerdown', onPointerDown);
    return () => window.removeEventListener('pointerdown', onPointerDown);
  }, [menuOpen]);

  // 菜单项卸载时焦点会丢到页面上，收回到触发按钮
  const closeMenu = () => {
    setMenuOpen(false);
    triggerRef.current?.focus();
  };

  const onMenuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    const action = menuKeyAction(event.key, items.indexOf(document.activeElement as HTMLElement), items.length);
    if (!action) return;
    if ('focus' in action) {
      event.preventDefault();
      items[action.focus].focus();
      return;
    }
    // 外层弹窗见到 defaultPrevented 的 Esc 不会跟着关闭；Tab 不拦截，让浏览器从触发按钮移走焦点
    if (action.close === 'restoreFocus') event.preventDefault();
    closeMenu();
  };

  const pick = async (target: 'folder' | 'image') => {
    if (menuOpen) closeMenu();
    const selected = await open(
      target === 'folder' ? { directory: true, multiple: false, title: dialogTitle } : {
        multiple: false,
        title: t('pages.selectInputImageTitle'),
        filters: [{ name: t('pages.imageFiles'), extensions: IMAGE_EXTENSIONS }],
      },
    );
    if (typeof selected === 'string') onPick(selected);
  };

  if (mode === 'folder') {
    return (
      <button type="button" className="btn btn-secondary ui-path-btn" aria-label={dialogTitle} onClick={() => void pick('folder')}>
        <FolderOpen style={{ width: iconSize, height: iconSize }} />
      </button>
    );
  }

  return (
    <div ref={ref} className="ui-path-menu-anchor">
      <button ref={triggerRef} type="button" className="btn btn-secondary ui-path-btn"
        onClick={() => (menuOpen ? closeMenu() : setMenuOpen(true))} title={t('pages.selectInputPathTitle')}
        aria-haspopup="menu" aria-expanded={menuOpen} style={{ gap: 4 }}>
        <FolderOpen style={{ width: iconSize, height: iconSize }} />
        <ChevronDown style={{ width: 12, height: 12 }} />
      </button>
      {menuOpen && (
        <div ref={menuRef} className="ui-path-menu" role="menu" onKeyDown={onMenuKeyDown}>
          <button type="button" role="menuitem" tabIndex={-1} className="ui-path-menu-item" onClick={() => void pick('folder')}>
            <FolderOpen style={{ width: 14, height: 14 }} />
            {t('pages.selectInputFolderOption')}
          </button>
          <button type="button" role="menuitem" tabIndex={-1} className="ui-path-menu-item" onClick={() => void pick('image')}>
            <ImageIcon style={{ width: 14, height: 14 }} />
            {t('pages.selectInputImageOption')}
          </button>
        </div>
      )}
    </div>
  );
}

export interface PathInputProps {
  value: string;
  /** 输入和选择都走这里 */
  onChange: (path: string) => void;
  /** 默认 folder */
  pick?: PathPickMode;
  /** output：默认对话框标题和占位用输出文件夹的文案 */
  kind?: 'input' | 'output';
  /** 选文件夹的对话框标题，默认 pages.selectInputTitle / pages.selectOutputTitle */
  dialogTitle?: string;
  /** 默认按 pick 和 kind 取 pages.selectInputFolder / selectInputPath / selectOutputFolder */
  placeholder?: string;
  /** sm：32px 高、12px 字、14px 图标（导出栏等紧凑处） */
  size?: 'sm' | 'md';
  readOnly?: boolean;
  /** 给输入框，配合外部 <label htmlFor> */
  id?: string;
  'aria-label'?: string;
  /** 外层行的样式 */
  style?: CSSProperties;
}

/**
 * 路径文本框 + 选择按钮。
 *
 * 例：`<PathInput value={folder} onChange={setFolder} />`、
 * `<PathInput kind="output" size="sm" value={exportPath} onChange={setExportPath} />`
 */
export default function PathInput({
  value,
  onChange,
  pick = 'folder',
  kind = 'input',
  dialogTitle,
  placeholder,
  size = 'md',
  readOnly,
  id,
  'aria-label': ariaLabel,
  style,
}: PathInputProps) {
  const { t } = useTranslation();
  const output = kind === 'output';
  const title = dialogTitle ?? t(output ? 'pages.selectOutputTitle' : 'pages.selectInputTitle');
  const hint = placeholder ?? (
    output ? t('pages.selectOutputFolder')
      : pick === 'folderOrImage' ? t('pages.selectInputPath')
        : t('pages.selectInputFolder')
  );

  return (
    <div className={size === 'sm' ? 'ui-path-row is-sm' : 'ui-path-row'} style={style}>
      <input
        id={id}
        className="form-input"
        aria-label={ariaLabel}
        placeholder={hint}
        value={value}
        readOnly={readOnly}
        onChange={e => onChange(e.target.value)}
      />
      <PathPickerButton mode={pick} dialogTitle={title} size={size} onPick={onChange} />
    </div>
  );
}
