# GOC 解题妙妙小工具

GMOD-RXSEND-MIO服务器5K模式中，为解决GOC学术造假问题的由中分神秘博士设计的解题工具。该工具利用[pot](https://github.com/pot-app/pot-desktop)开源项目，通过OCR识别密码后自动进行凯撒密码解密（向前移三位），并将结果复制到剪贴板，方便玩家做题。

## 工作流程

截图框选 → Tesseract.js OCR 识别英文 → 凯撒密码解密（向前移三位）→ 明文自动写入剪贴板并弹出系统通知。

## 使用方法

-   快捷键：`Alt + C`（可在偏好设置中修改）
-   托盘菜单：点击托盘图标的「截图解密 (OCR+凯撒)」
-   框选目标区域后松开鼠标，解密结果自动进入剪贴板

## 构建

```bash
pnpm install
pnpm tauri build
```

## 开源协议

本项目基于 [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop) 修改而来，原项目采用 **GPL-3.0** 协议开源，因此本项目同样以 **GPL-3.0** 协议发布：你可以自由使用、学习、修改和分发本软件，但分发时必须继续采用 GPL-3.0 协议、保留原版权与许可声明，并公开完整的修改后源代码。
