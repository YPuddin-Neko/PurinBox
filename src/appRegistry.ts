import type { ElementType } from 'react';
import {
  Home,
  Crop,
  ScanFace,
  Scaling,
  FlipHorizontal2,
  ScanSearch,
  FileCheck2,
  FileType,
  Layers,
  TextCursorInput,
  Tags,
  List,
  Grid3X3,
  Settings,
  Move3D,
  Sparkles,
  ZoomIn,
  Network,
  Copy,
  Scale,
  FileCode2,
  Star,
  Wand2,
  BarChart3,
  Workflow,
} from 'lucide-react';

/**
 * ═══════════════ 应用页面/任务注册表（单一事实来源） ═══════════════
 *
 * 新增功能页面只需在 PAGES 里加一条记录（并在 App.tsx 的 PAGE_COMPONENTS
 * 补上组件映射，缺失会在启动时报错）。以下内容全部由本表派生，不再各自维护：
 *   - App.tsx 的持久化页面表 / 路由表
 *   - Sidebar 的导航分组
 *   - Header 的面包屑翻译 key（routeI18nMap）与任务跳转（TASK_ROUTE_MAP）
 *   - TaskContext 的进度事件 → 任务 ID 映射（EVENT_TASK_MAP）
 */

export type SectionKey = 'preprocess' | 'dataset' | 'advanced' | 'automation';

/** 页面拥有的全局任务：id 为 addTask 的任务 ID，events 为后端进度事件名 */
export interface PageTaskDef {
  id: string;
  events: string[];
}

export interface PageDef {
  /** 路由路径（唯一键） */
  path: string;
  /** sidebar.* 翻译 key（侧边栏与面包屑共用） */
  i18nKey: string;
  icon: ElementType;
  /** 所属侧边栏分组；home/settings 不属于任何分组 */
  section?: SectionKey;
  /** true = 首次访问后保持挂载（display 切换），false = 走普通路由 */
  persistent: boolean;
  /** 该页面的全局任务（驱动 Header 任务面板与事件订阅） */
  tasks?: PageTaskDef[];
  /** 实验性（测试版）功能：由设置中的开关控制是否在侧边栏显示 */
  experimental?: boolean;
}

export const SECTION_ORDER: { key: SectionKey; titleKey: string }[] = [
  { key: 'preprocess', titleKey: 'sidebar.sectionPreprocess' },
  { key: 'dataset', titleKey: 'sidebar.sectionDataset' },
  { key: 'advanced', titleKey: 'sidebar.sectionAdvanced' },
  { key: 'automation', titleKey: 'sidebar.sectionAutomation' },
];

