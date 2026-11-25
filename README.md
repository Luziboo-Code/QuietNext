<h1 align="center">QuietNext</h1>

<p align="center">
  <b>本地优先、原生快速的多维笔记与知识库</b><br>
  A local-first, native workspace for notes, structured data and visual thinking.
</p>

<p align="center">
  <a href="https://github.com/Luziboo-Code/QuietNext/releases/latest"><img src="https://img.shields.io/github/v/release/Luziboo-Code/QuietNext?label=release&color=2f6feb" alt="Latest release"></a>
  <a href="https://github.com/Luziboo-Code/QuietNext/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Luziboo-Code/QuietNext/ci.yml?branch=main&label=CI" alt="CI"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey" alt="Platforms">
  <img src="https://img.shields.io/badge/built%20with-Rust%20%2B%20GPUI-dea584?logo=rust" alt="Rust and GPUI">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/Luziboo-Code/QuietNext?color=green" alt="MIT license"></a>
</p>

<p align="center">
  <a href="https://github.com/Luziboo-Code/QuietNext/releases/latest"><b>下载</b></a> ·
  <a href="#功能">功能</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#开发">开发</a> ·
  <a href="docs/ARCHITECTURE.md">架构</a>
</p>

---

## 简介

QuietNext 是一款用 **Rust + GPUI** 原生绘制的本地笔记应用。它把富文本文档、Markdown、多维数据表、画板、日历和提醒放在同一个工作区中，所有内容都保存在你自己的电脑上。

无需账号，默认不联网，不依赖 Electron 或 WebView。侧栏中的每个文件夹都是磁盘上的真实文件夹，每篇文档都是可独立保存和同步的文件。停止使用 QuietNext 后，你的数据仍然保持原样。

## 功能

### 写作与文档

- 富文本文档与 Markdown 文档并存，支持标题、列表、待办、引用、高亮块、分栏、表格、代码块、LaTeX 公式、图片、附件与封面。
- Markdown 快捷输入与 `/` 插入菜单。
- 字体颜色、背景颜色、标题自动编号，以及可配置的编号起始值。
- 文本批注：选中文字后添加批注，在右侧面板中编辑、管理或批量删除。
- 文档内查找替换、全文搜索、分屏编辑、模板、版本历史和回收站。
- 文件卡片双击使用系统默认程序打开；粘贴文件路径时可选择「粘贴为文本」或「导入文件」。
- 粘贴网址时可选择「粘贴为文本」或「粘贴为书签」，书签卡片会自动抓取网页标题与描述。

### 多维数据表

- 表格、看板、画廊、甘特/时间轴四种视图。
- 自定义属性与字段类型，支持筛选、分组、排序、统计和行填色。
- 看板按单选/人员字段分列；画廊与时间轴可直接打开记录详情。
- 数据保存在独立的 `.sqlite` 文件中，可导入和导出 CSV。

### 画板

- 内置基于 Excalidraw 的白板：图形、箭头、自由绘制、文字、便签、分区和图片。
- 可作为页面中的块，也可保存为独立的 `.excalidraw` 文件。
- 支持缩放、拖拽和大规模图形场景。

### 组织与同步

- 真实文件夹与嵌套目录，文档可置顶、收藏、移动和批量管理。
- WebDAV 与 S3 兼容对象存储同步，支持手动上传、下载和双向同步。
- 可选每五分钟自动同步；冲突保留云端副本，覆盖前备份本地文件。
- 随机打开文档：在设置中勾选文档或文件夹，按 `Ctrl+Alt+R` 随机打开一篇。
- 日历活动与日期提醒；提醒在应用内右上角弹窗显示。
- 多语言界面、内置主题和自定义主题。

## 下载

最新版本与安装包：

**https://github.com/Luziboo-Code/QuietNext/releases/latest**

| 平台 | 安装包 | 系统要求 |
|---|---|---|
| Windows x64 | `QuietNext-<version>-windows-x64-setup.exe` | Windows 10 1809 或更高 |
| macOS | 从源码构建 | macOS 13 或更高 |
| Linux | 从源码构建 | 现代 x86_64 桌面环境 |

### Windows 安装

运行安装程序即可，无需管理员权限。默认安装到当前用户目录，并创建开始菜单快捷方式；卸载不会删除你的笔记库。

如果首次运行时出现 SmartScreen 提示，选择「更多信息 → 仍要运行」。

## 数据与隐私

- 默认文档库位于 `文档/QuietNext`，可在首次启动时更改。
- 元数据、历史版本和同步配置保存在文档库内的 `.quiet/` 目录。
- 应用不包含遥测，不会上传文档内容。
- 网络访问仅发生在你主动使用同步、远程图片、书签抓取或更新检查时。
- WebDAV/S3 凭据只保存在本机配置中。

## 快速开始

### 运行发布版

从 [Releases](https://github.com/Luziboo-Code/QuietNext/releases/latest) 下载 Windows 安装包并安装。首次启动可设置语言、文档库位置和快捷键。

### 从源码运行

需要 Rust stable 工具链；Windows 还需要 Visual Studio Build Tools 与 Windows SDK。

```sh
git clone https://github.com/Luziboo-Code/QuietNext.git
cd QuietNext
cargo build --release --bin QuietNext
```

生成 Windows 安装包：

```powershell
powershell -ExecutionPolicy Bypass -File packaging/windows/setup/build.ps1
```

输出位于 `dist/`。

## 开发

```sh
scripts/check.sh
cargo test --bin QuietNext
cargo clippy --all-targets -- -D warnings
```

项目结构：

```text
crates/core/   文档引擎（模型、DocxXML、编辑核心、Markdown、文档库、数据库表）
src/           GPUI 界面、编辑器、数据表、画板、日历、同步与设置
packaging/     Windows/macOS/Linux 打包脚本
docs/          架构、格式、主题与设计文档
```

更多说明见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 与 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

代码以 [MIT](LICENSE) 许可发布。第三方代码、字体和图标见 [THIRD_PARTY.md](THIRD_PARTY.md)。

部分图标与品牌资源不属于 MIT 许可范围。若你要再分发或创建衍生版本，请阅读 [THIRD_PARTY.md](THIRD_PARTY.md)，并替换受限图标与品牌名称。

---

<p align="center">
  <sub>QuietNext · local-first notes, structured data and visual thinking.</sub>
</p>
