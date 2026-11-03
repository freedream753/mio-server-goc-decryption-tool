<div align="center">

<h1>GOC 解题妙妙小工具</h1>

**GMOD-RXSEND-MIO 服务器 5K 模式 · GOC 奇术终端解题工具**

自动识别解题页面的三卡片密文，按「前移 / 后移 N 位」标签解密，结果置顶展示、一键复制。

[![License](https://img.shields.io/badge/LICENSE-GPL--3.0-green.svg?style=for-the-badge)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/freedream753/mio-server-goc-decryption-tool?style=for-the-badge)](../../releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2B-blue.svg?style=for-the-badge)](#下载)
[![Built with](https://img.shields.io/badge/built_with-Tauri%20%2B%20Rust-orange.svg?style=for-the-badge)](https://tauri.app)

**简体中文** | [**English**](./README_EN.md)

</div>

---

## 目录

- [功能特性](#功能特性)
- [工作流程](#工作流程)
- [下载](#下载)
- [使用方法](#使用方法)
- [技术实现](#技术实现)
- [从源码构建](#从源码构建)
- [常见问题](#常见问题)
- [更新日志](#更新日志)
- [开源协议与致谢](#开源协议与致谢)

## 功能特性

1. **全自动解题：** `Alt + C` 一键全屏截图，自动定位三张密文卡片，无需手动框选，识别 → 解密 → 复制一气呵成。

2. **离线识别：** 使用 Windows 系统内置 OCR（Windows.Media.Ocr），零模型下载、零网络请求，单文件 exe 即装即用。

3. **滤镜免疫：** 颜色判定采用相对色序（通道差 + 跨色相对序），不依赖绝对 RGB 阈值，英伟达动态亮丽 / 游戏内增亮等滤镜引起的整体颜色漂移不影响识别。

4. **不抢焦点结果窗口：** 结果窗口始终置顶且不夺取游戏焦点（WS_EX_NOACTIVATE），点击任意答案行即复制该答案，不打断游戏按键操作。

5. **自学习修正：** 识别有误时可直接在结果窗口修正答案，修正记录写入本地 JSON，此后同类页面优先参考历史修正，一次纠正永久生效。

## 工作流程

```
Alt + C 全屏截图
    ↓
Rust 像素预筛（亮度通道 + 卡片边框几何特征）定位三张卡片
    ↓
Windows.Media.Ocr 识别「前移 / 后移 N 位」标签 + 大写密文
    ↓
按标签语义解密（后移 N 位 → 每字母 -N；前移 N 位 → 每字母 +N）
    ↓
置顶不抢焦点结果窗口：三行「密文 → 答案」映射，默认复制第一个答案
```

## 下载

前往 [Releases](../../releases) 页面下载单文件免安装 exe：

| 文件 | 架构 |
|---|---|
| `GOC-Solver-v1.0.3-x64.exe` | 64 位 Intel / AMD |
| `GOC-Solver-v1.0.3-x86.exe` | 32 位 x86 |
| `GOC-Solver-v1.0.3-arm64.exe` | ARM64 |

单文件、无外部 DLL 依赖、离线可用。

## 使用方法

1. 运行 exe，程序驻留系统托盘
2. 打开 GOC 奇术终端解题页面，按 `Alt + C`
3. 结果窗口展示三行「密文 → 答案」，点击任意一行复制该答案
4. 10 秒倒计时结束自动隐藏（点击复制 / 拖动窗口会重置倒计时，拖动期间倒计时冻结）
5. 识别有误时点击答案行修改，修正会自动保存

> 系统要求：Windows 10 及以上；识别依赖系统内置中文 OCR 语言包（zh-CN，中文语言模式的 Windows 默认自带）。

## 技术实现

- **识别管线**（`src-tauri/src/solver.rs`）：亮度通道二值化 + 行检测预筛定位卡片，紧裁 ROI 规避 Windows OCR 对宽图返回空串的怪癖，灰度归一化（底色→白、文字→黑、保留抗锯齿）提升识别率，硬二值化兜底
- **解密语义**：后移 N 位 → 每字母 -N，前移 N 位 → 每字母 +N；内置小词表仅做答案校验提示（✓ 命中 / ？未验证）
- **结果窗口**（`src-tauri/src/window.rs`）：Tauri v1 + windows-rs 补充 Win32 调用实现 WS_EX_NOACTIVATE / SW_SHOWNA，拖动窗口期间倒计时冻结
- **三架构构建**：x64（x86_64-pc-windows-gnu）、x86 / ARM64（llvm-mingw gnullvm），产物均通过 PE 头与 DLL 依赖表校验（仅系统库）

## 从源码构建

```bash
pnpm install
pnpm tauri build
```

构建细节与踩坑记录见 [PLAN-v1.0.3-新版解题页面适配.md](./PLAN-v1.0.3-新版解题页面适配.md)。

## 常见问题

<details>
<summary><b>按 Alt + C 没有反应？</b></summary>

确认程序在托盘运行中；确认游戏为无边框窗口化（非独占全屏），独占全屏下置顶窗口不可见。
</details>

<details>
<summary><b>识别结果为空或乱码？</b></summary>

确认系统已安装中文语言包（设置 → 时间和语言 → 语言 → 添加中文）。若个别字符识别错误，直接在结果窗口修正即可，修正会被记住。
</details>

<details>
<summary><b>开滤镜后识别不准？</b></summary>

v1.0.3 起颜色判定已适配英伟达动态亮丽 / 游戏内增亮等亮度类滤镜；若使用强色调滤镜（如夜视模式）仍异常，可提交 issue 附全屏截图。
</details>

## 更新日志

### v1.0.3（2026-11-03）新版解题页面适配

游戏更新三卡片解题页面后的全量重写：

1. **全自动识别：** `Alt + C` 全屏截图自动定位三张密文卡片，不再需要手动框选
2. **OCR 引擎更换：** Tesseract.js（wasm 模型）退役，改用 Windows.Media.Ocr 系统内置引擎——零模型下载、离线可用、exe 更小
3. **按标签解密：** 识别每张卡片「前移 / 后移 N 位」标签并按语义解密（后移 → -N、前移 → +N），取代固定凯撒 -3
4. **新增结果窗口：** 置顶不抢焦点，三行「密文 → 答案」映射，点击复制，10 秒倒计时自动隐藏
5. **新增自学习：** 结果窗口修正答案后永久记住，游戏语义有变改一次即可
6. **滤镜适配：** 相对色序判定 + 灰度归一化，兼容英伟达动态亮丽 / 游戏内增亮引起的颜色漂移
7. 密文词长上限扩展至 3~12 字母；旧手动框选管线退役
8. **实机定位加固：** 对话框列投影自适应阈值、输入框分层回退 + 等宽簇校验、密文带跨簇共识规则，修复全息屏变暗、光标悬停遮挡等实机场景的识别失败

### v1.0.1（2026-10-27）

1. OCR 识别逻辑调优，两张实拍回归图识别率 100%
2. 静态链接 WebView2Loader，实现真正的单文件 exe（免附带 DLL）
3. 修复构建遗漏 `custom-protocol` 导致的全窗口白屏、logo 破图等问题
4. 补全 x86 / arm64 产物，三架构发布规范定型

### v1.0.0（2026-10-19）首个公开发布

1. 手动框选 → Tesseract.js OCR 识别 → 凯撒 -3 解密 → 自动复制剪贴板
2. 提供 x64 / x86 / arm64 三架构单文件 exe

> v1.0.2 为内部构建版本（新增失败日志落盘与托盘「查看日志」），未单独发布，其内容已并入 v1.0.3。完整发布说明见 [Releases](../../releases)。

## 开源协议与致谢

本项目基于 [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop) 修改而来，原项目采用 **GPL-3.0** 协议开源，因此本项目同样以 **GPL-3.0** 协议发布：你可以自由使用、学习、修改和分发本软件，但分发时必须继续采用 GPL-3.0 协议、保留原版权与许可声明，并公开完整的修改后源代码。

- [Tauri](https://tauri.app) — 跨平台桌面应用框架
- [Windows.Media.Ocr](https://learn.microsoft.com/windows/uwp/audio-video-camera/imaging) — Windows 系统内置 OCR
- 由中分神秘博士设计 🧪
