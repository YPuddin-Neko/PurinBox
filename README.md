<div align="center">

<img src="public/logo.png" alt="PurinBox" width="280" />

# PurinBox

让训练数据准备更轻松的 AI 图片数据集处理工具箱

[![License: PolyForm Noncommercial 1.0.0](https://img.shields.io/badge/License-PolyForm%20Noncommercial%201.0.0-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)
![Version](https://img.shields.io/github/v/tag/YPuddin-Neko/PurinBox?label=Version)
![Rust](https://img.shields.io/badge/Rust-Tauri%202-orange)
![React](https://img.shields.io/badge/React-19-61DAFB)

[下载](https://github.com/YPuddin-Neko/PurinBox/releases/latest) · [功能介绍](#功能介绍) · [快速开始](#快速开始) · [更新日志](CHANGELOG.md)

</div>

---

PurinBox 是基于 **Tauri 2 + React** 的跨平台桌面工具，面向二次元与通用 AI 图像训练的数据集准备流程，涵盖图片预处理、自动打标、可视化标签管理、分桶分析以及超分、聚类、去重等常用工具。

> [!NOTE]
> 本项目是 **Vibe Coding** 的产物 — 从架构设计到每一行代码，全部由 AI 生成。

## 功能介绍

### 数据集预处理

多数处理工具支持输入单张图片或整个目录，并可选择是否递归扫描子文件夹。

| 功能 | 说明 |
| --- | --- |
| **美学评分** | 自动评估图片美学质量，按 7 个等级分类归档 |
| **图片裁切** | 支持中心、填满、比例与边缘四种批量裁切模式 |
| **三分法裁切** | 基于人物检测的构图裁切，自动对齐人物位置 |
| **图片缩放** | 按目标像素面积等比缩放，自动对齐训练常用分辨率 |
| **图片处理** | 批量水平、垂直或双向翻转 |
| **分辨率筛选** | 按宽高或分辨率阈值筛选图片并复制或清理 |
| **分辨率分析** | 统计分辨率分布，识别低频尺寸并按宽高比推荐聚合方案 |
| **保留指定文件** | 按规则批量保留或删除目录中的文件 |
| **图片格式转换** | 批量转为 PNG、JPG、WebP、BMP（源文件另支持 TIFF、GIF、PSD） |
| **转换透明通道** | 将透明图转换为指定纯色背景的不透明图片 |
| **批量重命名** | 按前缀与序号批量重命名，支持打乱顺序与同步重命名关联标签 |
| **透视变换** | 批量随机透视变换，用于数据增强 |
| **模糊 / 噪点** | 批量添加高斯模糊或高斯噪点，用于数据增强 |

### 数据集处理

| 功能 | 说明 |
| --- | --- |
| **图片打标** | 支持 WD / CL / PixAI 等本地 Tagger 模型生成 Danbooru 标签，支持 VLM 生成自然语言描述与 GBC 精细描述；设置中可开启「辅助打标」，结合本地模型初标与 VLM 二次调优 |
| **标签管理** | 可视化管理 Danbooru（TXT）、自然语言（TXT）、Danbooru + 自然语言（JSON）与 GBC（GBC.JSON）四类标注文件；支持多标签筛选、批量增删替换、拖拽排序、标签翻译与大图缩放查看 |

### 高级工具

| 功能 | 说明 |
| --- | --- |
| **标签优化** | 使用大语言模型对标签进行语义排序，或结合图片对已有标签纠错、补充与精简 |
| **分桶预览** | 模拟训练集分桶结果，兼容 SD-Scripts 与 Diffusion Pipe 分桶逻辑，支持参数推荐与导出 |
| **图片超分** | 基于 Real-ESRGAN 对图片进行批量超分辨率放大 |
| **图片聚类** | 按视觉特征自动对图片分组，便于归类画风、角色或场景 |
| **图片去重** | 基于感知哈希检测并清理重复或近似图片 |
| **数据集配平** | LoRA / DreamBooth 多概念文件夹步数与轮次配平计算及分布可视化 |
| **图片元数据** | 读取、编辑 A1111 / ComfyUI / NovelAI 生成元数据，并批量导出正向提示词 |

### GBC 精细描述

PurinBox 集成 KohakuGBC，用于结合画面局部区域与对象关系生成更准确的自然语言描述：

- **入口**：可在「图片打标 → GBC 细化」中对已有 TXT / JSON 标签的图片直接润色自然语言，或在「辅助打标」的自然语言预设中勾选启用。
- **环境与模型**：首次使用时自动在应用数据目录的 `env/gbc` 与 `models/gbc` 中准备独立运行环境及所选检测、嵌入模型（如 OWLv2、SAM 3 等），支持按所选设备（CPU / CUDA / MPS）加载。
- **图结构编辑**：开启保留 GBC 数据后会生成同名 `.gbc.json` 文件，可在「标签管理 → GBC 格式 (GBC.JSON)」中查看层级区域框、调整框选位置与节点描述，并在修改后回到「GBC 细化」重新生成最终自然语言。

### 通用特性

- 浅色 / 深色双主题与中 / 英 / 日三语界面
- 实时硬件监控（CPU、内存、GPU）与全局任务队列
- 四种标签翻译引擎（Google、百度、有道、Bing）与本地 SQLite 缓存
- HTTP / SOCKS5 网络代理与 Hugging Face Token 管理

## 快速开始

### 直接下载

前往 [Releases](https://github.com/YPuddin-Neko/PurinBox/releases/latest) 下载对应平台的安装包：

| 平台 | 安装包 |
| --- | --- |
| Windows | `PurinBox_x.x.x_x64-setup.exe` / 便携版 ZIP |
| macOS (Apple Silicon) | `PurinBox_x.x.x_aarch64.dmg` |
| macOS (Intel) | `PurinBox_x.x.x_x64.dmg` |
| Linux (x64 / ARM64) | `.deb` / `.rpm` / `.AppImage` |

### 系统要求

- **Windows** 10 / 11、**macOS** 12+ 或主流 **Linux** 发行版
- **GPU 加速（可选）**：Windows / Linux 需 CUDA 12.x + cuDNN 9.x，macOS 支持 CoreML / MPS

### 从源码构建

#### 环境依赖

- [Node.js](https://nodejs.org/) 20.19+ 或 22.12+
- [Rust](https://www.rust-lang.org/tools/install) >= 1.88
- [Tauri 2 前置依赖](https://v2.tauri.app/start/prerequisites/)

Linux 构建需额外安装系统库（以 Ubuntu / Debian 为例）：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential curl file libayatana-appindicator3-dev libgtk-3-dev \
  libssl-dev libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev \
  libfuse2 patchelf rpm wget
```

#### 运行与打包

```bash
git clone https://github.com/YPuddin-Neko/PurinBox.git
cd PurinBox

npm install
npm run tauri dev    # 开发模式
npm run tauri build  # 生产构建
```

## 技术栈

| 层 | 技术 |
| --- | --- |
| 前端 | React 19 + TypeScript + Vite 7 |
| 桌面框架 | Tauri 2 (Rust) |
| 图像处理 | image-rs, psd |
| 本地推理 | ONNX Runtime / PyTorch / Real-ESRGAN (Python) |
| 数据存储 | SQLite (rusqlite) |

## 项目结构

```text
PurinBox/
├── public/               # 静态资源 (Logo, Icon)
├── src/                  # 前端源码
│   ├── components/       # 组件
│   ├── pages/            # 功能页面
│   ├── i18n/             # 多语言资源
│   └── styles/           # 样式与主题变量
├── src-tauri/            # Tauri / Rust 后端
│   ├── src/commands/     # Tauri 命令与业务逻辑
│   ├── icons/            # 各平台应用图标
│   └── scripts/          # Python 推理与环境脚本
├── CHANGELOG.md
└── package.json
```

## 许可证

本项目采用 [PolyForm Noncommercial License 1.0.0](LICENSE)，以源码可用的方式提供，允许协议规定的非商业用途；商业用途需另行获得授权。第三方依赖与模型遵循各自许可证。

## 贡献

欢迎提交 Issue 和 Pull Request。
