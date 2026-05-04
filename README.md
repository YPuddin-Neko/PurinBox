<div align="center">

<img src="public/logo.png" alt="PurinBox" width="280" />

# PurinBox

一个AI图片训练数据集处理工具箱

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS-lightgrey)
![Version](https://img.shields.io/github/v/tag/YPuddin-Neko/PurinBox?label=Version)
![Rust](https://img.shields.io/badge/Rust-Tauri%202-orange)
![React](https://img.shields.io/badge/React-19-61DAFB)

</div>

---

PurinBox 是一个基于 **Tauri 2 + React** 的桌面应用，专注于 AI 训练数据集的预处理与管理。提供图片缩放、格式转换、自动打标、标签管理等一站式功能，帮助你快速准备高质量的训练数据。**(软件的所有代码均为AI生成)**

## ✨ 功能特性

### 📦 数据集预处理

| 功能 | 说明 |
| ------ | ------ |
| **图片缩放** | 批量缩放图片至指定尺寸，支持多种插值算法（Lanczos3、CatmullRom 等） |
| **图片处理** | 水平/垂直翻转、旋转等批量图片操作 |
| **分辨率筛选** | 按分辨率范围、宽高比等条件筛选/移动/删除图片 |
| **保留指定文件** | 根据匹配规则保留或删除目录中的文件 |
| **图片格式转换** | 批量转换图片格式（PNG、JPG、WebP、BMP、PSD），自动跳过同格式文件 |
| **转换透明通道** | 将带透明通道的图片转换为指定底色的不透明图片 |
| **批量重命名** | 支持序号、前缀、后缀、正则替换等多种重命名模式 |

### 🏷️ 数据集处理

| 功能 | 说明 |
| ------ | ------ |
| **图片打标 (Tagger)** | 使用 WD Tagger 模型自动生成 Booru 风格标签，支持 CPU/GPU 推理 |
| **图片打标 (LLM)** | 使用大语言模型（OpenAI 兼容 API）生成自然语言描述 |
| **标签管理** | 可视化编辑标签，支持批量添加/删除/替换、拖拽排序、公共标签筛选、标签翻译 |

### ⚙️ 系统功能

- 🌙 深色/浅色/跟随系统 三种主题模式
- 🌐 标签翻译（Google、百度、有道、Bing 四种翻译引擎）
- 📊 实时系统监控（CPU、内存、磁盘）
- 💾 翻译结果本地缓存（SQLite）

## 🚀 快速开始

### 系统要求

- **Windows** 10/11 或 **macOS** 12+
- 使用 Tagger 打标功能需要额外下载模型文件（应用内自动下载）

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
| --- | ------ |
| 前端框架 | React 19 + TypeScript |
| 桌面框架 | Tauri 2 |
| 后端语言 | Rust |
| 图片处理 | image-rs, psd |
| AI 推理 | ONNX Runtime |
| 构建工具 | Vite 7 |
| 样式 | 原生 CSS (CSS Variables) |
| 图标 | Lucide React |
| 数据库 | SQLite (rusqlite) |

## 📁 项目结构

```text
PurinBox/
├── public/               # 静态资源 (Logo, Icon)
├── scripts/              # 构建脚本 (版本同步等)
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
