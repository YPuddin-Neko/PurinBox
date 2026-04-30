import { useState, useRef, useCallback } from 'react';
import { FileType, Upload, Check } from 'lucide-react';

interface ImageItem { id: string; name: string; url: string; }

const formats = ['PNG', 'JPG', 'WebP', 'BMP', 'TIFF'];

export default function ConvertPage() {
  const [images, setImages] = useState<ImageItem[]>([]);
  const [targetFormat, setTargetFormat] = useState('PNG');
  const [quality, setQuality] = useState(90);
  const [dragActive, setDragActive] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFiles = useCallback((files: FileList | null) => {
    if (!files) return;
    const newImages: ImageItem[] = [];
    Array.from(files).forEach((file) => {
      if (file.type.startsWith('image/')) {
        newImages.push({ id: crypto.randomUUID(), name: file.name, url: URL.createObjectURL(file) });
      }
    });
    setImages((prev) => [...prev, ...newImages]);
  }, []);

  const removeImage = (id: string) => {
    setImages((prev) => prev.filter((img) => img.id !== id));
  };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <FileType style={{ width: 28, height: 28, color: '#ffa647' }} />
          <h1 className="page-title">格式转换</h1>
        </div>
        <p className="page-subtitle">在多种图像格式间自由转换，支持批量处理</p>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 320px', gap: 'var(--space-6)' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* Dropzone */}
          <div className={`dropzone ${dragActive ? 'active' : ''}`}
            onDragOver={(e) => { e.preventDefault(); setDragActive(true); }}
            onDragLeave={() => setDragActive(false)}
            onDrop={(e) => { e.preventDefault(); setDragActive(false); handleFiles(e.dataTransfer.files); }}
            onClick={() => fileInputRef.current?.click()}
            style={{ padding: images.length > 0 ? 'var(--space-6)' : undefined }}>
            <div className="dropzone-icon"><Upload /></div>
            <div className="dropzone-title">拖拽图像或点击上传</div>
            <div className="dropzone-subtitle">支持多种图像格式</div>
            <input ref={fileInputRef} type="file" multiple accept="image/*" style={{ display: 'none' }} onChange={(e) => handleFiles(e.target.files)} />
          </div>

          {/* File List */}
          {images.length > 0 && (
            <div className="tool-panel">
              <div className="tool-panel-header">
                <span className="tool-panel-title">文件列表 ({images.length})</span>
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
                {images.map((img) => (
                  <div key={img.id} style={{
                    display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
                    padding: 'var(--space-2) var(--space-3)',
                    borderRadius: 'var(--radius-sm)', background: 'var(--color-bg-input)',
                    border: '1px solid var(--color-border)',
                  }}>
                    <img src={img.url} alt={img.name} style={{ width: 36, height: 36, borderRadius: 'var(--radius-sm)', objectFit: 'cover' }} />
                    <span style={{ flex: 1, fontSize: 'var(--font-size-sm)', color: 'var(--color-text-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{img.name}</span>
                    <span style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>→ .{targetFormat.toLowerCase()}</span>
                    <button className="btn btn-ghost btn-sm" onClick={() => removeImage(img.id)} style={{ color: 'var(--color-error)' }}>×</button>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Settings */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">目标格式</span></div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 'var(--space-2)' }}>
              {formats.map((f) => (
                <button key={f} className={`btn ${targetFormat === f ? 'btn-primary' : 'btn-secondary'} btn-sm`}
                  onClick={() => setTargetFormat(f)} style={{ justifyContent: 'center' }}>{f}</button>
              ))}
            </div>
          </div>

          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">输出质量</span></div>
            <div className="form-group">
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <label className="form-label">质量</label>
                <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-accent)' }}>{quality}%</span>
              </div>
              <input type="range" min="1" max="100" value={quality} onChange={(e) => setQuality(Number(e.target.value))}
                style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
          </div>

          <button className="btn btn-primary btn-lg" style={{ width: '100%' }} disabled={images.length === 0}>
            <Check /> 开始转换 ({images.length})
          </button>
        </div>
      </div>
    </div>
  );
}
