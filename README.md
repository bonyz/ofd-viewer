# OFD 查看器

纯 Rust 编写的桌面 OFD（开放版式文档，国标 GB/T 33190-2016）查看器，单 exe 免环境依赖，解决"网上 OFD 工具都收费或捆绑"的问题。

- 版本：v0.3.0
- 作者：shaoyi1998
- 版权：© 2026 shaoyi1998
- 主页：https://github.com/shaoyi1998/ofd-viewer

## 功能

- 打开 .ofd 文件（按钮 / 命令行参数 / 拖拽）
- 渲染：文字（明文 + 嵌入子集字体精确字形 + Stroke 描边加粗）、图片、矢量路径、电子印章（Annots）
- 页面方向：竖向 / 横向（页内 PhysicalBox）
- 文字 CTM 变换（缩放/平移）
- 浏览器 PDF 式连续滚动：滚轮滚动、PageUp/PageDown 翻页、Ctrl+滚轮缩放
- 鼠标框选任意区域复制文字（Ctrl+C / 右键 / 按钮，Windows API 直写剪贴板）
- 页码跳转（侧栏输入 + 首页/末页）
- Ctrl+F 文字搜索（高亮、上一个 / 下一个、自动跳页）
- 导出 PDF（每页嵌 JPEG）
- Windows 原生打印（打印机对话框，300DPI 整页）
- 中英文界面（跟随系统语言，可手动切换）
- 关于对话框

## 使用

```bash
# 双击 exe，或：
ofd-viewer.exe 你的文件.ofd

# 开发运行
cargo run --release -- "你的文件.ofd"
```

快捷键：滚轮滚动 · PageUp/PageDown 翻页 · Ctrl+滚轮缩放 · Ctrl+F 查找 · Ctrl+C 复制框选 · 拖拽 .ofd 打开。

## 技术

- 解析：`zip` + `quick-xml`
- 渲染：`tiny-skia`（画布/路径）+ `ab_glyph`（字体栅格化/轮廓描边）+ `image`（图片编解码）
- GUI：`eframe`(egui 0.34) + `rfd`（原生文件对话框）
- 国际化：`sys-locale`（识别系统语言）+ 自建 i18n 模块
- 剪贴板：Windows API（OpenClipboard + SetClipboardData）
- 打印：Windows API（PrintDlgW + StretchDIBits）
- OFD 标准：左上原点 y 向下（GB/T 33190-2016）

## 构建

```bash
cargo build --release   # 产出 target/release/ofd-viewer.exe（约 9-10MB）
```

release profile 已开启 lto + strip + panic=abort + opt-level=z。

## 许可

MIT License，版权 © 2026 shaoyi1998。
