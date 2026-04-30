import { useLocation } from 'react-router-dom';
import { Bell, Search } from 'lucide-react';
import '../styles/layout.css';

const routeNames: Record<string, { breadcrumb: string; title: string }> = {
  '/': { breadcrumb: '工作台', title: '工作台' },
  '/scale': { breadcrumb: '图片缩放', title: '图片缩放' },
  '/flip': { breadcrumb: '图片处理', title: '图片处理' },
  '/filter': { breadcrumb: '分辨率筛选', title: '分辨率筛选' },
  '/file-keeper': { breadcrumb: '保留指定文件', title: '保留指定文件' },
  '/format-convert': { breadcrumb: '图片格式转换', title: '图片格式转换' },
  '/alpha-convert': { breadcrumb: '转换透明通道', title: '转换透明通道' },
  '/batch-rename': { breadcrumb: '批量重命名', title: '批量重命名' },
  '/tagger': { breadcrumb: '图片打标', title: '图片打标' },
  '/labeling': { breadcrumb: '数据集打标', title: '数据集打标' },
  '/crop': { breadcrumb: '图像裁切', title: '图像裁切' },
  '/resize': { breadcrumb: '尺寸调整', title: '尺寸调整' },
  '/convert': { breadcrumb: '格式转换', title: '格式转换' },
  '/augment': { breadcrumb: '数据增强', title: '数据增强' },
  '/organize': { breadcrumb: '数据集管理', title: '数据集管理' },
  '/settings': { breadcrumb: '设置', title: '设置' },
};

export default function Header() {
  const location = useLocation();
  const currentRoute = routeNames[location.pathname] || { breadcrumb: '未知', title: '未知' };

  return (
    <header className="main-header">
      <div className="header-left">
        <div className="header-breadcrumb">
          <span className="header-breadcrumb-item">AI Train Tools</span>
          <span className="header-breadcrumb-separator">/</span>
          <span className="header-breadcrumb-current">{currentRoute.breadcrumb}</span>
        </div>
      </div>
      <div className="header-right">
        <button className="header-btn" title="搜索">
          <Search />
        </button>
        <button className="header-btn" title="通知">
          <Bell />
        </button>
      </div>
    </header>
  );
}
