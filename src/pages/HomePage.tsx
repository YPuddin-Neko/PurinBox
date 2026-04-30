import { Link } from 'react-router-dom';
import {
  Scaling,
  FlipHorizontal2,
  ScanSearch,
  FileCheck2,
  FileType,
  Layers,
  TextCursorInput,
  Tags,
  Crop,
  ImagePlus,
  FolderSync,
  Image,
  BarChart3,
  HardDrive,
  Clock,
  Zap,
} from 'lucide-react';

const tools = [
  {
    id: 'scale',
    title: '图片缩放',
    desc: '批量上采样或下采样图片到目标分辨率，适配模型训练需求',
    icon: <Scaling />,
    color: 'purple',
    path: '/scale',
    tags: ['上采样', '下采样', '批量'],
    category: '数据集预处理',
  },
  {
    id: 'flip',
    title: '图片处理',
    desc: '对图片进行水平或垂直镜像翻转，用于数据增强',
    icon: <FlipHorizontal2 />,
    color: 'cyan',
    path: '/flip',
    tags: ['水平翻转', '垂直翻转', '批量'],
    category: '数据集预处理',
  },
  {
    id: 'filter',
    title: '分辨率筛选',
    desc: '根据分辨率条件筛选图片，支持删除或输出匹配的图片',
    icon: <ScanSearch />,
    color: 'pink',
    path: '/filter',
    tags: ['筛选', '分辨率', '清理'],
    category: '数据集预处理',
  },
  {
    id: 'file-keeper',
    title: '保留指定文件',
    desc: '勾选要保留的文件类型，一键删除其他文件',
    icon: <FileCheck2 />,
    color: 'orange',
    path: '/file-keeper',
    tags: ['清理', '后缀', '保留'],
    category: '数据集预处理',
  },
  {
    id: 'format-convert',
    title: '图片格式转换',
    desc: '支持 PSD 在内的主流格式转换到 PNG/JPG/WebP 等',
    icon: <FileType />,
    color: 'green',
    path: '/format-convert',
    tags: ['PSD', '格式', '批量'],
    category: '数据集预处理',
  },
  {
    id: 'alpha-convert',
    title: '转换透明通道',
    desc: '检测图片透明通道并转换为不透明图片',
    icon: <Layers />,
    color: 'blue',
    path: '/alpha-convert',
    tags: ['Alpha', '透明', '背景'],
    category: '数据集预处理',
  },
  {
    id: 'batch-rename',
    title: '批量重命名',
    desc: '按规则批量重命名图片，支持自定义前缀、编号和打乱顺序',
    icon: <TextCursorInput />,
    color: 'cyan',
    path: '/batch-rename',
    tags: ['重命名', '前缀', '编号'],
    category: '数据集预处理',
  },
  {
    id: 'tagger',
    title: '图片打标',
    desc: '使用 AI 模型自动为训练图片生成标签，支持 WD Tagger 等主流模型',
    icon: <Tags />,
    color: 'orange',
    path: '/tagger',
    tags: ['AI打标', 'WD Tagger', 'ONNX'],
    category: '数据集处理',
  },
  {
    id: 'crop',
    title: '图像裁切',
    desc: '智能裁切训练图像，支持自定义比例和批量处理',
    icon: <Crop />,
    color: 'green',
    path: '/crop',
    tags: ['裁切', '比例', '智能'],
    category: '更多工具',
  },
];

const stats = [
  { label: '已处理图片', value: '0', icon: <Image />, color: 'purple' },
  { label: '今日任务', value: '0', icon: <BarChart3 />, color: 'cyan' },
  { label: '数据集大小', value: '0 MB', icon: <HardDrive />, color: 'pink' },
  { label: '节省时间', value: '0 min', icon: <Clock />, color: 'green' },
];

