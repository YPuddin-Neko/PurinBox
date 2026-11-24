import { useEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent } from 'react';
import type { BboxRegion } from '../utils/bboxRegions';

type HandleDir = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

const HANDLES: readonly { dir: HandleDir; left: string; top: string; cursor: string }[] = [
  { dir: 'nw', left: '0%', top: '0%', cursor: 'nwse-resize' },
  { dir: 'n', left: '50%', top: '0%', cursor: 'ns-resize' },
  { dir: 'ne', left: '100%', top: '0%', cursor: 'nesw-resize' },
  { dir: 'e', left: '100%', top: '50%', cursor: 'ew-resize' },
  { dir: 'se', left: '100%', top: '100%', cursor: 'nwse-resize' },
  { dir: 's', left: '50%', top: '100%', cursor: 'ns-resize' },
  { dir: 'sw', left: '0%', top: '100%', cursor: 'nesw-resize' },
  { dir: 'w', left: '0%', top: '50%', cursor: 'ew-resize' },
];

const MIN_SPAN = 0.015;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
const round3 = (value: number) => Math.round(value * 1000) / 1000;

export interface BboxImageProps {
  src: string;
  alt: string;
  regions: BboxRegion[];
  onClick?: () => void;
  selectedRegion?: string;
  hoveredRegion?: string | null;
  focusSelected?: boolean;
  hideRegions?: boolean;
  onRegionSelect?: (id: string) => void;
  onRegionHover?: (id: string | null) => void;
  onRegionBoxChange?: (id: string, nextBox: [number, number, number, number]) => void;
  onNaturalSize?: (size: { width: number; height: number }) => void;
}

