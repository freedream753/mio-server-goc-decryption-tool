# v1.1.0 · 移除凯撒解密，快捷键改为自动识别符文

> 状态：**已实施并打包运行通过（v1.1.0）**。本文件是本轮的唯一决策记录与契约来源；实现细节以源码为准。

## 〇、验收结果（调度者独立复跑）

| 项 | 证据 |
|---|---|
| Rust 单测 | `cargo test --manifest-path src-tauri/Cargo.toml rune` → 3 passed；样张定位 4 框、符文「雷 风 风 风」、反馈色「黄 绿 黄 红」 |
| 前端构建 | `pnpm build` → `✓ built in 5.44s` |
| release 打包 | `cargo build --release --target x86_64-pc-windows-msvc --features custom-protocol` → `Finished release profile`，`goc-solver.exe` 18,085,888 B（v1.0.4 为 19,713,024 B） |
| 运行 | PID 存活，HTTP `127.0.0.1:60828` 监听，热键注册成功（用户配置为 `Alt+W`） |
| 端到端（真屏截图） | 把样张全屏铺到 2560×1600 屏后触发扫描：定位 (1103,743)(1201,740)(1304,743)(1403,743) → 雷/风/风/风 + 黄/绿/黄/红（放大后仍 4/4，验证相对几何） |
| 结果窗口 | 截屏取证：4 位符文与颜色正确、`剩余机会 3/4 · 候选 9/1296 · 按数量匹配`、`本次已记录`、推荐 `空·火·水·雷`（与求解器实跑一致） |
| 残留 | `grep caesar|tesseract|get_answer_words|save_correction|Corrections` 全仓 0 命中 |

## 一、目标

1. **删除整条凯撒解密链路**（卡片像素定位 + WinOCR 标签解析 + `caesar()` ±N 解密 + 答案词表 + 自学习修正 + 词表反查/±位移按钮 + Tesseract.js 旧管线）。
2. **快捷键 `Alt+C` 改为「自动识别符文」**：全屏截图 → 定位「当前猜测」的 4 个符文框 → 识别符文（空/火/水/风/土/雷）与反馈色（红/黄/绿）→ 自动写入本局历史 → 置顶窗口展示「下一步推荐提交」并可复制。

## 二、决策记录（访谈结论）

| 编号 | 决策 | 依据 |
|---|---|---|
| D1 | 快捷键走**全自动**：截图→识别盘面→自动写历史→推荐 | 用户第一轮选 A |
| D2 | 凯撒链路**彻底移除**；Result 窗口改造为符文结果载体 | 用户第一轮选 A |
| D3 | 符文不需要图标模板：游戏内符文是**中文单字**渲染，反馈色是**方框边框色** | 用户提供的实拍截图（`src-tauri/tests/fixtures/rune-panel-fullscreen.png`） |
| D4 | 盘面只显示「当前猜测 + 四色反馈」→ 每次热键自动追加一条历史，同一屏去重 | 同上截图（面板含「剩余1次」「破解失败，请重试」） |
| D5 | 识别结果**允许人工逐个纠正**后重算；并提供「撤回本次」「新一局」 | 兜底决策：识别必错场景可救回，成本低 |
| D6 | 保留手动符文助手窗口（`runes`）作为规则切换/新一局/手动录入入口 | 已有功能，不与自动管线冲突 |
| D7 | 参考实现：`D:\yushy\desktop\mio-server-goc-decryption-tool`（v1.0.4）把热键改为打开符文助手、旧管线改名 `legacy_solver_recognize` 留托盘；本轮**不保留 Legacy**（D2），但沿用其「热键→符文」接线思路与置顶窗口参数 | 用户指定参考该目录 |
| D8 | 默认判色规则定为 **`count`（按数量匹配）** | 实跑证据：样张真值 `guess=1209`（雷风风风）/ `result=16`（黄绿黄红）在 `presence` 下 `count=0` 自相矛盾，在 `count` 下有 9 个候选；说明游戏反馈按剩余数量消耗 |
| D9 | 字符识别以**字形模板匹配**为主判据，WinOCR 仅作一致性校验 | T1 实测：WinOCR 中文引擎对孤立单字（12~13px × 12 种预处理组合）全部返回空串，对多字行正常；单字缺行内上下文是硬伤 |
| D10 | 样张真值修正为「雷/风/风/风」+「黄/绿/黄/红」 | 5× 目视 + 逐框边框中位色复核：框2 为青绿（hue≈187，绿=位置正确），此前任务书文字「黄黄红红」为误读 |

## 三、Rust ↔ 前端契约（三份任务书共同遵守）