export const PAGES: PageDef[] = [
  { path: '/', i18nKey: 'sidebar.home', icon: Home, persistent: false },

  // ─── 数据集预处理 ───
  { path: '/aesthetic', i18nKey: 'sidebar.aesthetic', icon: Star, section: 'preprocess', persistent: true,
    tasks: [{ id: 'aesthetic', events: ['aesthetic-progress'] }] },
  { path: '/crop', i18nKey: 'sidebar.crop', icon: Crop, section: 'preprocess', persistent: true,
    tasks: [{ id: 'crop', events: ['crop-progress'] }] },
  { path: '/person-crop', i18nKey: 'sidebar.personCrop', icon: ScanFace, section: 'preprocess', persistent: true,
    tasks: [{ id: 'person-crop', events: ['person-crop-progress'] }] },
  { path: '/scale', i18nKey: 'sidebar.scale', icon: Scaling, section: 'preprocess', persistent: true,
    tasks: [{ id: 'scale', events: ['scale-progress'] }] },
  { path: '/flip', i18nKey: 'sidebar.imageProcess', icon: FlipHorizontal2, section: 'preprocess', persistent: true,
    tasks: [{ id: 'flip', events: ['flip-progress'] }] },
  { path: '/filter', i18nKey: 'sidebar.filter', icon: ScanSearch, section: 'preprocess', persistent: true,
    tasks: [{ id: 'filter', events: ['filter-progress'] }] },
  { path: '/resolution-analyze', i18nKey: 'sidebar.resolutionAnalyze', icon: BarChart3, section: 'preprocess', persistent: true,
    tasks: [{ id: 'resolution-analyze', events: ['resolution-analyze-progress'] }] },
  { path: '/file-keeper', i18nKey: 'sidebar.fileKeeper', icon: FileCheck2, section: 'preprocess', persistent: true,
    tasks: [{ id: 'keeper', events: ['keeper-progress'] }] },
  { path: '/format-convert', i18nKey: 'sidebar.formatConvert', icon: FileType, section: 'preprocess', persistent: true,
    tasks: [{ id: 'convert', events: ['convert-progress'] }] },
  { path: '/alpha-convert', i18nKey: 'sidebar.alphaConvert', icon: Layers, section: 'preprocess', persistent: true,
    tasks: [{ id: 'alpha', events: ['alpha-progress'] }] },
  { path: '/batch-rename', i18nKey: 'sidebar.batchRename', icon: TextCursorInput, section: 'preprocess', persistent: true,
    tasks: [
      { id: 'rename', events: ['rename-progress'] },
      { id: 'dedup-rename', events: ['dedup-rename-progress'] },
    ] },
  { path: '/perspective', i18nKey: 'sidebar.perspective', icon: Move3D, section: 'preprocess', persistent: true,
    tasks: [{ id: 'perspective', events: ['perspective-progress'] }] },
  { path: '/blur-noise', i18nKey: 'sidebar.blurNoise', icon: Sparkles, section: 'preprocess', persistent: true,
    tasks: [{ id: 'blur-noise', events: ['blur-noise-progress'] }] },

  // ─── 数据集处理 ───
  { path: '/tagger', i18nKey: 'sidebar.tagger', icon: Tags, section: 'dataset', persistent: true,
    tasks: [
      // python-env-progress: Python 环境部署进度并入打标任务显示（沿用既有行为）
      { id: 'tagger', events: ['tagger-progress', 'python-env-progress'] },
      { id: 'llm-tagger', events: ['llm-tagger-progress'] },
    ] },
  { path: '/tag-manager', i18nKey: 'sidebar.tagManager', icon: List, section: 'dataset', persistent: true },

  // ─── 高级工具 ───
  { path: '/tag-sort', i18nKey: 'sidebar.tagOptimize', icon: Wand2, section: 'advanced', persistent: true,
    tasks: [
      { id: 'tag-sort', events: ['tag-sort-progress'] },
      { id: 'tag-refine', events: ['tag-refine-progress'] },
    ] },
  { path: '/bucket-preview', i18nKey: 'sidebar.bucketPreview', icon: Grid3X3, section: 'advanced', persistent: true },
  { path: '/upscale', i18nKey: 'sidebar.upscale', icon: ZoomIn, section: 'advanced', persistent: true,
    tasks: [{ id: 'upscale', events: ['upscale-progress'] }] },
  { path: '/image-cluster', i18nKey: 'sidebar.imageCluster', icon: Network, section: 'advanced', persistent: true,
    tasks: [{ id: 'image-cluster', events: ['cluster-progress'] }] },
  { path: '/image-dedup', i18nKey: 'sidebar.imageDedup', icon: Copy, section: 'advanced', persistent: true },
  { path: '/dataset-balancer', i18nKey: 'sidebar.datasetBalancer', icon: Scale, section: 'advanced', persistent: true },
  { path: '/sd-metadata', i18nKey: 'sidebar.sdMetadata', icon: FileCode2, section: 'advanced', persistent: true,
    tasks: [{ id: 'sd-metadata', events: ['sd-metadata-progress'] }] },

  // ─── 自动化 ───
  { path: '/workflow', i18nKey: 'sidebar.workflow', icon: Workflow, section: 'automation', persistent: true, experimental: true },

  { path: '/settings', i18nKey: 'sidebar.settings', icon: Settings, persistent: false },
];

// ═══════════════ 派生映射（原五份手动同步表，勿再手写） ═══════════════

/** 持久化页面（首次访问后保持挂载） */
export const persistentPages = PAGES.filter(p => p.persistent);

/** 普通路由页面 */
export const routePages = PAGES.filter(p => !p.persistent);

/** 路由路径 → 面包屑翻译 key（原 Header routeI18nMap） */
export const routeI18nMap: Record<string, string> = Object.fromEntries(
  PAGES.map(p => [p.path, p.i18nKey]),
);

/** 进度事件名 → 任务 ID（原 TaskContext EVENT_TASK_MAP） */
export const EVENT_TASK_MAP: Record<string, string> = Object.fromEntries(
  PAGES.flatMap(p => (p.tasks ?? []).flatMap(task => task.events.map(event => [event, task.id]))),
);

/** 任务 ID → 路由路径（原 Header TASK_ROUTE_MAP，任务面板点击跳转用） */
export const TASK_ROUTE_MAP: Record<string, string> = Object.fromEntries(
  PAGES.flatMap(p => (p.tasks ?? []).map(task => [task.id, p.path])),
);

/** 侧边栏分组导航（home/settings 由 Sidebar 单独渲染） */
export const navSections = SECTION_ORDER.map(section => ({
  titleKey: section.titleKey,
  items: PAGES.filter(p => p.section === section.key),
})).filter(section => section.items.length > 0);

export const homePage = PAGES.find(p => p.path === '/')!;
export const settingsPage = PAGES.find(p => p.path === '/settings')!;
