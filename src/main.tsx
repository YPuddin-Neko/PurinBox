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

    // 检测剪贴板是否有文本
    let clipboardHasText = false;
    try {
      const text = await navigator.clipboard.readText();
      clipboardHasText = text.length > 0;
    } catch { clipboardHasText = false; }

    menu = document.createElement('div');
    menu.style.cssText = `position:fixed;left:${x}px;top:${y}px;z-index:99999;min-width:120px;padding:4px;background:var(--color-bg-secondary);border:1px solid var(--color-border);border-radius:8px;box-shadow:0 8px 24px rgba(0,0,0,0.25);`;

    menu.appendChild(createMenuItem('全选', () => { target.focus(); target.select(); }));
    menu.appendChild(createMenuItem('复制', () => document.execCommand('copy')));
    menu.appendChild(createMenuItem('粘贴', async () => {
      target.focus();
      const t = await navigator.clipboard.readText();
      document.execCommand('insertText', false, t);
    }, !clipboardHasText));

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

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
