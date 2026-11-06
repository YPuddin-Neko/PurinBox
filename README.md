<div align="center">

<img src="public/logo.png" alt="PurinBox" width="280" />

# PurinBox

一个AI图片训练数据集处理工具箱

[![License: PolyForm Noncommercial 1.0.0](https://img.shields.io/badge/License-PolyForm%20Noncommercial%201.0.0-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)
![Version](https://img.shields.io/github/v/tag/YPuddin-Neko/PurinBox?label=Version)
![Rust](https://img.shields.io/badge/Rust-Tauri%202-orange)
![React](https://img.shields.io/badge/React-19-61DAFB)

[下载](https://github.com/YPuddin-Neko/PurinBox/releases/latest) · [功能介绍](#功能特性) · [快速开始](#快速开始)

</div>

---

PurinBox 是一个基于 **Tauri 2 + React** 的桌面应用，专注于二次元动漫图的训练数据集的预处理与管理。提供图片处理、自动打标、标签管理、分桶预览、图片超分/聚类/去重等一揽子功能，帮助你快速准备高质量的训练数据。

> [!NOTE]
> 本项目是 **Vibe Coding** 的产物 — 从架构设计到每一行代码，全部由 AI 生成。

## 功能特性

### 数据集预处理

| 功能 | 说明 |
| --- | --- |
| **灵活输入** | 多数图片处理功能支持选择单张图片或文件夹，并可递归扫描子文件夹中的图片 |
| **美学评分** | 使用 AI 模型自动评估图片美学质量，按 7 个等级分类到子文件夹 |
| **图片裁切** | 批量裁切图片，支持中心裁切、填满裁切、比例裁切和边缘裁切；填满裁切可指定保留上方/下方/左侧/右侧画面 |
| **三分法裁切** | 基于检测模型的智能构图裁切，自动识别人物位置进行三分法裁切 |
| **图片缩放** | 按目标像素面积批量等比上采样/下采样，保持原图宽高比并自动对齐到训练常用尺寸 |
| **图片处理** | 批量水平、垂直或双向翻转图片 |
| **分辨率筛选** | 按最小宽度、最小高度或低于/高于指定分辨率筛选图片，对命中的图片执行复制或删除 |
| **分辨率分析** | 统计数据集的分辨率分布，识别稀有尺寸，按宽高比推荐聚合目标分辨率，并可将图片导出到对应的分辨率文件夹 |
| **保留指定文件** | 根据匹配规则保留或删除目录中的文件 |
| **图片格式转换** | 批量转换为 PNG、JPG、WebP 或 BMP（源文件另支持 TIFF、GIF、PSD），自动跳过同格式文件 |
| **转换透明通道** | 将带透明通道的图片转换为指定底色的不透明图片 |
| **批量重命名** | 按前缀加序号批量重命名，可设置起始编号、序号位数和随机打乱顺序 |
| **透视变换** | 批量对图片进行随机透视变换，用于数据增强 |
| **模糊/噪点** | 批量添加高斯模糊和高斯噪点，用于数据增强 |

### 数据集处理

| 功能 | 说明 |
| --- | --- |
| **图片打标** | Tagger 模型（WD / CL）自动生成 Danbooru 风格标签，LLM 生成自然语言描述，支持 CPU/GPU 推理、递归扫描和 TXT/JSON 输出；辅助打标（实验性功能）支持本地模型生成标签后由 LLM 视觉能力进行二次确认与调优，默认隐藏，可在设置中开启 |
| **标签管理** | 可视化编辑标签，支持对当前图片或整个数据集批量添加、删除和替换标签，以及拖拽排序、公共标签筛选、全局标签翻译、标签去重、自动补全、预览大图和双击编辑，兼容 TXT/JSON 格式 |

### 高级工具

| 功能 | 说明 |
| --- | --- |
| **标签优化** | 使用 LLM 对标签文件进行语义排序，或结合图片对已有标签进行细化调优（纠错/补充/移除） |
| **分桶预览** | 预览训练集图片的 Bucket 分桶结果，支持 SD-Scripts 与 Diffusion Pipe 分桶逻辑、递归扫描、推荐参数、图片预览和导出分桶结构 |
| **图片超分** | 使用超分模型对图片进行超分辨率放大 |
| **图片聚类** | 基于视觉特征对数据集图片进行自动聚类分组，发现相似图片 |
| **图片去重** | 基于感知哈希检测并清理重复/相似图片 |
| **数据集配平** | LoRA/DreamBooth 训练步数计算器，支持多概念文件夹配平、按轮次/按步数两种模式、数据分布可视化（树状图/饼图/时间线） |
| **图片元数据** | 读取 A1111/ComfyUI/NovelAI 生成元数据，批量导出正向提示词 |

### 自动化

| 功能 | 说明 |
| --- | --- |
| **自动化工作流（实验性功能）** | 通过节点编排图片处理、AI 打标、筛选和文件操作步骤；默认隐藏，可在设置的实验性功能中开启 |

### 系统功能

- 实时系统监控（CPU、内存、GPU）
- 标签翻译（Google、百度、有道、Bing 四种翻译引擎）
- 网络代理设置（HTTP/SOCKS5）
- 翻译结果本地缓存（SQLite）
- 全局任务队列（多任务并行，实时进度追踪）
- 统一检测 GPU 加速环境，并在不可用时回退到 CPU

## 快速开始

### 直接下载

前往 [Releases](https://github.com/YPuddin-Neko/PurinBox/releases/latest) 下载对应平台的安装包：

| 平台 | 文件 |
| --- | --- |
| Windows | `PurinBox_x.x.x_x64-setup.exe` |
| macOS (Apple Silicon) | `PurinBox_x.x.x_aarch64.dmg` |
| macOS (Intel) | `PurinBox_x.x.x_x64.dmg` |
| Linux (x64 / ARM64) | `.deb` / `.rpm` / `.AppImage` |

### 系统要求

- **Windows** 10/11、**macOS** 12+ 或主流 **Linux** 发行版
- GPU 加速（可选）：Windows/Linux 需要 CUDA 12.x 和 cuDNN 9.x，macOS 使用 CoreML

### 从源码构建

#### 前置依赖

- [Node.js](https://nodejs.org/) 20.19+ 或 22.12+
- [Rust](https://www.rust-lang.org/tools/install) >= 1.88
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

Linux 还需要安装 Tauri 构建依赖（以 Ubuntu/Debian 为例）：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential curl file libayatana-appindicator3-dev libgtk-3-dev \
  libssl-dev libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev \
  libfuse2 patchelf rpm wget
```

#### 构建步骤

```bash
# 克隆项目
git clone https://github.com/YPuddin-Neko/PurinBox.git
cd PurinBox

# 安装前端依赖
npm install

# 开发模式运行
npm run tauri dev

# 构建生产版本
npm run tauri build
```

### KohakuGBC 精细描述

「图片打标 → GBC 细化」选择图片或数据集文件夹，自动读取同目录、同名的 TXT／JSON 标签；两者同时存在时优先读取 JSON。程序直接结合已有描述或标签和 GBC 区域分析进行细化，成功后更新原标签，失败时保留原文件。

在「图片打标 → 辅助打标」的 VLM 调优中，勾选「使用 GBC 细化自然语言」。程序先结合 Tagger 标签和当前调优提示词生成结果，再通过整图、局部区域和对象关系细化自然语言。JSON 只替换自然语言字段，保留标签；辅助打标的预设、标签写回规则和触发词继续生效。只输出标签的预设不启用自然语言细化。

在「设置 → 环境设置」中安装 GBC 独立环境，也可使用 Python 3.11 或更新版本，在项目根目录运行 `python src-tauri/scripts/gbc_setup.py`。依赖安装到 `env/gbc`；首次从固定源码安装数据包需要 Rust 编译环境。安装器固定使用 KohakuGBC `d90f360dbf561f63270522e44fa0e74ebed59c12`。

程序读取 `models/gbc/owlv2-base-patch16-ensemble` 和 `models/gbc/all-MiniLM-L6-v2` 下的本地模型，任务运行时不自动下载。辅助打标中的 GBC 检测跟随原有 CPU／GPU 选择，VLM 通过 API 生成描述。GPU 在 macOS 使用 Apple MPS，其他平台使用 CUDA；嵌入模型在 CPU 上运行。

自然语言保存为同名 TXT，或更新完整／简化 JSON 中的自然语言字段。分析结果另存为 `.gbc.json`。取消会结束当前工作进程，保留已经保存的结果。标签编辑的图片预览会叠加彩色区域框和对应名称，点击图片可放大查看，通过「区域框」开关可对照原图。GBC 和 Layout JSON 在标签编辑中只读展示。

## 技术栈

| 层 | 技术 |
| --- | --- |
| 前端框架 | React 19 + TypeScript |
| 桌面框架 | Tauri 2 |
| 后端语言 | Rust |
| 图片处理 | image-rs, psd |
| AI 推理 | ONNX Runtime (Python) |
| 超分引擎 | Real-ESRGAN (Python) |
| 构建工具 | Vite 7 |
| 样式 | 原生 CSS (CSS Variables) |
| 图标 | Lucide React |
| 数据库 | SQLite (rusqlite) |

## 项目结构

```text
PurinBox/
├── public/               # 静态资源 (Logo, Icon)
├── src/                  # 前端源码
│   ├── components/       # React 组件
│   ├── pages/            # 页面组件
│   ├── i18n/             # 国际化（多语言）
│   └── styles/           # CSS 样式
├── src-tauri/            # Tauri/Rust 后端
│   ├── src/
│   │   ├── commands/     # Tauri 命令 (业务逻辑)
│   │   ├── lib.rs        # 插件注册
│   │   └── main.rs       # 入口
│   ├── icons/            # 应用图标
│   └── scripts/          # Python 推理脚本
├── package.json
└── index.html
```

## 许可证

本项目采用 [PolyForm Noncommercial License 1.0.0](LICENSE)，以源码可用的方式提供，允许协议规定的非商业用途。商业用途需要另行获得授权。

第三方依赖和模型遵循各自的许可证。

## 贡献

欢迎提交 Issue 和 Pull Request！