Rust 扫描完成后：

- 写入静态 `RUNE_SCAN`（最近一次结果）
- emit 事件 **`rune_scan`**（payload 同下）
- 调用 `show_result_window()`（置顶不抢焦点、10 秒倒计时隐藏等现有行为不变）

前端拉取命令：**`get_rune_scan() -> Option<Value>`**（替代旧 `get_solve_result`）。

payload：

```json
{
  "ok": true,
  "slots": [
    { "rune": 5, "color": 1 },
    { "rune": 3, "color": 1 },
    { "rune": 3, "color": 0 },
    { "rune": 3, "color": 0 }
  ],
  "source": "auto",
  "msg": ""
}
```

- `rune`：`0=空 1=火 2=水 3=风 4=土 5=雷`（与 `src/utils/runes.js` 的 `symbols` 同序）；`null`=未识别
- `color`：`0=红 1=黄 2=绿`（与 `src/window/Runes/` 的 `colors` 同序）；`null`=未识别
- `source`：`"auto"`（热键/托盘全屏）或 `"manual"`（框选降级）
- `ok:false` 时 `slots` 为空、`msg` 给人类可读中文失败原因

本局会话语义（前端 localStorage，键 `goc-rune-session-v1`，结构保持不变）：

- `mode`: `"presence" | "count"`
- `history`: `[{ guess, result }]`，`guess = Σ rune_i * 6^i`（0..1295），`result = Σ color_i * 3^i`（0..80）
- `spent`: 0..3

自动写入规则：4 个 `rune` 与 4 个 `color` **全部非 null** 才自动追加；与 `history` 末条完全相同则跳过（同一屏重复按热键）；任一为 null 时不写入，等待用户在窗口里补齐后手点「记录本次」。

## 四、任务分解与文件边界

| 任务 | 范围（只准动这些） | 内容 |
|---|---|---|
| T1 Rust | `src-tauri/src/**`、`src-tauri/Cargo.toml` | 新符文识别模块（定位/边框取色/字符识别）、热键与托盘重接线、删除凯撒链路、结果窗口尺寸调整、回归测试 |
| T2 前端符文化 | `src/window/Result/**`、`src/window/Runes/**`、`src/utils/runes.js`、`src/utils/runes.worker.js`、`src/utils/rune-session.js`、新增 `src/utils/rune-store.js` | Result 窗口重写为符文结果窗口；共享 session 存取；纠正/撤回/新一局 |
| T3 清理收尾 | `src/App.jsx`、`src/window/Config/**`、`src/i18n/locales/*.json`、`package.json`、`public/**`、`src/utils/caesar.js`(删)、`src/window/Recognize/`(删)、`src/services/recognize/`(删)、`README.md`、`README_EN.md`、`RUNE-SOLVER.md`、`scripts/**` | 删凯撒/Tesseract 前端与依赖、删设置页「修正记录」、文档与测试脚本同步 |

## 五、验收标准

1. `cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo test --manifest-path src-tauri/Cargo.toml rune -- --nocapture` 在实拍样张 `src-tauri/tests/fixtures/rune-panel-fullscreen.png` 上跑出 `雷/风/风/风` + `黄/绿/黄/红`（识别率不足时须给出实测输出与降级方案，不得虚报）。
2. `pnpm build` 通过；`node --test scripts/runes.test.mjs` 通过。
3. 全仓 `grep` 无 `caesar`、`CAESAR`、`tesseract`、`get_answer_words`、`save_correction` 等残留（文档中的历史说明除外）。
4. 热键 `Alt+C`、托盘「截图识别符文」、结果窗口 OCR 框选按钮三条入口都能跑同一符文管线；设置页快捷键修改仍然生效。

## 六、风险

| 风险 | 对策 |
|---|---|
| 中文单字 WinOCR 实测不可用（12~13px 单字 12 种预处理全空） | 主判据改为字形模板匹配（内置 6 字模板，来源标注样张裁切），WinOCR 仅做一致性校验，不一致标 `null` 交人工纠正 |
| 面板青色 UI 装饰（hue≈180~200）与「绿」反馈混淆 | 只沿方框边框环采样，不采面板内其他青色元素；几何定位先锁框，再取色 |
| 盘面未提交就按热键 → 脏历史 | 「撤回本次」按钮 + 结果窗口展示识别到的猜测与颜色供核对 |
| 游戏分辨率/DPI 差异 | 全屏搜索 + 尺寸自适应缩放；定位失败降级为框选（`ocr_recognize` 手动入口） |
| 判定规则（存在即黄 / 按数量）未变 | 保留 `mode` 切换，默认 `presence` |
