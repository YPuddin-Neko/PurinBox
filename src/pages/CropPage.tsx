import { useState, useRef, useCallback } from 'react';
import { Crop as CropIcon, Upload, Check, Plus } from 'lucide-react';

interface ImageItem {
  id: string;
  name: string;
  url: string;
}

const presetRatios = [
  { label: '自由', value: null },
  { label: '1:1', value: 1 },
  { label: '4:3', value: 4 / 3 },
  { label: '16:9', value: 16 / 9 },
  { label: '3:2', value: 3 / 2 },
  { label: '2:3', value: 2 / 3 },
  { label: '9:16', value: 9 / 16 },
];

export default function CropPage() {
  const [images, setImages] = useState<ImageItem[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [aspectRatio, setAspectRatio] = useState<number | null>(null);
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
          <CropIcon style={{ width: 28, height: 28, color: '#00d4ff' }} />
          <h1 className="page-title">图像裁切</h1>
        </div>
        <p className="page-subtitle">智能裁切训练图像，支持自定义比例和批量处理</p>
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
            <div className="tool-panel">
              <div className="tool-panel-header"><span className="tool-panel-title">裁切比例</span></div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 'var(--space-2)' }}>
                {presetRatios.map((r) => (
                  <button key={r.label} className={`btn ${aspectRatio === r.value ? 'btn-primary' : 'btn-secondary'} btn-sm`} onClick={() => setAspectRatio(r.value)} style={{ justifyContent: 'center' }}>{r.label}</button>
                ))}
              </div>
            </div>
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
            <button className="btn btn-primary btn-lg" style={{ width: '100%' }}><Check /> 应用裁切</button>
          </div>
        </div>
      )}
    </div>
  );
}
