import { useState, useRef, useCallback } from 'react';
import { Scaling, Upload, Check, Plus } from 'lucide-react';

interface ImageItem { id: string; name: string; url: string; }

const presetSizes = [
  { label: '512×512', w: 512, h: 512 },
  { label: '768×768', w: 768, h: 768 },
  { label: '1024×1024', w: 1024, h: 1024 },
  { label: '512×768', w: 512, h: 768 },
  { label: '768×1024', w: 768, h: 1024 },
  { label: '1024×1536', w: 1024, h: 1536 },
];

export default function ResizePage() {
  const [images, setImages] = useState<ImageItem[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [width, setWidth] = useState(512);
  const [height, setHeight] = useState(512);
  const [keepRatio, setKeepRatio] = useState(true);
  const [dragActive, setDragActive] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const selectedImage = selectedIndex >= 0 ? images[selectedIndex] : null;

  const handleFiles = useCallback((files: FileList | null) => {
    if (!files) return;
    const newImages: ImageItem[] = [];
    Array.from(files).forEach((file) => {
      if (file.type.startsWith('image/')) {
        newImages.push({ id: crypto.randomUUID(), name: file.name, url: URL.createObjectURL(file) });
      }
    });
    setImages((prev) => {
      if (prev.length === 0 && newImages.length > 0) setSelectedIndex(0);
      return [...prev, ...newImages];
    });
  }, []);

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Scaling style={{ width: 28, height: 28, color: '#ff6b9d' }} />
          <h1 className="page-title">尺寸调整</h1>
        </div>
        <p className="page-subtitle">批量调整图像尺寸，适配不同模型训练需求</p>
      </div>

      {images.length === 0 ? (
        <div className={`dropzone ${dragActive ? 'active' : ''}`}
          onDragOver={(e) => { e.preventDefault(); setDragActive(true); }}
          onDragLeave={() => setDragActive(false)}
          onDrop={(e) => { e.preventDefault(); setDragActive(false); handleFiles(e.dataTransfer.files); }}
          onClick={() => fileInputRef.current?.click()}>
          <div className="dropzone-icon"><Upload /></div>
          <div className="dropzone-title">拖拽图像到此处或点击上传</div>
          <div className="dropzone-subtitle">支持 PNG、JPG、WebP 格式</div>
          <input ref={fileInputRef} type="file" multiple accept="image/*" style={{ display: 'none' }} onChange={(e) => handleFiles(e.target.files)} />
        </div>
      ) : (
        <div className="tool-page-layout">
          <div className="tool-main">
            <div className="tool-panel" style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
              <div className="tool-panel-header">
                <span className="tool-panel-title">{selectedImage?.name || '未选择'}</span>
              </div>
              <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 'var(--radius-md)', background: 'var(--color-bg-input)', minHeight: 400 }}>
                {selectedImage && <img src={selectedImage.url} alt={selectedImage.name} style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain' }} />}
              </div>
            </div>
          </div>
          <div className="tool-sidebar">
            {/* Size Presets */}
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">预设尺寸</span></div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 'var(--space-2)' }}>
                {presetSizes.map((s) => (
                  <button key={s.label} className={`btn ${width === s.w && height === s.h ? 'btn-primary' : 'btn-secondary'} btn-sm`}
                    onClick={() => { setWidth(s.w); setHeight(s.h); }} style={{ justifyContent: 'center' }}>
                    {s.label}
                  </button>
                ))}
              </div>
            </div>
            {/* Custom Size */}
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">自定义尺寸</span></div>
              <div style={{ display: 'flex', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">宽度 (px)</label>
                  <input className="form-input" type="number" value={width} onChange={(e) => setWidth(Number(e.target.value))} />
                </div>
                <div className="form-group" style={{ flex: 1 }}>
                  <label className="form-label">高度 (px)</label>
                  <input className="form-input" type="number" value={height} onChange={(e) => setHeight(Number(e.target.value))} />
                </div>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                <button className={`toggle ${keepRatio ? 'active' : ''}`} onClick={() => setKeepRatio(!keepRatio)} />
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }}>保持宽高比</span>
              </div>
            </div>
            {/* Image List */}
            <div className="tool-panel" style={{ flex: 1 }}>
              <div className="tool-panel-header">
                <span className="tool-panel-title">图片 ({images.length})</span>
                <button className="btn btn-ghost btn-sm" onClick={() => fileInputRef.current?.click()}><Plus /></button>
              </div>
              <div className="image-grid" style={{ overflowY: 'auto' }}>
                {images.map((img, i) => (
                  <div key={img.id} className={`image-thumb ${i === selectedIndex ? 'image-thumb-selected' : ''}`} onClick={() => setSelectedIndex(i)}>
                    <img src={img.url} alt={img.name} />
                  </div>
                ))}
              </div>
              <input ref={fileInputRef} type="file" multiple accept="image/*" style={{ display: 'none' }} onChange={(e) => handleFiles(e.target.files)} />
            </div>
            <button className="btn btn-primary btn-lg" style={{ width: '100%' }}><Check /> 批量调整</button>
          </div>
        </div>
      )}
    </div>
  );
}
