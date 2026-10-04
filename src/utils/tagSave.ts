import type { SaveAllResult, SaveFailure } from '../api/commandOptions';

/** 返回值中的失败项；返回值不是 SaveAllResult 时没有可识别的失败项 */
export function saveFailures(result: unknown): SaveFailure[] {
  const failed = (result as Partial<SaveAllResult> | null)?.failed;
  if (!Array.isArray(failed)) return [];
  return failed
    .filter(item => typeof item?.path === 'string')
    .map(item => ({ path: item.path, error: String(item.error ?? '') }));
}

/**
 * 保存返回后清除未保存标记：只清发出去、没有失败、且发出后没再改过的条目。
 * sent 记每个条目发送时的内容（tags 数组、data 对象的引用或 caption 文本），content 取条目当前的内容。
 */
export function settleSaved<T extends { path: string; dirty?: boolean }>(
  items: T[],
  sent: ReadonlyMap<string, unknown>,
  failures: readonly SaveFailure[],
  content: (item: T) => unknown,
): T[] {
  const failed = new Set(failures.map(item => item.path));
  let changed = false;
  const next = items.map(item => {
    if (!item.dirty || !sent.has(item.path) || failed.has(item.path) || content(item) !== sent.get(item.path)) return item;
    changed = true;
    return { ...item, dirty: false };
  });
  return changed ? next : items;
}

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/** 批量保存有失败项时的提示：失败数一行，其下逐行列出「文件名: 原因」，超过 limit 条时以省略号结尾 */
export function saveFailureAlert(
  failures: readonly SaveFailure[],
  t: (key: string, options: { n: number }) => string,
  limit = 10,
): string {
  const lines = failures.slice(0, limit).map(item => `${fileName(item.path)}: ${item.error}`);
  if (failures.length > limit) lines.push('…');
  return [t('tagEditor.saveFailedCount', { n: failures.length }), ...lines].join('\n');
}
