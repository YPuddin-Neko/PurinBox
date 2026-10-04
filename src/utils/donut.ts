function polar(cx: number, cy: number, radius: number, angle: number) {
  return [cx + radius * Math.cos(angle), cy + radius * Math.sin(angle)] as const;
}

export function arcPath(cx: number, cy: number, outer: number, inner: number, start: number, end: number): string {
  if (end <= start) return '';
  const point = (radius: number, angle: number) => polar(cx, cy, radius, angle).join(' ');
  // 整圆的起点与终点重合，单段弧画不出来，拆成两个半圆
  if (end - start >= Math.PI * 2 - 1e-6) {
    const opposite = start + Math.PI;
    return `M ${point(outer, start)} A ${outer} ${outer} 0 1 1 ${point(outer, opposite)} A ${outer} ${outer} 0 1 1 ${point(outer, start)} L ${point(inner, start)} A ${inner} ${inner} 0 1 0 ${point(inner, opposite)} A ${inner} ${inner} 0 1 0 ${point(inner, start)} Z`;
  }
  const large = end - start > Math.PI ? 1 : 0;
  return `M ${point(outer, start)} A ${outer} ${outer} 0 ${large} 1 ${point(outer, end)} L ${point(inner, end)} A ${inner} ${inner} 0 ${large} 0 ${point(inner, start)} Z`;
}
