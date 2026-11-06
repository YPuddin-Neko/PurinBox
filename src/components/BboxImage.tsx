import { useEffect, useRef, useState } from 'react';
import type { BboxRegion } from '../utils/bboxRegions';

export default function BboxImage({ src, alt, regions, onClick }: {
  src: string; alt: string; regions: BboxRegion[]; onClick?: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [natural, setNatural] = useState({ key: '', width: 0, height: 0 });
  const key = src;
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setSize({ width: element.clientWidth, height: element.clientHeight }));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const scale = natural.key === key && natural.width ? Math.min(size.width / natural.width, size.height / natural.height) : 0;
  const width = natural.width * scale, height = natural.height * scale;
  const props = {
    alt, draggable: false, onClick,
    onLoad: (event: React.SyntheticEvent<HTMLImageElement>) => setNatural({ key, width: event.currentTarget.naturalWidth, height: event.currentTarget.naturalHeight }),
    style: { width: '100%', height: '100%', objectFit: 'contain' as const, display: 'block', cursor: onClick ? 'zoom-in' : undefined },
  };
  return <div ref={host} style={{ width: '100%', height: '100%', minHeight: 0, position: 'relative' }}>
    <img key={key} src={src} {...props} />
    {width > 0 && height > 0 && <div className="bbox-overlay" style={{ width, height, left: (size.width - width) / 2, top: (size.height - height) / 2 }}>
      {regions.map(region => {
        const [left, top, right, bottom] = region.box;
        return <div key={region.id} className="bbox-region" title={region.label} style={{
          left: `${left * 100}%`, top: `${top * 100}%`, width: `${(right - left) * 100}%`, height: `${(bottom - top) * 100}%`,
          borderColor: region.color, borderStyle: region.nested ? 'dashed' : 'solid',
        }}>
          <span className="bbox-label" style={{ background: region.color, maxWidth: Math.max(24, width * (1 - left)), transform: top * height >= 22 ? 'translateY(-100%)' : undefined }}>{region.label}</span>
        </div>;
      })}
    </div>}
  </div>;
}
