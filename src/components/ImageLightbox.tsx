import { useCallback, useEffect, useRef, useState, type PointerEvent, type MouseEvent } from 'react';
import { createPortal } from 'react-dom';
import BboxImage from './BboxImage';
import type { BboxRegion } from '../utils/bboxRegions';
import { RotateCcw, X, ZoomIn, ZoomOut } from 'lucide-react';

interface ImageLightboxProps {
  src: string;
  filename: string;
  onClose: () => void;
  regions?: BboxRegion[];
}

const MIN_SCALE = 0.5;
const MAX_SCALE = 8;
const ZOOM_STEP = 1.25;

const glassPillStyle = {
  pointerEvents: 'auto' as const,
  background: 'rgba(18, 22, 34, 0.7)',
  border: '1px solid rgba(255, 255, 255, 0.14)',
  boxShadow: '0 8px 28px rgba(0, 0, 0, 0.28)',
  backdropFilter: 'blur(12px)',
  WebkitBackdropFilter: 'blur(12px)',
  color: '#fff',
};

const iconBtnStyle = {
  width: 30,
  height: 30,
  padding: 0,
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  borderRadius: 999,
  color: '#fff',
} as const;

export default function ImageLightbox({ src, filename, onClose, regions = [] }: ImageLightboxProps) {
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);

  const stageRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef({ scale: 1, x: 0, y: 0 });
  const suppressCloseRef = useRef(false);
  const dragRef = useRef({
    active: false,
    moved: false,
    pointerId: -1,
    startX: 0,
    startY: 0,
    originX: 0,
    originY: 0,
  });

  const resetView = useCallback(() => {
    viewRef.current = { scale: 1, x: 0, y: 0 };
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    resetView();
  }, [src, resetView]);

  const applyZoomAt = useCallback((nextScaleRaw: number, clientX?: number, clientY?: number) => {
    const prev = viewRef.current;
    const clamped = Math.min(MAX_SCALE, Math.max(MIN_SCALE, Math.round(nextScaleRaw * 1000) / 1000));
    const nextScale = Math.abs(clamped - 1) < 0.025 && prev.scale !== 1 ? 1 : clamped;
    if (Math.abs(nextScale - prev.scale) < 0.001) return;

    let nextX = 0;
    let nextY = 0;
    if (nextScale > 1) {
      const stage = stageRef.current;
      const ratio = nextScale / prev.scale;
      if (stage && clientX !== undefined && clientY !== undefined) {
        const rect = stage.getBoundingClientRect();
        const mx = clientX - (rect.left + rect.width / 2);
        const my = clientY - (rect.top + rect.height / 2);
        nextX = mx - (mx - prev.x) * ratio;
        nextY = my - (my - prev.y) * ratio;
      } else {
        nextX = prev.x * ratio;
        nextY = prev.y * ratio;
      }
    }

    viewRef.current = { scale: nextScale, x: nextX, y: nextY };
    setScale(nextScale);
    setOffset({ x: nextX, y: nextY });
  }, []);

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const sensitivity = event.ctrlKey ? 0.01 : 0.0022;
      const factor = Math.min(1.35, Math.max(0.74, Math.exp(-event.deltaY * sensitivity)));
      applyZoomAt(viewRef.current.scale * factor, event.clientX, event.clientY);
    };
    stage.addEventListener('wheel', onWheel, { passive: false });
    return () => stage.removeEventListener('wheel', onWheel);
  }, [applyZoomAt]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
      else if (event.key === '+' || event.key === '=') {
        event.preventDefault();
        applyZoomAt(viewRef.current.scale * ZOOM_STEP);
      } else if (event.key === '-' || event.key === '_') {
        event.preventDefault();
        applyZoomAt(viewRef.current.scale / ZOOM_STEP);
      } else if (event.key === '0') {
        event.preventDefault();
        resetView();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose, applyZoomAt, resetView]);

  const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || viewRef.current.scale <= 1) return;
    event.preventDefault();
    event.stopPropagation();
    dragRef.current = {
      active: true,
      moved: false,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: viewRef.current.x,
      originY: viewRef.current.y,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(true);
  };

  const handlePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag.active || drag.pointerId !== event.pointerId) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    if (!drag.moved && Math.hypot(dx, dy) > 3) drag.moved = true;
    const nextX = drag.originX + dx;
    const nextY = drag.originY + dy;
    viewRef.current = { ...viewRef.current, x: nextX, y: nextY };
    setOffset({ x: nextX, y: nextY });
  };

  const finishDrag = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag.active || drag.pointerId !== event.pointerId) return;
    if (drag.moved) {
      suppressCloseRef.current = true;
      window.setTimeout(() => {
        suppressCloseRef.current = false;
      }, 0);
    }
    drag.active = false;
    drag.pointerId = -1;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setDragging(false);
  };

  const handleDoubleClick = (event: MouseEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (viewRef.current.scale > 1.05) resetView();
    else applyZoomAt(2.5, event.clientX, event.clientY);
  };

  const handleBackdropClick = () => {
    if (suppressCloseRef.current) {
      suppressCloseRef.current = false;
      return;
    }
    onClose();
  };

  const isTransformed = Math.abs(scale - 1) > 0.005 || Math.abs(offset.x) > 0.5 || Math.abs(offset.y) > 0.5;
  const zoomPercent = Math.round(scale * 100);

  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      onClick={handleBackdropClick}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 10000,
        display: 'grid',
        gridTemplateRows: 'auto 1fr',
        gap: 12,
        padding: '18px 24px 24px',
        background: 'rgba(9, 12, 20, 0.76)',
        backdropFilter: 'blur(14px) saturate(1.12)',
        overflow: 'hidden',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, minHeight: 38, pointerEvents: 'none', position: 'relative', zIndex: 2 }}>
        <div
          style={{
            ...glassPillStyle,
            maxWidth: 'min(58vw, 680px)',
            padding: '7px 14px',
            borderRadius: 999,
            fontSize: 12,
            fontWeight: 600,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {filename}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 8, pointerEvents: 'auto' }} onClick={event => event.stopPropagation()}>
          <div
            style={{
              ...glassPillStyle,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 2,
              padding: 3,
              borderRadius: 999,
            }}
          >
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              aria-label="Zoom out"
              disabled={scale <= MIN_SCALE + 0.001}
              onClick={() => applyZoomAt(viewRef.current.scale / ZOOM_STEP)}
              style={iconBtnStyle}
            >
              <ZoomOut style={{ width: 15, height: 15 }} />
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={resetView}
              style={{
                height: 28,
                minWidth: 52,
                padding: '0 8px',
                borderRadius: 999,
                fontFamily: 'var(--font-mono)',
                fontSize: 11.5,
                fontWeight: 600,
                color: '#fff',
              }}
            >
              {zoomPercent}%
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              aria-label="Zoom in"
              disabled={scale >= MAX_SCALE - 0.001}
              onClick={() => applyZoomAt(viewRef.current.scale * ZOOM_STEP)}
              style={iconBtnStyle}
            >
              <ZoomIn style={{ width: 15, height: 15 }} />
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              aria-label="Reset zoom"
              disabled={!isTransformed}
              onClick={resetView}
              style={iconBtnStyle}
            >
              <RotateCcw style={{ width: 14, height: 14 }} />
            </button>
          </div>

          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="Close preview"
            onClick={onClose}
            style={{
              ...glassPillStyle,
              width: 36,
              height: 36,
              padding: 0,
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              borderRadius: 999,
            }}
          >
            <X style={{ width: 16, height: 16 }} />
          </button>
        </div>
      </div>

      <div
        ref={stageRef}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={finishDrag}
        onPointerCancel={finishDrag}
        style={{
          minHeight: 0,
          position: 'relative',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          overflow: 'hidden',
          touchAction: 'none',
          cursor: scale > 1 ? (dragging ? 'grabbing' : 'grab') : 'default',
        }}
      >
        <div
          onClick={event => event.stopPropagation()}
          onDoubleClick={handleDoubleClick}
          style={{
            width: regions.length ? '100%' : undefined,
            height: regions.length ? '100%' : undefined,
            maxWidth: 'calc(100vw - 56px)',
            maxHeight: 'calc(100vh - 96px)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            transform: `translate3d(${offset.x}px, ${offset.y}px, 0) scale(${scale})`,
            transformOrigin: 'center center',
            transition: dragging ? 'none' : 'transform 140ms var(--ease-out-expo)',
            willChange: 'transform',
            cursor: scale > 1 ? (dragging ? 'grabbing' : 'grab') : 'zoom-in',
          }}
        >
          {regions.length ? (
            <BboxImage src={src} alt={filename} regions={regions} />
          ) : (
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
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}
