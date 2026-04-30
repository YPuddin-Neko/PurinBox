import { FolderSync } from 'lucide-react';

export default function OrganizePage() {
  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <FolderSync style={{ width: 28, height: 28, color: '#60a5fa' }} />
          <h1 className="page-title">数据集管理</h1>
        </div>
        <p className="page-subtitle">管理和组织训练数据集，支持分类和重命名</p>
      </div>

      <div className="empty-state">
        <div className="empty-state-icon"><FolderSync /></div>
        <div className="empty-state-title">尚未加载数据集</div>
        <div className="empty-state-desc">选择一个文件夹以开始管理训练数据集</div>
        <button className="btn btn-primary btn-lg" style={{ marginTop: 'var(--space-4)' }}>
          <FolderSync /> 选择文件夹
        </button>
      </div>
    </div>
  );
}
