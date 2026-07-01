import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { ChevronDown, FolderOpen, ImageIcon } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface Props {
  onSelect: (path: string) => void;
}

const imageExtensions = ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'tiff', 'tif', 'gif'];

export default function InputPathPickerButton({ onSelect }: Props) {
  const { t } = useTranslation();
  const [openMenu, setOpenMenu] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!openMenu) return;
    const onPointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpenMenu(false);
      }
    };
    window.addEventListener('pointerdown', onPointerDown);
    return () => window.removeEventListener('pointerdown', onPointerDown);
  }, [openMenu]);

  const selectFolder = async () => {
    setOpenMenu(false);
    const selected = await open({ directory: true, multiple: false, title: t('pages.selectInputTitle') });
    if (selected) onSelect(selected as string);
  };

  const selectImage = async () => {
    setOpenMenu(false);
    const selected = await open({
      multiple: false,
      title: t('pages.selectInputImageTitle'),
      filters: [{ name: t('pages.imageFiles'), extensions: imageExtensions }],
    });
    if (selected) onSelect(selected as string);
  };

  const itemStyle = {
    width: '100%',
    display: 'flex',
    alignItems: 'center',
    gap: 8,
    padding: '8px 10px',
    border: 'none',
    background: 'transparent',
    color: 'var(--color-text-secondary)',
    cursor: 'pointer',
    fontSize: 12,
    fontFamily: 'inherit',
    textAlign: 'left' as const,
    whiteSpace: 'nowrap' as const,
  };

  return (
    <div ref={ref} style={{ position: 'relative', flexShrink: 0 }}>
      <button
        className="btn btn-secondary"
        onClick={() => setOpenMenu(v => !v)}
        title={t('pages.selectInputPathTitle')}
        style={{ gap: 4 }}
      >
        <FolderOpen style={{ width: 16, height: 16 }} />
        <ChevronDown style={{ width: 12, height: 12 }} />
      </button>
      {openMenu && (
        <div style={{
          position: 'absolute',
          right: 0,
          top: 'calc(100% + 6px)',
          minWidth: 148,
          padding: 4,
          borderRadius: 'var(--radius-md)',
          border: '1px solid var(--color-border)',
          background: 'var(--color-bg-elevated)',
          boxShadow: 'var(--shadow-lg)',
          zIndex: 50,
        }}>
          <button type="button" style={itemStyle} onClick={selectFolder}>
            <FolderOpen style={{ width: 14, height: 14 }} />
            {t('pages.selectInputFolderOption')}
          </button>
          <button type="button" style={itemStyle} onClick={selectImage}>
            <ImageIcon style={{ width: 14, height: 14 }} />
            {t('pages.selectInputImageOption')}
          </button>
        </div>
      )}
    </div>
  );
}
