import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';

// 自定义右键菜单
(() => {
  let menu: HTMLDivElement | null = null;

  const closeMenu = () => { if (menu) { menu.remove(); menu = null; } };

  const createMenuItem = (label: string, action: () => void, disabled = false) => {
    const btn = document.createElement('button');
    btn.textContent = label;
    btn.disabled = disabled;
    btn.style.cssText = `display:block;width:100%;padding:6px 12px;border:none;background:none;font-size:12px;text-align:left;border-radius:4px;font-family:inherit;cursor:${disabled ? 'default' : 'pointer'};color:${disabled ? 'var(--color-text-tertiary)' : 'var(--color-text-secondary)'};opacity:${disabled ? '0.5' : '1'};`;
    if (!disabled) {
      btn.onmouseenter = () => { btn.style.background = 'var(--color-bg-hover)'; btn.style.color = 'var(--color-text-primary)'; };
      btn.onmouseleave = () => { btn.style.background = 'none'; btn.style.color = 'var(--color-text-secondary)'; };
      btn.onclick = () => { action(); closeMenu(); };
    }
    return btn;
  };

  const createMenu = async (x: number, y: number, target: HTMLInputElement | HTMLTextAreaElement) => {
    closeMenu();

    // 用 Tauri 原生插件读取剪贴板（不会弹浏览器权限窗）
    const { readText, writeText } = await import('@tauri-apps/plugin-clipboard-manager');
    let clipboardText = '';
    try { clipboardText = await readText() ?? ''; } catch {}

    const hasSelection = (target.selectionStart ?? 0) !== (target.selectionEnd ?? 0);

    menu = document.createElement('div');
    menu.style.cssText = `position:fixed;left:${x}px;top:${y}px;z-index:99999;min-width:120px;padding:4px;background:var(--color-bg-secondary);border:1px solid var(--color-border);border-radius:8px;box-shadow:0 8px 24px rgba(0,0,0,0.25);`;

    menu.appendChild(createMenuItem('全选', () => { target.focus(); target.select(); }));
    menu.appendChild(createMenuItem('复制', async () => {
      const sel = target.value.substring(target.selectionStart ?? 0, target.selectionEnd ?? 0);
      if (sel) await writeText(sel);
    }, !hasSelection));
    menu.appendChild(createMenuItem('粘贴', async () => {
      try {
        const t = await readText();
        if (t) {
          target.focus();
          document.execCommand('insertText', false, t);
        }
      } catch {}
    }, !clipboardText));

    document.body.appendChild(menu);

    requestAnimationFrame(() => {
      if (!menu) return;
      const r = menu.getBoundingClientRect();
      if (r.right > window.innerWidth) menu.style.left = `${window.innerWidth - r.width - 8}px`;
      if (r.bottom > window.innerHeight) menu.style.top = `${window.innerHeight - r.height - 8}px`;
    });
  };

  document.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const t = e.target as HTMLElement;
    if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) {
      createMenu(e.clientX, e.clientY, t);
    } else {
      closeMenu();
    }
  });

  document.addEventListener('mousedown', (e) => {
    if (menu && !menu.contains(e.target as Node)) closeMenu();
  });
  document.addEventListener('keydown', () => closeMenu());
  window.addEventListener('blur', () => closeMenu());
})();

// 禁止拖放文件到 WebView（Windows 上会导致页面导航）
document.addEventListener('dragover', (e) => e.preventDefault());
document.addEventListener('drop', (e) => e.preventDefault());

// 禁止 Ctrl+滚轮 / Ctrl+加减号 缩放页面
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && (e.key === '+' || e.key === '-' || e.key === '=' || e.key === '0')) {
    e.preventDefault();
  }
});
document.addEventListener('wheel', (e) => {
  if (e.ctrlKey || e.metaKey) e.preventDefault();
}, { passive: false });

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
