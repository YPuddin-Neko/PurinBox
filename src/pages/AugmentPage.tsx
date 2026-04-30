import { ImagePlus } from 'lucide-react';

const augmentOptions = [
  { id: 'hflip', label: '水平翻转', desc: '左右镜像翻转图像', active: false },
  { id: 'vflip', label: '垂直翻转', desc: '上下镜像翻转图像', active: false },
  { id: 'rotate', label: '随机旋转', desc: '在指定角度范围内随机旋转', active: false },
  { id: 'brightness', label: '亮度调整', desc: '随机调整图像亮度', active: false },
  { id: 'contrast', label: '对比度调整', desc: '随机调整图像对比度', active: false },
  { id: 'saturation', label: '饱和度调整', desc: '随机调整颜色饱和度', active: false },
  { id: 'blur', label: '高斯模糊', desc: '添加随机程度的模糊效果', active: false },
  { id: 'noise', label: '添加噪声', desc: '为图像添加随机噪声', active: false },
];

import { useState } from 'react';

export default function AugmentPage() {
  const [options, setOptions] = useState(augmentOptions);

  const toggleOption = (id: string) => {
    setOptions((prev) => prev.map((opt) => opt.id === id ? { ...opt, active: !opt.active } : opt));
  };

  const activeCount = options.filter((o) => o.active).length;

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <ImagePlus style={{ width: 28, height: 28, color: '#4ade80' }} />
          <h1 className="page-title">数据增强</h1>
        </div>
        <p className="page-subtitle">通过多种变换方式扩充训练数据集</p>
      </div>

      <div className="tool-panel" style={{ marginBottom: 'var(--space-5)' }}>
        <div className="tool-panel-header">
          <span className="tool-panel-title">增强选项</span>
          <span style={{ fontSize: 'var(--font-size-sm)', color: 'var(--color-text-accent)' }}>
            已选 {activeCount} 项
          </span>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(250px, 1fr))', gap: 'var(--space-3)' }}>
          {options.map((opt) => (
            <div key={opt.id} onClick={() => toggleOption(opt.id)} style={{
              display: 'flex', alignItems: 'center', gap: 'var(--space-3)',
              padding: 'var(--space-3) var(--space-4)',
              borderRadius: 'var(--radius-md)',
              background: opt.active ? 'rgba(124, 92, 252, 0.08)' : 'var(--color-bg-input)',
              border: `1px solid ${opt.active ? 'var(--color-border-active)' : 'var(--color-border)'}`,
              cursor: 'pointer', transition: 'all 0.2s',
            }}>
              <button className={`toggle ${opt.active ? 'active' : ''}`} onClick={(e) => { e.stopPropagation(); toggleOption(opt.id); }} />
              <div>
                <div style={{ fontSize: 'var(--font-size-base)', fontWeight: 600, color: 'var(--color-text-primary)' }}>{opt.label}</div>
                <div style={{ fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>{opt.desc}</div>
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="empty-state">
        <div className="empty-state-icon"><ImagePlus /></div>
        <div className="empty-state-title">选择增强选项后上传数据集</div>
        <div className="empty-state-desc">选择需要的增强方式，然后上传图像进行批量处理</div>
      </div>
    </div>
  );
}
