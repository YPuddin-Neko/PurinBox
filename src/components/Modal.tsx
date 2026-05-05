import { useEffect, useRef, ReactNode } from 'react';
import { AlertTriangle, Info, X } from 'lucide-react';

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  children: ReactNode;
  variant?: 'info' | 'warning' | 'error';
}

export function Modal({ open, onClose, title, children, variant = 'info' }: ModalProps) {
  const overlayRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, onClose]);

  if (!open) return null;

  const iconColor = variant === 'error' ? '#f87171' : variant === 'warning' ? '#fbbf24' : '#60a5fa';

  return (
    <div ref={overlayRef} onClick={e => { if (e.target === overlayRef.current) onClose(); }}
      style={{
        position: 'fixed', inset: 0, zIndex: 99998,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
        animation: 'fadeIn 0.15s ease',
      }}>
      <div style={{
        background: 'var(--color-bg-secondary)', border: '1px solid var(--color-border)',
        borderRadius: 12, padding: '20px 24px', minWidth: 320, maxWidth: 480,
        boxShadow: '0 16px 48px rgba(0,0,0,0.3)', animation: 'slideUp 0.2s ease',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {variant === 'error' || variant === 'warning'
              ? <AlertTriangle style={{ width: 18, height: 18, color: iconColor }} />
              : <Info style={{ width: 18, height: 18, color: iconColor }} />}
            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--color-text-primary)' }}>{title || '提示'}</span>
          </div>
          <button onClick={onClose} style={{
            background: 'none', border: 'none', cursor: 'pointer', padding: 4, borderRadius: 6,
            color: 'var(--color-text-tertiary)', display: 'flex',
          }}><X style={{ width: 16, height: 16 }} /></button>
        </div>
        <div style={{ fontSize: 13, lineHeight: 1.6, color: 'var(--color-text-secondary)', whiteSpace: 'pre-wrap' }}>
          {children}
        </div>
      </div>
    </div>
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

export function ConfirmModal({ open, onClose, onConfirm, title, message, confirmText = '确定', cancelText = '取消', variant = 'warning' }: ConfirmModalProps) {
  return (
    <Modal open={open} onClose={onClose} title={title || '确认'} variant={variant}>
      <div>{message}</div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 16 }}>
        <button className="btn btn-secondary btn-sm" onClick={onClose}>{cancelText}</button>
        <button className="btn btn-primary btn-sm" onClick={() => { onConfirm(); onClose(); }}
          style={variant === 'error' ? { background: 'rgba(248,113,113,0.15)', color: '#f87171', borderColor: 'rgba(248,113,113,0.3)' } : undefined}>
          {confirmText}
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
  return (
    <Modal open={open} onClose={onClose} title={title || '提示'} variant={variant}>
      <div>{message}</div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 16 }}>
        <button className="btn btn-primary btn-sm" onClick={onClose}>确定</button>
      </div>
    </Modal>
  );
}
