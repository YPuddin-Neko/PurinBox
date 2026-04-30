import { useState, useRef, useCallback } from 'react';
import {
  Tags,
  Upload,
  Save,
  Trash2,
  ChevronLeft,
  ChevronRight,
  Plus,
  X,
  Copy,
  FileText,
  FolderOpen,
} from 'lucide-react';

interface ImageItem {
  id: string;
  name: string;
  url: string;
  caption: string;
}

export default function LabelingPage() {
  const [images, setImages] = useState<ImageItem[]>([]);
  const [selectedIndex, setSelectedIndex] = useState<number>(-1);
  const [caption, setCaption] = useState('');
  const [dragActive, setDragActive] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const selectedImage = selectedIndex >= 0 ? images[selectedIndex] : null;

  const handleFiles = useCallback((files: FileList | null) => {
    if (!files) return;
    const newImages: ImageItem[] = [];
    Array.from(files).forEach((file) => {
      if (file.type.startsWith('image/')) {
        const url = URL.createObjectURL(file);
        newImages.push({
          id: crypto.randomUUID(),
          name: file.name,
          url,
          caption: '',
        });
      }
    });
    setImages((prev) => {
      const updated = [...prev, ...newImages];
      if (prev.length === 0 && newImages.length > 0) {
        setSelectedIndex(0);
        setCaption(newImages[0].caption);
      }
      return updated;
    });
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragActive(false);
      handleFiles(e.dataTransfer.files);
    },
    [handleFiles]
  );

  const selectImage = (index: number) => {
    // Save current caption before switching
    if (selectedIndex >= 0) {
      setImages((prev) =>
        prev.map((img, i) =>
          i === selectedIndex ? { ...img, caption } : img
        )
      );
    }
    setSelectedIndex(index);
    setCaption(images[index]?.caption || '');
  };

  const handlePrev = () => {
    if (selectedIndex > 0) selectImage(selectedIndex - 1);
  };

  const handleNext = () => {
    if (selectedIndex < images.length - 1) selectImage(selectedIndex + 1);
  };

  const handleCaptionChange = (value: string) => {
    setCaption(value);
    if (selectedIndex >= 0) {
      setImages((prev) =>
        prev.map((img, i) =>
          i === selectedIndex ? { ...img, caption: value } : img
        )
      );
    }
  };

  const handleDeleteCurrent = () => {
    if (selectedIndex < 0) return;
    setImages((prev) => prev.filter((_, i) => i !== selectedIndex));
    if (selectedIndex >= images.length - 1) {
      setSelectedIndex(Math.max(0, selectedIndex - 1));
    }
    if (images.length <= 1) {
      setSelectedIndex(-1);
      setCaption('');
    }
  };

  const labeledCount = images.filter((img) => img.caption.trim().length > 0).length;
  const totalCount = images.length;
  const progress = totalCount > 0 ? (labeledCount / totalCount) * 100 : 0;

  // Quick tags
  const quickTags = ['1girl', '1boy', 'solo', 'simple background', 'white background', 'full body', 'upper body', 'looking at viewer', 'smile', 'outdoors', 'indoors'];

  const addQuickTag = (tag: string) => {
    const newCaption = caption ? `${caption}, ${tag}` : tag;
    handleCaptionChange(newCaption);
    textareaRef.current?.focus();
  };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '4px' }}>
          <Tags style={{ width: 28, height: 28, color: '#7c5cfc' }} />
          <h1 className="page-title">数据集打标</h1>
        </div>
        <p className="page-subtitle">为训练图像添加描述标签，支持批量编辑</p>
      </div>

      {/* Progress */}
      {totalCount > 0 && (
        <div style={{ marginBottom: 'var(--space-5)' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 'var(--space-2)' }}>
            <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }}>
              标注进度
            </span>
            <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-accent)' }}>
              {labeledCount}/{totalCount} ({Math.round(progress)}%)
            </span>
          </div>
          <div className="progress-bar">
            <div className="progress-fill" style={{ width: `${progress}%` }} />
          </div>
        </div>
      )}

      {images.length === 0 ? (
        /* Dropzone */
        <div
          className={`dropzone ${dragActive ? 'active' : ''}`}
          onDragOver={(e) => { e.preventDefault(); setDragActive(true); }}
          onDragLeave={() => setDragActive(false)}
          onDrop={handleDrop}
          onClick={() => fileInputRef.current?.click()}
        >
          <div className="dropzone-icon">
            <Upload />
          </div>
          <div className="dropzone-title">拖拽图像到此处或点击上传</div>
          <div className="dropzone-subtitle">支持 PNG、JPG、WebP 格式</div>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept="image/*"
            style={{ display: 'none' }}
            onChange={(e) => handleFiles(e.target.files)}
          />
        </div>
      ) : (
        /* Tool Layout */
        <div className="tool-page-layout">
          <div className="tool-main">
            {/* Image Preview */}
            <div className="tool-panel" style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
              <div className="tool-panel-header">
                <span className="tool-panel-title">
                  {selectedImage ? selectedImage.name : '未选择'}
                </span>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <button className="btn btn-ghost btn-sm" onClick={handlePrev} disabled={selectedIndex <= 0}>
                    <ChevronLeft />
                  </button>
                  <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-tertiary)', display: 'flex', alignItems: 'center' }}>
                    {selectedIndex + 1} / {images.length}
                  </span>
                  <button className="btn btn-ghost btn-sm" onClick={handleNext} disabled={selectedIndex >= images.length - 1}>
                    <ChevronRight />
                  </button>
                </div>
              </div>
              <div style={{
                flex: 1,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                borderRadius: 'var(--radius-md)',
                background: 'var(--color-bg-input)',
                overflow: 'hidden',
                minHeight: '300px',
              }}>
                {selectedImage ? (
                  <img
                    src={selectedImage.url}
                    alt={selectedImage.name}
                    style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain' }}
                  />
                ) : (
                  <div className="empty-state" style={{ padding: 'var(--space-8)' }}>
                    <FileText style={{ width: 32, height: 32, color: 'var(--color-text-tertiary)' }} />
                    <p style={{ color: 'var(--color-text-tertiary)', fontSize: 'var(--font-size-sm)' }}>选择一张图片开始标注</p>
                  </div>
                )}
              </div>
            </div>

            {/* Caption Editor */}
            <div className="tool-panel">
              <div className="tool-panel-header">
                <span className="tool-panel-title">标签描述</span>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <button className="btn btn-ghost btn-sm" onClick={() => navigator.clipboard.writeText(caption)} title="复制">
                    <Copy />
                  </button>
                  <button className="btn btn-danger btn-sm" onClick={handleDeleteCurrent} title="删除当前图片">
                    <Trash2 />
                  </button>
                </div>
              </div>
              <textarea
                ref={textareaRef}
                className="form-input"
                value={caption}
                onChange={(e) => handleCaptionChange(e.target.value)}
                placeholder="输入图像描述标签，多个标签用逗号分隔..."
                style={{ minHeight: '100px', resize: 'vertical' }}
              />
              {/* Quick Tags */}
              <div style={{ marginTop: 'var(--space-3)' }}>
                <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', marginBottom: 'var(--space-2)' }}>
                  快速标签
                </div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--space-1)' }}>
                  {quickTags.map((tag) => (
                    <button
                      key={tag}
                      className="tool-card-tag"
                      style={{ cursor: 'pointer', transition: 'all 0.15s' }}
                      onClick={() => addQuickTag(tag)}
                      onMouseEnter={(e) => {
                        (e.target as HTMLElement).style.borderColor = 'var(--color-accent-primary)';
                        (e.target as HTMLElement).style.color = 'var(--color-accent-primary)';
                      }}
                      onMouseLeave={(e) => {
                        (e.target as HTMLElement).style.borderColor = '';
                        (e.target as HTMLElement).style.color = '';
                      }}
                    >
                      <Plus style={{ width: 10, height: 10, display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />
                      {tag}
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>

          {/* Sidebar - Image List */}
          <div className="tool-sidebar">
            <div className="tool-panel" style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
              <div className="tool-panel-header">
                <span className="tool-panel-title">图片列表 ({images.length})</span>
                <button className="btn btn-ghost btn-sm" onClick={() => fileInputRef.current?.click()}>
                  <Plus />
                </button>
              </div>
              <div className="image-grid" style={{ overflowY: 'auto', flex: 1 }}>
                {images.map((img, i) => (
                  <div
                    key={img.id}
                    className={`image-thumb ${i === selectedIndex ? 'image-thumb-selected' : ''}`}
                    onClick={() => selectImage(i)}
                  >
                    <img src={img.url} alt={img.name} />
                    {img.caption.trim() && (
                      <div className="image-thumb-check">
                        <Tags style={{ width: 10, height: 10 }} />
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              accept="image/*"
              style={{ display: 'none' }}
              onChange={(e) => handleFiles(e.target.files)}
            />
          </div>
        </div>
      )}
    </div>
  );
}
