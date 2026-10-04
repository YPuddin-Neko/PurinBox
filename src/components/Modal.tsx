import { useEffect, ReactNode, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, Info, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  children: ReactNode;
  variant?: 'info' | 'warning' | 'error';
  /** 宽度上限（px），默认 480；窗口较窄时不超过窗口宽度减 32px */
  maxWidth?: number;
  /** 固定宽度，如 440、'min(80vw, 960px)'；不传则按内容收缩，最小 320 */
  width?: number | string;
  /**
   * 分段式：卡片底色、无外边距，标题栏与内容区之间有分隔线，内容区默认留白 16px 20px。
   * 内容底部的操作栏可用负外边距贴到两侧边缘（见 tags.css 的 .tag-batch-actions）
   */
  sectioned?: boolean;
  headerExtra?: ReactNode;
  bodyStyle?: CSSProperties;
  /** 合并进对话框外框的样式，如 { borderRadius: 12, maxHeight: '85vh' } */
  dialogStyle?: CSSProperties;
  /** 合并进标题文字的样式，如 { fontSize: 14, fontWeight: 600 } */
  titleStyle?: CSSProperties;
  className?: string;
  /** 标题前的图标；默认按 variant 显示，传 false 不显示 */
  headerIcon?: ReactNode;
}

export function Modal({
  open, onClose, title, children, variant = 'info', maxWidth = 480, width, sectioned = false,
  headerExtra, bodyStyle, dialogStyle, titleStyle, className, headerIcon,
}: ModalProps) {
  const { t } = useTranslation();

  useEffect(() => {
    if (!open) return;
    // 弹窗里的控件（如补全下拉框）先处理 Esc 并 preventDefault 时不关闭；输入法组字中的 Esc 也不关闭
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented && !e.isComposing) onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, onClose]);

  if (!open) return null;

  const iconColor = variant === 'error' ? '#f87171' : variant === 'warning' ? '#fbbf24' : '#60a5fa';

  return createPortal(
    <div onClick={e => { if (e.target === e.currentTarget) onClose(); }}
      style={{
        position: 'fixed', inset: 0, zIndex: 99998,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
        animation: 'fadeIn 0.15s ease',
      }}>
      <div role="dialog" className={className} aria-modal="true" aria-label={title || t('modal.hint')} style={{
        background: sectioned ? 'var(--color-bg-card)' : 'var(--color-bg-secondary)', border: '1px solid var(--color-border)',
        borderRadius: sectioned ? 16 : 12, padding: sectioned ? 0 : '20px 24px',
        width, minWidth: 320, maxWidth: `min(${maxWidth}px, calc(100vw - 32px))`,
        maxHeight: '90vh', display: 'flex', flexDirection: 'column', overflow: 'hidden',
        boxShadow: '0 16px 48px rgba(0,0,0,0.3)', animation: 'slideUp 0.2s ease',
        ...dialogStyle,
      }}>
        <div style={sectioned
          ? { display: 'flex', alignItems: 'center', gap: 10, padding: '16px 20px', borderBottom: '1px solid var(--color-border)' }
          : { display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0, ...(sectioned ? { flex: 1 } : null) }}>
            {headerIcon ?? (variant === 'error' || variant === 'warning'
              ? <AlertTriangle style={{ width: 18, height: 18, color: iconColor }} />
              : <Info style={{ width: 18, height: 18, color: iconColor }} />)}
            <span style={{
              fontSize: sectioned ? 13 : 14, fontWeight: sectioned ? 700 : 600, color: 'var(--color-text-primary)', overflowWrap: 'anywhere',
              ...titleStyle,
            }}>{title || t('modal.hint')}</span>
          </div>
          {headerExtra}
          <button type="button" aria-label={t('common.close')} onClick={onClose} style={{
            background: 'none', border: 'none', cursor: 'pointer', padding: 4, borderRadius: 6,
            color: 'var(--color-text-tertiary)', display: 'flex',
          }}><X style={{ width: 16, height: 16 }} /></button>
        </div>
        <div style={{
          fontSize: 13, lineHeight: 1.6, color: 'var(--color-text-secondary)', whiteSpace: 'pre-wrap', overflow: 'auto', minHeight: 0,
          ...(sectioned ? { padding: '16px 20px' } : null), ...bodyStyle,
        }}>
          {children}
        </div>
      </div>
    </div>, document.body
  );
}

interface ConfirmModalProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title?: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  variant?: 'info' | 'warning' | 'error';
}

export function ConfirmModal({ open, onClose, onConfirm, title, message, confirmText, cancelText, variant = 'warning' }: ConfirmModalProps) {
  const { t } = useTranslation();
  const ct = confirmText || t('modal.confirm');
  const cct = cancelText || t('modal.cancel');
  return (
    <Modal open={open} onClose={onClose} title={title || t('modal.confirmTitle')} variant={variant}>
      <div>{message}</div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 16 }}>
        <button className="btn btn-secondary btn-sm" onClick={onClose}>{cct}</button>
        <button className={`btn btn-sm ${variant === 'error' ? 'btn-danger' : 'btn-primary'}`} onClick={() => { onConfirm(); onClose(); }}>
          {ct}
        </button>
      </div>
    </Modal>
  );
}

interface AlertModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  message: string;
  variant?: 'info' | 'warning' | 'error';
}

export function AlertModal({ open, onClose, title, message, variant = 'error' }: AlertModalProps) {
  const { t } = useTranslation();
  return (
    <Modal open={open} onClose={onClose} title={title || t('modal.hint')} variant={variant}>
      <div>{message}</div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 16 }}>
        <button className="btn btn-primary btn-sm" onClick={onClose}>{t('modal.confirm')}</button>
      </div>
    </Modal>
  );
}
