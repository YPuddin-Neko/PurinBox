import { useEffect } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';

interface ImageLightboxProps {
  src: string;
  filename: string;
  onClose: () => void;
}

export default function ImageLightbox({ src, filename, onClose }: ImageLightboxProps) {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 10000,
        display: 'grid',
        gridTemplateRows: 'auto 1fr',
        gap: 14,
        padding: '18px 24px 24px',
        background: 'rgba(9, 12, 20, 0.72)',
        backdropFilter: 'blur(14px) saturate(1.12)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, minHeight: 38, pointerEvents: 'none' }}>
        <div
          style={{
            pointerEvents: 'auto',
            maxWidth: 'min(72vw, 760px)',
            padding: '7px 12px',
            borderRadius: 999,
            background: 'rgba(18, 22, 34, 0.62)',
            border: '1px solid rgba(255, 255, 255, 0.12)',
            boxShadow: '0 8px 28px rgba(0, 0, 0, 0.24)',
            color: '#fff',
            fontSize: 12,
            fontWeight: 600,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {filename}
        </div>
        <button
          className="btn btn-ghost btn-sm"
          aria-label="Close preview"
          onClick={event => {
            event.stopPropagation();
            onClose();
          }}
          style={{
            pointerEvents: 'auto',
            width: 34,
            height: 34,
            padding: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            borderRadius: 17,
            background: 'rgba(18, 22, 34, 0.62)',
            border: '1px solid rgba(255, 255, 255, 0.12)',
            boxShadow: '0 8px 28px rgba(0, 0, 0, 0.24)',
            color: '#fff',
          }}
        >
          <X style={{ width: 16, height: 16 }} />
        </button>
      </div>

      <div style={{ minHeight: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <div
          onClick={event => event.stopPropagation()}
          style={{
            maxWidth: 'calc(100vw - 56px)',
            maxHeight: 'calc(100vh - 96px)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <img
            src={src}
            alt={filename}
            draggable={false}
            style={{
              maxWidth: '100%',
              maxHeight: 'calc(100vh - 96px)',
              objectFit: 'contain',
              borderRadius: 10,
              background: 'rgba(18, 22, 34, 0.36)',
              border: '1px solid rgba(255, 255, 255, 0.14)',
              boxShadow: '0 24px 90px rgba(0, 0, 0, 0.48)',
            }}
          />
        </div>
      </div>
    </div>,
    document.body,
  );
}
