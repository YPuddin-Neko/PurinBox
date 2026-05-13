<div align="center">

<img src="public/logo.png" alt="PurinBox" width="280" />

# PurinBox

一个AI图片训练数据集处理工具箱

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS-lightgrey)
![Version](https://img.shields.io/github/v/tag/YPuddin-Neko/PurinBox?label=Version)
![Rust](https://img.shields.io/badge/Rust-Tauri%202-orange)
![React](https://img.shields.io/badge/React-19-61DAFB)

[下载](https://github.com/YPuddin-Neko/PurinBox/releases/latest) · [功能介绍](#-功能特性) · [快速开始](#-快速开始)

</div>

---

PurinBox 是一个基于 **Tauri 2 + React** 的桌面应用，专注于二次元动漫图的训练数据集的预处理与管理。提供图片处理、自动打标、标签管理、图片超分/聚类/去重等一揽子功能，帮助你快速准备高质量的训练数据。

> [!NOTE]
> 本项目是 **Vibe Coding** 的产物 — 从架构设计到每一行代码，全部由 AI 生成。

## ✨ 功能特性

### 📦 数据集预处理

| 功能 | 说明 |
| --- | --- |
| **图片裁切** | 批量裁切图片，支持自由裁切、固定比例、智能居中等模式 |
| **三分法裁切** | 基于检测模型的智能构图裁切，自动识别人物位置进行三分法裁切 |
| **图片缩放** | 批量缩放图片至指定尺寸，支持多种插值算法（Lanczos3、CatmullRom 等） |
| **图片处理** | 水平/垂直翻转、旋转等批量图片操作 |
| **分辨率筛选** | 按分辨率范围、宽高比等条件筛选/移动/删除图片 |
| **保留指定文件** | 根据匹配规则保留或删除目录中的文件 |
| **图片格式转换** | 批量转换图片格式（PNG、JPG、WebP、BMP、PSD），自动跳过同格式文件 |
| **转换透明通道** | 将带透明通道的图片转换为指定底色的不透明图片 |
| **批量重命名** | 支持序号、前缀、后缀、正则替换等多种重命名模式 |
| **透视变换** | 批量对图片进行随机透视变换，用于数据增强 |
| **模糊/噪点** | 批量添加高斯模糊、运动模糊、高斯噪点等效果，用于数据增强 |

### 🏷️ 数据集处理

| 功能 | 说明 |
| --- | --- |
| **图片打标** | Tagger 模型（WD / CL）自动生成 Danbooru 风格标签，LLM 生成自然语言描述，支持 CPU/GPU 推理，支持 TXT/JSON 输出 |
| **标签管理** | 可视化编辑标签，支持批量添加/删除/替换、拖拽排序、公共标签筛选、标签翻译，兼容 TXT/JSON 格式 |

### 🔧 高级工具

| 功能 | 说明 |
| --- | --- |
| **标签排序** | 使用 LLM 对标签文件进行语义排序，支持并发处理、标签对比验证 |
| **分桶预览** | 预览训练集图片的 Bucket 分桶结果，支持图片预览和导出分桶结构 |
| **图片超分** | 使用超分模型对图片进行超分辨率放大 |
| **图片聚类** | 基于视觉特征对数据集图片进行自动聚类分组，发现相似图片 |
| **图片去重** | 基于感知哈希检测并清理重复/相似图片 |

### ⚙️ 系统功能

- 📊 实时系统监控（CPU、内存、GPU）
- 🌐 标签翻译（Google、百度、有道、Bing 四种翻译引擎）
- 🔌 网络代理设置（HTTP/SOCKS5）
- 💾 翻译结果本地缓存（SQLite）
- 🎯 全局任务队列（多任务并行，实时进度追踪）

## 🚀 快速开始

### 直接下载

前往 [Releases](https://github.com/YPuddin-Neko/PurinBox/releases/latest) 下载对应平台的安装包：

| 平台 | 文件 |
| --- | --- |
| Windows | `PurinBox_x.x.x_x64-setup.exe` |
| macOS (Apple Silicon) | `PurinBox_x.x.x_aarch64.dmg` |
| macOS (Intel) | `PurinBox_x.x.x_x64.dmg` |

### 系统要求

- **Windows** 10/11 或 **macOS** 12+
- GPU 加速：Windows 需要 CUDA，macOS 使用 CoreML

### 从源码构建

#### 前置依赖

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://www.rust-lang.org/tools/install) >= 1.77
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

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

## 🏗️ 技术栈

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

## 📁 项目结构

```text
PurinBox/
├── public/               # 静态资源 (Logo, Icon)
├── src/                  # 前端源码
│   ├── components/       # React 组件
│   ├── pages/            # 页面组件
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

## 📄 许可证

本项目基于 [GNU General Public License v3.0](LICENSE) 许可证开源。

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！