export default function BboxImage({
  src,
  alt,
  regions,
  onClick,
  selectedRegion,
  hoveredRegion,
  focusSelected = false,
  hideRegions = false,
  onRegionSelect,
  onRegionHover,
  onRegionBoxChange,
  onNaturalSize,
}: BboxImageProps) {
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [natural, setNatural] = useState({ key: '', width: 0, height: 0 });
  const [draggingId, setDraggingId] = useState<string | null>(null);

  const dragRef = useRef<{
    id: string;
    mode: 'move' | HandleDir;
    pointerId: number;
    startX: number;
    startY: number;
    startBox: [number, number, number, number];
    overlayW: number;
    overlayH: number;
  } | null>(null);

  const key = src;

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const sync = () => setSize({ width: element.clientWidth, height: element.clientHeight });
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const scale = natural.key === key && natural.width
    ? Math.min(size.width / natural.width, size.height / natural.height)
    : 0;
  const width = natural.width * scale;
  const height = natural.height * scale;

  // Smaller regions sit above larger enclosing regions so inner boxes stay easy to click
  const orderedRegions = useMemo(() => (
    [...regions].sort((a, b) => {
      const areaA = (a.box[2] - a.box[0]) * (a.box[3] - a.box[1]);
      const areaB = (b.box[2] - b.box[0]) * (b.box[3] - b.box[1]);
      return areaB - areaA;
    })
  ), [regions]);

  const hasSelectedBox = Boolean(selectedRegion && regions.some(r => r.id === selectedRegion));
  const dimming = Boolean(focusSelected && hasSelectedBox);

  const startDrag = (
    event: PointerEvent<HTMLElement>,
    region: BboxRegion,
    mode: 'move' | HandleDir,
  ) => {
    if (!onRegionBoxChange || event.button !== 0 || width <= 0 || height <= 0) return;
    event.preventDefault();
    event.stopPropagation();
    dragRef.current = {
      id: region.id,
      mode,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      startBox: [...region.box] as [number, number, number, number],
      overlayW: width,
      overlayH: height,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    setDraggingId(region.id);
  };

  const handlePointerMove = (event: PointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId || !onRegionBoxChange) return;
    event.stopPropagation();
    const dx = (event.clientX - drag.startX) / drag.overlayW;
    const dy = (event.clientY - drag.startY) / drag.overlayH;
    const [l, t, r, b] = drag.startBox;

    if (drag.mode === 'move') {
      const boxW = r - l;
      const boxH = b - t;
      const nl = clamp(l + dx, 0, 1 - boxW);
      const nt = clamp(t + dy, 0, 1 - boxH);
      onRegionBoxChange(drag.id, [round3(nl), round3(nt), round3(nl + boxW), round3(nt + boxH)]);
      return;
    }

    let nl = l;
    let nt = t;
    let nr = r;
    let nb = b;
    if (drag.mode.includes('w')) nl = clamp(l + dx, 0, r - MIN_SPAN);
    if (drag.mode.includes('e')) nr = clamp(r + dx, l + MIN_SPAN, 1);
    if (drag.mode.includes('n')) nt = clamp(t + dy, 0, b - MIN_SPAN);
    if (drag.mode.includes('s')) nb = clamp(b + dy, t + MIN_SPAN, 1);
    onRegionBoxChange(drag.id, [round3(nl), round3(nt), round3(nr), round3(nb)]);
  };

  const endDrag = (event: PointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    event.stopPropagation();
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setDraggingId(null);
  };

  return (
    <div ref={host} style={{ width: '100%', height: '100%', minHeight: 0, position: 'relative' }}>
      <img
        key={key}
        src={src}
        alt={alt}
        draggable={false}
        onClick={onClick}
        onLoad={(event) => {
          const nw = event.currentTarget.naturalWidth;
          const nh = event.currentTarget.naturalHeight;
          setNatural({ key, width: nw, height: nh });
          onNaturalSize?.({ width: nw, height: nh });
        }}
        style={{
          width: '100%',
          height: '100%',
          objectFit: 'contain',
          display: 'block',
          cursor: onClick ? 'zoom-in' : undefined,
        }}
      />
      {!hideRegions && width > 0 && height > 0 && (
        <div
          className="bbox-overlay"
          style={{
            width,
            height,
            left: (size.width - width) / 2,
            top: (size.height - height) / 2,
          }}
        >
          {orderedRegions.map((region, orderIdx) => {
            const [left, top, right, bottom] = region.box;
            const isSelected = selectedRegion === region.id;
            const isHovered = hoveredRegion === region.id;
            const isDimmed = dimming && !isSelected && !isHovered;
            const isEditable = Boolean(onRegionBoxChange && isSelected);
            const isDragging = draggingId === region.id;
            const interactive = Boolean(onRegionSelect || onRegionHover || isEditable);
            const labelAbove = top * height >= 26;

            return (
              <div
                key={region.id}
                className={[
                  'bbox-region',
                  isSelected ? 'is-selected' : '',
                  isHovered ? 'is-hovered' : '',
                  isDimmed ? 'is-dimmed' : '',
                  isDragging ? 'is-dragging' : '',
                ].filter(Boolean).join(' ')}
                title={region.label}
                onMouseEnter={onRegionHover ? () => onRegionHover(region.id) : undefined}
                onMouseLeave={onRegionHover ? () => onRegionHover(null) : undefined}
                onClick={onRegionSelect ? (event) => {
                  event.stopPropagation();
                  onRegionSelect(region.id);
                } : undefined}
                onPointerDown={isEditable ? (event) => startDrag(event, region, 'move') : undefined}
                onPointerMove={isEditable ? handlePointerMove : undefined}
                onPointerUp={isEditable ? endDrag : undefined}
                onPointerCancel={isEditable ? endDrag : undefined}
                style={{
                  pointerEvents: interactive ? 'auto' : undefined,
                  cursor: isEditable ? (isDragging ? 'grabbing' : 'move') : (onRegionSelect ? 'pointer' : undefined),
                  zIndex: isSelected ? 30 : isHovered ? 20 : orderIdx + 1,
                  background: isSelected
                    ? `color-mix(in srgb, ${region.color} 14%, transparent)`
                    : isHovered
                      ? `color-mix(in srgb, ${region.color} 9%, transparent)`
                      : undefined,
                  left: `${left * 100}%`,
                  top: `${top * 100}%`,
                  width: `${(right - left) * 100}%`,
                  height: `${(bottom - top) * 100}%`,
                  borderColor: region.color,
                  borderStyle: region.nested ? 'dashed' : 'solid',
                }}
              >
                <span
                  className="bbox-label"
                  style={{
                    borderColor: isSelected ? region.color : `color-mix(in srgb, ${region.color} 55%, transparent)`,
                    maxWidth: Math.max(48, width * (1 - left) - 4),
                    transform: labelAbove ? 'translateY(calc(-100% - 4px))' : 'translate(4px, 4px)',
                  }}
                >
                  <i className="bbox-label-dot" style={{ background: region.color }} />
                  <span className="bbox-label-text">{region.label}</span>
                </span>

                {isEditable && HANDLES.map(handle => (
                  <span
                    key={handle.dir}
                    className="bbox-handle"
                    onPointerDown={(event) => startDrag(event, region, handle.dir)}
                    onPointerMove={handlePointerMove}
                    onPointerUp={endDrag}
                    onPointerCancel={endDrag}
                    onClick={(event) => event.stopPropagation()}
                    style={{
                      left: handle.left,
                      top: handle.top,
                      cursor: handle.cursor,
                      '--handle-color': region.color,
                    } as CSSProperties}
                  />
                ))}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
