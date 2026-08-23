import { invoke } from '@tauri-apps/api/core';

// 以"已发起的放行 Promise"为缓存而不是布尔标记：
// 并发同路径调用共享同一个 Promise，等它真正落地后才继续，
// 不会出现"第二个调用者在授权还没生效时就去 convertFileSrc 拿 403"的竞态
const pending = new Map<string, Promise<void>>();

/**
 * 运行时放行 asset 协议目录。
 *
 * assetProtocol 的 scope 已从 `**` 收窄为空（渲染进程不再默认可读全盘），
 * 任何要用 convertFileSrc 展示的用户目录（数据集、扫描输入等）都必须先经这里放行。
 *
 * 传目录直接放行整棵子树；传文件路径时带上 `{ file: true }`——
 * 会归并到父目录再放行/去重，同目录 N 张图只发一次 IPC（Rust 侧递归放行目录）。
 */
export function ensureAssetScope(path: string, opts?: { file?: boolean }): Promise<void> {
  if (!path) return Promise.resolve();

  let target = path;
  if (opts?.file) {
    const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    if (cut > 0) target = path.slice(0, cut);
  }

  const hit = pending.get(target);
  if (hit) return hit;

  const p = invoke('allow_asset_dir', { path: target })
    .then(() => undefined)
    .catch((e) => {
      pending.delete(target); // 失败可重试
      console.warn('放行 asset 目录失败:', target, e);
    });
  pending.set(target, p);
  return p;
}
