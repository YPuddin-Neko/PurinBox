const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const;

interface FormatBytesOptions {
  /** 内存和显存：只用 MB 和 GB，MB 取整、GB 恒保留一位小数（"512 MB"、"15.8 GB"、"128.0 GB"） */
  memory?: boolean;
  /** 顶栏的紧凑写法：数字后直接跟单位首字母（"15.8G"、"512M"） */
  compact?: boolean;
}

/**
 * 字节数 → 可读大小（1024 进制）。非有限值和负数按 0 处理。
 * 默认写法：字节数取整；其余单位不足 100 时保留一位小数，否则取整（"1.5 KB"、"45.3 MB"、"512 MB"、"15.8 GB"）。
 */
export function formatBytes(bytes: number, { memory = false, compact = false }: FormatBytesOptions = {}): string {
  const first = memory ? UNITS.indexOf('MB') : 0;
  const last = memory ? UNITS.indexOf('GB') : UNITS.length - 1;
  const digitsFor = (unit: number, value: number) => (memory
    ? (unit === last ? 1 : 0)
    : (unit === 0 || Number(value.toFixed(1)) >= 100 ? 0 : 1));

  let value = (Number.isFinite(bytes) && bytes > 0 ? bytes : 0) / 1024 ** first;
  let unit = first;
  while (value >= 1024 && unit < last) {
    value /= 1024;
    unit += 1;
  }
  let digits = digitsFor(unit, value);
  // 四舍五入后可能变成 1024：按上一级单位显示
  if (Number(value.toFixed(digits)) >= 1024 && unit < last) {
    value /= 1024;
    unit += 1;
    digits = digitsFor(unit, value);
  }
  return compact ? `${value.toFixed(digits)}${UNITS[unit][0]}` : `${value.toFixed(digits)} ${UNITS[unit]}`;
}
