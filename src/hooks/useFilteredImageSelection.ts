import { useState } from 'react';

export function useFilteredImageSelection(items: readonly { _i: number }[]) {
  const [preferredIndex, setSelectedIdx] = useState(-1);
  const found = items.findIndex(item => item._i === preferredIndex);
  const position = found < 0 && items.length ? 0 : found;
  const selectedIdx = items[position]?._i ?? -1;
  const goPrev = () => { if (position > 0) setSelectedIdx(items[position - 1]._i); };
  const goNext = () => { if (position + 1 < items.length) setSelectedIdx(items[position + 1]._i); };
  return { selectedIdx, setSelectedIdx, position, goPrev, goNext };
}