export default function HomePage() {
  const trainingTools = tools.filter((t) => t.category === '数据集预处理');
  const processingTools = tools.filter((t) => t.category === '数据集处理');
  const otherTools = tools.filter((t) => t.category === '更多工具');

  return (
    <div className="page">
      {/* Page Header */}
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '4px' }}>
          <Zap style={{ width: 28, height: 28, color: 'var(--color-accent-primary)' }} />
          <h1 className="page-title">工作台</h1>
        </div>
        <p className="page-subtitle">
          AI 训练数据处理工具集，助力高效准备训练数据
        </p>
      </div>

      {/* Stats */}
      <div className="stats-bar">
        {stats.map((stat) => (
          <div key={stat.label} className="stat-card">
            <div className={`stat-icon ${stat.color}`}>{stat.icon}</div>
            <div className="stat-info">
              <span className="stat-value">{stat.value}</span>
              <span className="stat-label">{stat.label}</span>
            </div>
          </div>
        ))}
      </div>

      {/* 数据集预处理 */}
      <div style={{ marginBottom: 'var(--space-4)' }}>
        <h2 style={{
          fontSize: 'var(--font-size-lg)', fontWeight: 700,
          color: 'var(--color-text-primary)', marginBottom: 'var(--space-5)',
          letterSpacing: '-0.01em',
          display: 'flex', alignItems: 'center', gap: 'var(--space-2)',
        }}>
          <span style={{
            width: 3, height: 18, borderRadius: 2,
            background: 'var(--color-gradient-primary)', display: 'inline-block',
          }} />
          数据集预处理
        </h2>
      </div>
      <div className="tools-grid" style={{ marginBottom: 'var(--space-8)' }}>
        {trainingTools.map((tool, index) => (
          <Link key={tool.id} to={tool.path} className="tool-card" style={{ animationDelay: `${index * 0.05}s` }}>
            <div className={`tool-card-icon ${tool.color}`}>{tool.icon}</div>
            <div>
              <div className="tool-card-title">{tool.title}</div>
              <div className="tool-card-desc">{tool.desc}</div>
            </div>
            <div className="tool-card-tags">
              {tool.tags.map((tag) => (<span key={tag} className="tool-card-tag">{tag}</span>))}
            </div>
          </Link>
        ))}
      </div>

      {/* 数据集处理 */}
      <div style={{ marginBottom: 'var(--space-4)' }}>
        <h2 style={{
          fontSize: 'var(--font-size-lg)', fontWeight: 700,
          color: 'var(--color-text-primary)', marginBottom: 'var(--space-5)',
          letterSpacing: '-0.01em',
          display: 'flex', alignItems: 'center', gap: 'var(--space-2)',
        }}>
          <span style={{
            width: 3, height: 18, borderRadius: 2,
            background: 'linear-gradient(135deg, #f59e0b, #f97316)', display: 'inline-block',
          }} />
          数据集处理
        </h2>
      </div>
      <div className="tools-grid" style={{ marginBottom: 'var(--space-8)' }}>
        {processingTools.map((tool, index) => (
          <Link key={tool.id} to={tool.path} className="tool-card" style={{ animationDelay: `${index * 0.05}s` }}>
            <div className={`tool-card-icon ${tool.color}`}>{tool.icon}</div>
            <div>
              <div className="tool-card-title">{tool.title}</div>
              <div className="tool-card-desc">{tool.desc}</div>
            </div>
            <div className="tool-card-tags">
              {tool.tags.map((tag) => (<span key={tag} className="tool-card-tag">{tag}</span>))}
            </div>
          </Link>
        ))}
      </div>

      {/* 更多工具 */}
      <div style={{ marginBottom: 'var(--space-4)' }}>
        <h2 style={{
          fontSize: 'var(--font-size-lg)', fontWeight: 700,
          color: 'var(--color-text-primary)', marginBottom: 'var(--space-5)',
          letterSpacing: '-0.01em',
          display: 'flex', alignItems: 'center', gap: 'var(--space-2)',
        }}>
          <span style={{
            width: 3, height: 18, borderRadius: 2,
            background: 'var(--color-gradient-secondary)', display: 'inline-block',
          }} />
          更多工具
        </h2>
      </div>
      <div className="tools-grid">
        {otherTools.map((tool, index) => (
          <Link key={tool.id} to={tool.path} className="tool-card" style={{ animationDelay: `${index * 0.05}s` }}>
            <div className={`tool-card-icon ${tool.color}`}>{tool.icon}</div>
            <div>
              <div className="tool-card-title">{tool.title}</div>
              <div className="tool-card-desc">{tool.desc}</div>
            </div>
            <div className="tool-card-tags">
              {tool.tags.map((tag) => (<span key={tag} className="tool-card-tag">{tag}</span>))}
            </div>
          </Link>
        ))}
      </div>
    </div>
  );
}
