# v1.2.0 · 四关卡手动助手改造

> 状态：**已完成（2026-11-12）**。本文件是本轮唯一决策记录与契约来源。用户已拍板：改造现有 Runes 窗口为「关卡助手」；第三关保留热键自动识别；第四关做 A（放大镜）+ B（目标单词跟踪）。
> 核心诉求：取消 10s 倒计时（全部手动关闭）；不自动过关，界面提供「下一关」按钮手动推进。

## 〇、四关卡与验收口径

1. **第一关 能量校准**：按快捷键（新热键 `hotkey_wave_scan`，默认 Alt+V）→ 识别黄色系统目标曲线 → 输出四个滑块 0~100（振幅/频率/相位/偏移），用户照抄。算法 = GOC解题工具11.9/analysis/solver_lib.py 的 analyze()（绝对值模式，只识别目标线）。
2. **第二关 西蒙符印**：助手窗口开始/停止同按钮；录制期间定时截屏，定位 2×3 符文格（火水风/土雷光），只记 暗→亮 边沿，实时显示「第N步·雷」；不适配 4/5/6 次数。
3. **第三关 密码破解**：核心算法 = 用户提供版（count 规则 Counter 消耗式反馈 + 1296×1296 矩阵 + lru_cache minimax，首轮固定 空火水风=index 51，4 次内必解）。现有 runes.js 的 count 分支就是该算法，删除 presence 分支即可。UI 按游戏盘面复刻：当前猜测 4 框（照抄）+ 点击循环 绿→红→黄 的反馈色 + 6 符文选项按钮（修改选中框符号）+「修正」按钮。删除：判色规则切换/剩余次数/未录入次数/提交解密。
4. **第四关 符文转轮**：A=放大镜（框选转轮区域 → Rust ~6fps 截屏该区域 ×3 放大推流显示）；B=目标单词跟踪（用户输入单词，大字显示当前字母，点「已固定」推进；中心字母 OCR 识别 en 引擎尝试，失败则仅放大镜）。不做自动点击。

## 一、Rust 侧改动

### 已完成
- **`src-tauri/src/wave.rs`（新建，已完成）**：`wave_scan()` 热键入口（单飞 WAVE_IN_FLIGHT）→ capture_fullscreen → solve_image：橙掩膜(R-B>18&R>75, 上部75%)≥150px → x0/x1=0.3/99.7分位 → 近黑行块(和<30, >半宽)含中位y → 三级阈值(330,45)(280,30)(230,18)按 sum>tsum & B-R<tbr 提列均值（cov≥0.2&std>2，否则最大cov兜底）→ y归一化 -(y-hh)/hh → fit_two（基波0.3~3.3步0.1粗搜+refine_f(0.06)，二次谐波[1.5f1,min(9.5,3.3f1)]步0.05粗搜+refine_f(0.04)，两轮联合0.012）→ to_sliders: amp=(A1-0.2)/0.4*100, freq=(f1-0.3)/2.7*100, phase=p1/2π*100, off=(0.15-off)/0.36*100，clamp 0..100。payload={ok, sliders:{amp,freq,phase,off}, msg}，写 WAVE_SCAN 静态 + emit "wave_scan" + show/focus runes 窗口。测试：level1-wave.png 样张 + 合成信号 roundtrip。

### 待完成
1. **`solver.rs`**：加 `capture_monitor_to_mem() -> Result<RgbaImage,String>`（照抄 capture_fullscreen 的显示器选择逻辑，但不写文件直接返回 img.to_rgba8()——注意 screenshots 0.7.2 返回类型是自己的 Image，`img.to_rgba8()` 不存在，用 `image::load_from_memory(&img.to_png(Compression::Fast))` 或按 width/height/bytes 手工组 RgbaImage，实现时看 crate API）；ocr 模块加 `en_engine()`（TryCreateFromLanguage("en-US")），recognize_png 复用。
2. **`simon.rs`（新建）**：
   - static SIMON_RUNNING: AtomicBool、SIMON_STEPS: Mutex<Vec<Value>>。
   - `simon_start()`：清空 steps、RUNNING=true、emit "simon_state"{running:true}；spawn 线程：首次定位网格（失败重试~10帧≈2s 后 emit simon_state{running:false,error} 并停）；随后 ~200ms 周期 capture_monitor_to_mem → 对缓存 tiles 做 lit 检测（内部中央50%区域 min(R,G,B) 中位数 > 70 = 亮，亮2帧确认/暗2帧清除的滞回）→ 新亮块 push {seq,index} + emit "simon_step"{seq,index,name}。
   - `simon_stop()`：RUNNING=false + emit simon_state{running:false,total}。
   - `get_simon_steps()` 命令返回 Mutex 内容。
   - 网格定位：掩膜 = B>R+15 && B>70（蓝色系边框，亮块填充也命中）；行带(bands)+竖边(bars，thr=60%带高，2px桥接，逻辑照抄 runes.rs 的 bands/bars/refine_y 可直接复制改) → 每6条竖边窗口组3框 → 行校验：3框尺寸互差≤1/3、近正方形0.6~1.7、节距均匀≤1/3、框宽/节距∈[0.3,0.85]（不放 ring_contrast，亮块是实心的）→ 两行配对：x 中心逐列对齐±15%、行高相近、行间距∈(0.1h, 1.2h)。符号名 ["火","水","风","土","雷","光"]（第二关无「空」，光=第6格）。
3. **`wheel.rs`（新建）**：
   - static WHEEL_REGION: Mutex<Option<(u32,u32,u32,u32)>>（物理像素）、WHEEL_RUNNING: AtomicBool、LAST_CUT: Mutex<Option<(i32,i32,u32,u32)>>。
   - `wheel_select_region()`：SHOT_PURPOSE 设 "wheel"（见 window.rs）→ 隐藏 runes 窗口 → spawn thread 跑 window::ocr_recognize（拉框选窗）。
   - `on_region_selected()`（window.rs success 事件派发调用）：WHEEL_REGION = LAST_CUT → emit "wheel_region_set" → show runes 窗口。
   - `wheel_start()`：需 WHEEL_REGION（无则 emit "wheel_frame"{error:"请先框选转轮区域"}）；spawn 线程：en_engine 一次创建；~150ms 周期 capture_monitor_to_mem → crop region（clamp）→ ×3 CatmullRom → to_png → base64 → emit "wheel_frame"{data:"data:image/png;base64,..."}；中心字母识别：region 纵向 35%~65% 中央带 → min(R,G,B) 通道减中位背景 → 反相(黑字白底) → ×6 放大 → pad_canvas(16,白) → en OCR → 单个大写 A-Z 则 emit "wheel_frame"{letter} 否则 letter:null。
   - `wheel_stop()`：RUNNING=false。
   - base64 用 `base64::engine::general_purpose::STANDARD.encode`（base64 0.22 已在依赖）。
4. **`window.rs`**：
   - **删除倒计时**：RESULT_DEADLINE、RESULT_DRAG_HELD、ensure_result_reaper、touch_result_deadline、reset_result_timer、begin_result_drag 全删；result_window_notify 里去掉 ensure_result_reaper 调用；show_result_window 去掉 touch_result_deadline。
   - `rune_window()`：标题改 "GOC · 关卡助手"，尺寸 680x780（min 600x560）。
   - 加 `show_runes_window()`（wave_scan 用：show+focus runes 窗口，不存在则调 rune_window）。
   - 加 static SHOTPurpose（Mutex<&'static str>，值 "rune"|"wheel"，默认 "rune"）：screenshot_window 的 listen("success") 改为按 purpose 派发：rune → runes::rune_scan_manual()；wheel → wheel::on_region_selected()。
5. **`cmd.rs` + `main.rs` invoke_handler**：删 reset_result_timer、begin_result_drag；加 get_wave_scan、wave_scan_cmd(转调 wave::wave_scan)、simon_start、simon_stop、get_simon_steps、wheel_select_region、wheel_start、wheel_stop、cut_image 内写 LAST_CUT（加到 wheel.rs 的 LAST_CUT 静态，cmd.rs cut_image 末尾赋值）。
6. **`hotkey.rs`**：register_shortcut "all" 注册两条；"hotkey_wave_scan" → wave::wave_scan；update_shortcut 按 name 选 handler（rune_scan / wave_scan），失败回滚逻辑保持。默认值 "Alt+V"。
7. **`tray.rs`**：id "runes" 菜单项文案 zh_cn "关卡助手"、en "Level Assistant"、zh_tw "關卡助手"。
8. **fixtures**：把 `20261112最新用例\` 五张图拷到 `src-tauri\tests\fixtures\`：level1-wave.png(第一关)、level2-simon.png(第二关)、level2-simon-lit.png(第二关亮起)、level3-panel.png(第三关)、level4-wheel.png(第四关)。
9. **Rust 测试**（各模块 tests）：simon 定位 level2-simon.png 断言 2x3=6 框、lit 图断言亮块=第5格(雷, index 4)；wheel 对 level4-wheel.png 裁 (310,170,196,196) 区域跑中心字母识别，有 en 引擎时断言 "H"，无引擎则打印跳过。
10. 版本号：package.json / Cargo.toml / tauri.conf.json 三处 1.1.0 → 1.2.0。

## 二、前端改动

1. **`src/utils/runes.js`**：删 presence 分支，`feedback(g,s)` 固定 count 规则；`createSolver()` 无参（内部 matrix/cache/choose 保留现有实现，guesses=cs.length===1296?[51]:codes.keys()）。
2. **`src/utils/rune-session.js`**：`analyze(solver, history)`；turns=4-history.length；返回 {count,turns,solved,guess}。
3. **`src/utils/rune-store.js`**：session 精简为 {history}，SESSION_KEY 升 "goc-rune-session-v2"；删 mode/setMode/spent/setSpent；remainingTurns(s)=4-history.length；其余 appendRecord/undoLast/startNewGame/isRecorded/recordFromSlots/encode* 保留。
4. **`src/utils/runes.worker.js`**：消息 {id, history} → analyze(solver, history)。
5. **`src/window/Runes/index.jsx` 重写为关卡助手**：header「GOC · 关卡助手」+「下一关」按钮（tab 顺序 wave→simon→cipher→wheel 循环）+ 隐藏按钮。四页：
   - wave：说明 + 「识别目标曲线」按钮（invoke('wave_scan_cmd')；也可按 Alt+V）+ 挂载时 get_wave_scan + listen('wave_scan')；结果四行大字：振幅 X · 频率 X · 相位 X · 偏移 X（照抄）；失败显示 msg。
   - simon：开始/停止同按钮（simon_start/simon_stop）+ listen('simon_step'/'simon_state') + 挂载 get_simon_steps；序列显示「第1步·雷 → …」。
   - cipher：状态 guess=[0,1,2,3]、colors=[null×4]、selected=0。UI：标题行「第 3 关 / 4 · 密码破解」+ 小工具栏（新一局/撤回反馈）+「当前猜测（照抄进游戏）」4 个大方框（显示符号，带 color 边框色：绿 c2/红 c0/黄 c1，点击循环 null→绿(2)→红(0)→黄(1)→null，同时设为 selected）+ 6 个符文选项按钮（点击把选中框符号改为该符文并 selected+1 取模）+「修正」按钮（4 色齐后可用：appendRecord → worker 重算 → guess 换新推荐、colors 清空、selected=0）+ 历史小列表（第N次 符文 + 色点）。worker 协议 {id, history}。
   - wheel：①「框选转轮区域」按钮（invoke('wheel_select_region')）②「开始放大镜/停止」toggle（wheel_start/wheel_stop）+ listen('wheel_frame')，<img> 显示 data URL，letter 有值时大字显示 ③ 单词输入框 + 当前字母大字 + 「已固定」推进按钮 + 完成提示。
6. **`src/window/Result/index.jsx`**：删 COUNTDOWN/count/countRef/pausedRef/graceTORef/倒计时 interval/result-drag-end 监听/onWindowPress 里 begin_result_drag/touchInteract 与 resetTimer 调用/result-countdown span；mode 相关显示删掉（规则行删、状态文案去掉"切换判色规则"措辞）；其余（框选/记录本次/撤回/新一局/推荐复制）保留，worker 消息改 {id, history}。
7. **`src/window/Config/pages/Hotkey/index.jsx`**：抽 applyDraft(name) 复用，加 hotkey_wave_scan 行（useConfig('hotkey_wave_scan','Alt+V')，t('config.hotkey.wave_scan')）。
8. **i18n**：zh_CN/zh_TW/en_US 三文件加 config.hotkey.wave_scan（"波形校准（第一关）"/"波形校準（第一關）"/"Wave Calibration (Level 1)"）；其余 locale 缺键回落 en，不动。
9. **`scripts/runes.test.mjs`**：适配新 API（无 mode/spent）；`RUNE-SOLVER.md` 与本文件结尾补验收记录。

## 三、验证命令
- `cargo test --manifest-path src-tauri/Cargo.toml wave simon wheel rune -- --nocapture`（注意记忆：cargo test 必须 --release 才能出有效 PE，跑 `cargo +stable-x86_64-pc-windows-gnu test --release ... `；快速迭代可先 cargo check）
- `node --test scripts/runes.test.mjs`；`pnpm build`
- 手动链路自测用 fixtures，不需要开游戏。

## 四、明确不做
- 不自动点击游戏（第四关 C 方案砍掉）；第三关不改 Result 自动识别窗口的录入逻辑；不做黄蓝双线增量模式（预留二期）。

## 五、验收记录（2026-11-12）

- `cargo +stable-x86_64-pc-windows-gnu test --release`（wave/simon/wheel/rune 共 10 个测试）：**10 passed, 0 failed**。唯一告警为既有的 .rsrc manifest 合并提示，与本次改动无关。
- `node --test scripts/runes.test.mjs`：count 规则反馈 + 1296 局全解直方图 [1,20,918,357] 通过。
- `pnpm build`：通过。
- 实施中的关键偏差与实测结论（相对原计划）：
  1. **第二关掩膜加 B<150 上界**：样张实测暗块填充 B≈39、边框 B≈76~122、亮起青色 B≈213、开始按钮 B≈237；不设上界时亮块填充会并掉该格两根竖边，6 竖边凑不齐。亮块判据从 min(R,G,B)>70 改为 **B 通道中位数 >140**（青色 min=R=1，原判据失效）。行间距判据改为相对瓦片高（样张 gap≈0.27×瓦片高）。
  2. **第一关拟合数值稳定化**：双频率近奇异设计矩阵令高斯消元解发散、峰位偏移（f1 1.341 vs 期望 1.4）；基波粗搜/细化改为单频 3 列方程组（sse1），联合微调改坐标下降（固定另一频率），合成样张 roundtrip 恢复通过。
  3. **第四关中心字母放弃 OCR 主判据**：WinOCR 对孤立单字母在 4~12x、双极性、双拼「词」等全部变体下必返回空串（与 runes.rs 的孤立单字结论一致）；横拼四字母行虽能出字（"BHU"/"ænHUA"）但槽位映射不稳。改为 **Arial 等无衬线系统字体 A-Z 栅格化模板 + soft(0.35)+blur(1.2)+NCC**，门限 Top-1≥0.55 且领先≥0.10；样张 H=0.569/领先 0.24。要点：游戏字体为无衬线（衬线模板字干内缩会输给 O）；紧裁后四周外扩 12%（否则 Arial 的 I 是纯实心块，NCC 无定义）；行剖面按「总 ink 最大的连续行块」取字母（剔除上方菱形指示标记）。en OCR 保留为兜底。
  4. 新增依赖 `pathfinder_geometry 0.5.1`（font-kit 栅格化 API 需要 Transform2F，font-kit 不再导出）。

---

## v1.2.1 · 四关卡集成进 Alt+C 结果窗口（2026-11-12）

> 用户拍板：「我不要关卡助手，所有东西集成进 altc 页面里面，而且风格和交互符合原 altc 页面。」独立 Runes 关卡助手窗口（680x780）整体移除，四关卡作为页签并入 result 窗口（420 宽紧凑暗色，NOACTIVATE/置顶/拖拽区交互不变）。

### 改动清单
- **删除独立窗口**：src/window/Runes/（index.jsx + style.css）删除；window.rs 的 une_window()/show_runes_window() 删除；tray 三语言「关卡助手/Level Assistant/關卡助手」菜单项删除；App.jsx 移除 runes 路由与 Escape 分支；main.rs invoke_handler 移除 window::rune_window。
- **前端集成**：Result/index.jsx 增加页签条（识别/波形/西蒙/密码/转轮），五个页面**常驻挂载**（隐藏仅 display:none），保证西蒙录制、转轮放大镜流的事件监听与状态不随页签切换丢失；四关卡页面落在 Result/pages.jsx（lvl- 前缀紧凑样式，追加进 Result/style.css）。识别页内容与交互原样保留。
- **页签尺寸**：新增 set_result_size 命令（window.rs set_result_window_size：Win32 SetWindowPos NOMOVE|NOACTIVATE|NOZORDER，显示器缩放换算物理像素，绕开 tao 隐藏窗口缩放 bug），前端切页签时调用：识别 420x360 / 波形 420x396 / 西蒙 420x430 / 密码 420x560 / 转轮 420x600。
- **emit 目标 runes→result**：wave.rs（wave_scan）、simon.rs（simon_state/simon_step）、wheel.rs（wheel_region_set/wheel_frame）全部改推 result 窗口；wave/wheel 在窗口不存在时经 get_or_create_result_window()（自主线程化建窗逻辑）先建窗再 show；wheel_select_region 改 hide_result_window()（SW_HIDE 兜底）。
- **自动切页**：识别页收到 rune_scan 自动切回「识别」；wave_scan 到达切「波形」；wheel_region_set 到达切「转轮」。
- 版本号三处 1.2.0→1.2.1。

### 验收记录
- cargo +stable-x86_64-pc-windows-gnu test --release：10/10 通过（runes/wave/simon/wheel fixtures）。
- node --test scripts/runes.test.mjs：2/2 通过。
- pnpm build：通过（chunk >500kB 警告为既有现象）。
- tauri build：exe 产物 release/GOC解题工具-1.2.1.exe（MSI 打包需联网 WiX，按惯例仅取 exe，内嵌前端单文件可用）。

---

## v1.2.2 · 删除「识别」页 + 波形画布定位重写（2026-11-12）

> 用户反馈两点：①「识别」页与「密码」页重复（同一套符文解谜），要求删掉识别相关代码；②波形识别报「未定位到波形画布区域」。

### 一、删除「识别」页
- 根因：v1.1 的「识别」页与 v1.2.0 的「密码」页是同一套符文解谜（同 worker、同会话），重复。
- Result/index.jsx 重写：页签变为 密码/波形/西蒙/转轮，默认「密码」；rune_scan payload 不再有独立展示页——8 项齐全直接 appendRecord 写入会话（与末条相同自动去重），密码页经 SESSION_EVENT 刷新并重算推荐；识别不完整不写历史，用户在密码页手动录入。撤回走密码页「撤回」。
- 同步清理：style.css 删除识别页专属样式（result-slots/slot/select/colors/status/recommend/actions/countdown 等），pages.jsx 注释中的「识别页」引用更新。

### 二、波形识别失败（根因 + 修复）
- 根因（用失败当次的真实全屏截图 2560x1440 实测复现）：旧算法用「全屏橙像素分位数」推画布横向范围。当时屏幕是 Honeyview 在看游戏截图——标题栏橙色图标、浅灰界面、深灰背板（亮度 96）、壁纸干扰使橙像素 x 分位横跨整屏（x0=28, x1=2550），y 中位被顶部垃圾拉到 175；暗行检测（sum<30 且 >带宽/2）在错误分带上 darkcnt 最大仅 563/2522 → 0 行命中 → 「未定位到波形画布区域」。真实画布在 y[627,774] x[1001,1544]（被工具窗口遮挡右缘）。
- 修复（wave.rs solve_image 重写，沿用项目既有「相对判定 + 自适应分级」经验，呼应 README 滤镜免疫与 solver_lib extract_adaptive）：
  1. 暗阈值 30→45→60 分级：行块（暗像素 > min(W*0.1, 100~250)）× 列（块内暗过半列高）围出画布候选区块；
  2. 区块内橙像素 ≥150 打分过滤（画布必含目标曲线）；
  3. 橙像素 0.3%/99.7% 分位收紧横向范围，带内暗行块（含曲线 y 中位、高 ≥40）定纵向；
  4. 列提取（三级阈值）→ 双正弦拟合 → R²≥0.5 才接受，否则记入 best_err 继续试下一候选；fit_two 的 R² 门槛上移到候选循环。
- 新增回归 fixture：tests/fixtures/level1-wave-fullscreen-overlay.png（即失败当次真实截图，含看图软件干扰 + 工具窗口遮挡画布），测试 wave_scan_fullscreen_overlay。

### 验收记录
- cargo test --release：11/11 通过（新增全屏干扰用例；真实截图求解 R²=0.887，滑块 22/79/15/44；旧样张 R²=0.891 滑块 22/83/15/44，两图同局结果一致）。
- node --test 2/2；pnpm build 通过。
- tauri build：exe 产物 release/GOC解题工具-1.2.2.exe（MSI 步骤按惯例仅取 exe）。

---

## v1.2.3 · 波形结果改为滑块可视化（2026-11-12）

> 用户反馈：「我要的是四个和系统中一样的滑块，我可以直接照抄滑块位置。」数字-only 展示需要心算对应关系，改成与游戏同构的滑块控件。

### 改动
- pages.jsx WavePage：四个 wave-row 数字行改为 wave-slider 滑块行（标签 + 轨道 + 填充 + 圆形滑钮按百分比定位 + 数值），识别成功后滑钮位置即应照抄的游戏滑块位置。
- style.css：wave-row 样式替换为 wave-slider/track/fill/thumb/val。
- 版本号三处 1.2.2→1.2.3（上一版 exe 已交付，避免混淆）。

### 验收
- pnpm build 通过；tauri build exe → release/GOC解题工具-1.2.3.exe（ProductVersion 1.2.3）。

---

## v1.2.4 · 删转轮 + 西蒙录制修复 + 波形框选 + UI 修正（2026-11-12）

> 用户反馈七点：删转轮、西蒙完全识别不到要排查修复、框选按钮去掉「框选」两字、标题改「GOC妙妙小工具v x.xx.xx」、设置-关于加版本号、框选要支持波形手动框选、密码页边框溢出/蓝底看不清绿红黄/指示不明显、点波形识别时窗口重定位一次。

### 一、删除转轮功能
- 删 src-tauri/src/wheel.rs；main.rs 去 mod 与三个命令；cmd.rs 删 wheel_* 命令。
- LAST_CUT 从 wheel.rs 移到 window.rs（cut_image 写入、波形框选读取）。
- 前端删 WheelPage / wheel 页签 / wheel_region_set 监听 / wheel-* 样式；页签变为 密码/波形/西蒙。
- 连带清理：focus_result_window 全链路删除（唯一调用方是转轮单词输入框）。

### 二、西蒙「完全识别不到」根因 + 修复
- 排查：高分辨率回归测试（样张 ×2.5 放大 2560 宽）证明行带定位本身分辨率鲁棒（先写测试证实假设不成立，排除 thr_row 塌带假说）。
- 真根因：capture_monitor_to_mem 走「PNG 编码→内存→PNG 解码」兜圈子，2560x1440 一帧多花 ~300-500ms，把名义 200ms 的采样周期拖到 600ms+；滞回需连续 2 帧亮（≈1.2s+）才记一步，游戏符号 ~0.5s 的亮起闪现在两帧确认之间结束——一步都录不上。
- 修复：solver.rs 直接取 screenshots Image 内部已转换好的 RGBA（win32 路径经 Image::from_bgra，alpha=255），按长度校验 from_raw，去掉 PNG 往返；simon.rs 睡眠 200ms→100ms，实际周期 ≈150-200ms，两帧滞回 ≈300-400ms，可可靠逮住短闪现。定位/亮起判定逻辑未动。
- 附带回归测试 simon_locate_highres 留作分辨率鲁棒性看门。

### 三、波形手动框选（框选功能支持波形）
- cmd.rs 新增 wave_select_region 命令：置 SHOT_PURPOSE="wave" → 隐藏结果窗口 → 拉全屏框选窗；window.rs success 事件按 purpose 派发到 wave::on_region_selected。
- wave.rs：on_region_selected 对 pot_screenshot_cut.png 跑同一求解管线；solve_image 增加 manual 参数，手动模式几何门槛按小图放宽（min_bh 60→40、min_cols 200→60、min_span 100→60），全屏模式不动。
- WavePage 加「框选」按钮；测试 wave_scan_manual_region（从全屏干扰样张裁 700x400 小图模拟玩家框选）。

### 四、波形识别时窗口不再重定位
- 根因：每次求解完成都调 show_result_window（内部 SetWindowPos 重定位到鼠标附近），窗口已可见时表现为「点识别窗口跳一下」。
- 修复：wave.rs notify_result 统一入口——窗口已可见则原位不动只推事件，仅在窗口不存在/隐藏时创建并显示。手动框选路径同规则。

### 五、UI 修正
- header 框选按钮去「框选」文字，纯图标（18x16，悬停 tooltip 保留）。
- 标题改「GOC妙妙小工具v{version}」，版本号从 package.json 导入（单点来源）。
- 设置-关于：名称旁加 Chip 版本号标识（同源 package.json）。
- 密码页：cipher-slots 改 minmax(0,1fr) + slot box-sizing/min-width 修复边框溢出；标色状态改 背景+边框+文字 三重着色（绿/红/黄在深蓝底上一眼可辨）；选中框加青色边框+外圈双高亮。
- 版本号三处 1.2.3→1.2.4。

### 验收
- cargo test --release：10/10 通过（删 wheel 测试后从 11→10，新增 simon_locate_highres、wave_scan_manual_region）。
- pnpm build 通过。

---

## v1.2.5 · 波形滤镜适配（相对色序判据）（2026-11-12）

> 用户用「20261112最新用例/第一关2.png」（2560x1439 真机全屏，带滤镜）做波形测试，识别失败；用户点明根因：不同玩家会叠加不同亮度/饱和度的游戏滤镜，绝对颜色会漂移，要求适配。

### 根因（样张实测）
- 画布底色 sum≈79、面板 sum≈138：旧暗阈值档 30/45/60 全部够不着画布；
- 场景大量近黑像素：扩到 90/120 后全屏连成一个行块，暗区结构法失效；
- 目标曲线被蓝调滤镜洗淡：橙像素判据（R-B>18 & R>75）在画布内命中 ≈0，曲线实测 R-B≈-1（但仍是画布内最暖的亮线，玩家的白/蓝曲线 R-B≈-77）。

### 修复（wave.rs solve_image / curve_points 重写，README「滤镜免疫」经验的推广）
- 滤镜 ≈ 全图单调变换 ⇒ 相对关系不漂：画布比面板暗、目标线是画布里「最暖的亮线」。
- 画布定位：绝对暗档 [30,60,90,120]（兜底极暗/中性画布）+ 相对档 2 档（蓝主导像素 B>R+20 的 G 中位数 ×0.3/0.4，实测画布 G15 vs 面板 G44，比率在滤镜下不变）。
- 候选打分：旧「橙像素计数」改为「暖亮像素」= 比区块内亮度 p75 亮 且 比区块暖度(R-B)中位暖 8+。
- 提线 curve_points：基线取画布内亮度/暖度中位，逐列在「亮于背景+Δ」的像素里只取「暖于基线+Δ」者（玩家的冷色曲线被排除，不再混入均值）；三级 (Δ亮,Δ暖)=(60,8)/(40,4)/(25,MIN) 从严到宽，末级退化为全部亮像素均值。
- R²≥0.5 门槛与多候选循环保持不变，作为最终安全网。
- 新增 fixture：level1-wave-real-fullscreen.png（即该用例图）+ 测试 wave_scan_real_fullscreen。

### 验收
- cargo test --release 11/11：滤镜样张解出 21/75/49/20；旧样张 24/85/14/44（比 v1.2.4 的 22/83/15/44 更准——不再混入玩家曲线）；手动框选与全屏一致；合成信号精确还原。
- 版本号 1.2.4→1.2.5，exe 产物 release/GOC解题工具-1.2.5.exe。

---

## v1.2.6 · 西蒙滤镜适配（4K 用例逐帧分析）（2026-11-12）

> 用户给第二关用例视频（4K@60fps，18.3s，带滤镜+主播摄像头），要求逐帧分析适配。

### 逐帧分析结论（60fps 逐帧实测）
- 定位失效根因：滤镜画面下边框 B≈236、填充 B≈139 全部 ≥150，被旧 is_blue（60<B<150）掩膜整体排除；且 4K 下瓦片内部行只有 6 根 2-4px 竖边（12-24px < w/150=25），行带塌成 1px 细线全被丢弃——双杀。
- 亮块时长 0.2~0.5s（最短 ~0.2s）：两帧滞回确认（≥300ms）会漏短闪。
- 未亮填充 B=135~175 不等（雷格悬停高亮偏亮 +35）：旧 tile_lit 绝对阈值 B>140 会把雷格永远判亮。
- 亮起 B=228~249，与未亮间隔雷格处仅 175→228。

### 修复（simon.rs）
- 定位掩膜：绝对蓝色区间 → **B 通道梯度边缘**（|ΔB|>24，±1 窗口）——边框本质是「面板↔填充」跳变沿，亮度/对比/饱和/色调类滤镜都不改跳变结构。
- 定位结构：行带投影 → **强线两两配对**（w/40 强线 + 自括区，列前缀和加速）：字形笔画行会插在瓦片顶/底边线之间，相邻配对围不出瓦片行（1097 样张实测插了 3 条文字线）。
- bars 竖边：桥接 2→5 列（放大/压缩相位碎片）、覆盖门槛 4/5→3/5→2/5 分级（面板晕光削最外侧竖边）、5 条时按 [边,gap,边,gap] 节距合成缺失的外侧边。
- refine_y：行覆盖门槛 55%→30%（晕光把部分横边调暗到 42%，扫描顺序保证首/末命中必为边框行）。
- 亮块判定：绝对 B>140 → **六格互比**（中心 50% B 中位数 > 六格上中位 +55；实测未亮偏差最大 +35、亮起最低 +91，双向余量 ≥20）。
- 滞回：2 帧确认 → **1 帧确认/3 帧清除**，采样睡眠 40ms（4K 周期 ≈150-250ms，0.2s 短闪必中 1 帧）。
- 新增 fixtures：level2-simon-4k-filtered.png / level2-simon-4k-lit.png（t=7.1s 土亮）+ 测试 simon_locate_4k_filtered（断言六格中心与视频实测一致 ±40px）、simon_lit_4k_filtered。

### 验收
- cargo test --release 13/13（新增 2 项 4K 用例；1097 原样张 / 2560 放大回归 / 亮样张全部保持通过）。
- 版本号 1.2.5→1.2.6，exe 产物 release/GOC解题工具-1.2.6.exe。

---

## v1.2.7 · 全局裁切模式 + 黑闪修复 + 页签重排（2026-11-12）

> 用户反馈四项：①波形框选完成后整屏黑一下；②结果窗口实际占位比可视内容大，点击窗口下方被透明区域拦截；③标题旁框选按钮改为全局裁切模式（框选做题界面后密码/波形/西蒙只识别框内画面，4K 全屏不再整体采集），要有屏幕上的明显指示，按钮切换为「取消裁切模式」；④页签改名重排（校准/西蒙/解密）+ 西蒙热键 Alt+X + 解密四绿自动重开。

### 一、黑闪修复（黑屏一下）
- 根因：框选窗 mouseUp 流程 `appWindow.close()` 销毁全屏透明 WebView2 窗口，销毁瞬间触发整屏黑闪；且 cut_image（同步命令）在主线程解码+编码 4K PNG 阻塞 1s+，拖慢整个收尾链。
- 修复：截图窗口改为**常驻复用**——框选完成只 hide() 不 close()（右键取消/Esc 同理），下次框选经 screenshot_request 复用重截（该复用路径本就存在，前端监听早已就绪）；cut_image 改 async 命令（移出主线程）。

### 二、结果窗口命中区域 = 可视区域
- 根因：窗口高度来自 TAB_SIZES 硬编码，高于实际内容时底部透明区域照样拦截点击。
- 修复：`.result-container` 去 height:100% 改内容自适应（max-height:calc(100vh-4px) 兜底内滚）；前端 ResizeObserver 量容器实际高度调 set_result_size（变化 >1px 才上报防反馈环），TAB_SIZES 删除。

### 三、全局裁切模式
- `window.rs`：`CROP_REGION`（全局物理像素,配置项 crop_region 持久化,启动 restore_crop_region 恢复）+ `SHOT_ORIGIN`（cmd::screenshot 记录框选截图所在显示器原点，用于图内坐标→全局坐标换算）；SHOT_PURPOSE 新增 "crop" 派发 → crop_selected。
- `solver.rs`：apply_crop_region——capture_fullscreen / capture_monitor_to_mem 截图后与裁切区求交集裁剪（三条管线统一生效；裁切区在另一块屏无交集时原样返回）。截图框选覆盖窗本身不裁（用户要能重新框选）。
- 指示窗：label="crop" 透明置顶点击穿透窗（set_ignore_cursor_events），窗口比裁切区四周外扩 14px，画橙色边框 + 「裁切模式 · 识别仅限框内」角标（src/window/Crop/index.jsx，复用 index.html 入口经 App.jsx windowMap 派发）。
- 结果窗口标题旁按钮：get_crop_state + listen crop_state 双通道同步状态，未激活=四角框图标（裁切模式）→ crop_select；激活=橙色实心（取消裁切模式）→ crop_clear。旧 ocr_recognize 命令删除（各入口各自置 SHOT_PURPOSE）。

### 四、页签重排 + 热键 + 自动重开
- 页签：校准(波形,Alt+V) / 西蒙(Alt+X) / 解密(密码,Alt+C)，默认页签校准。
- hotkey.rs：新增 hotkey_simon（默认 Alt+X）→ simon::simon_toggle（SIMON_RUNNING 判断开始/停止）；设置-热键页加第三行（i18n：zh_CN/zh_TW/en_US）。
- 解密页：修正写入后 encodedColors==80（四格全绿）→ 600ms 后自动 startNewGame 重开新一局。

### 验收
- cargo check 无警告（顺手删除 v1.2.6 遗留的死代码 simon::bars）；cargo test --release 13/13；pnpm build 通过。
- 版本号 1.2.6→1.2.7，exe 产物 release/GOC解题工具-1.2.7.exe。

## v1.2.8（2026-11-13）：西蒙连亮/关卡步数 + 解密定位重写 + 指示窗捕获隐身

依据用户 1.2.7 全流程实拍视频（2026-11-13_12-43-17.mkv，1080p60）逐帧分析修复。

### 一、西蒙：同格连亮被记成一步（修复）
- 实测（60fps 逐帧测量）：第三关真实序列「土土火风水风」，同格连亮起始间隔仅 450ms（亮 150ms + 灭 300ms）；不同格灭档仅 117~133ms。
- 旧「1 帧亮确认 + 3 帧暗清除」在 ~80ms 采样周期下清除需 ≈300ms，处于临界 → 合并。
- 修复：1 帧暗即清除 + 同格 300ms 不应期（真实连亮 ≥450ms 必过；过渡帧抖动发生在记录后 ~100ms 内被拦下）。

### 二、西蒙：第一关/第二关/第三关按钮 + 足额自动暂停
- 页签按钮改为 第一关/第二关/第三关/自由：前三者目标步数固定 4/5/6，录满自动暂停并提示「已录满 N 步，自动暂停」；自由 = 不限步数手动停（原行为）。
- 热键 Alt+C 切换沿用最近一次选择的目标；simon_state 增加 info 字段。

### 三、解密：识别定位必败（重写定位管线）
- 热键/按钮本身是通的，败在 locate()：彩色掩膜把面板深蓝底、青色装饰线、青色实心按钮、绿色关卡标签全部吞进（实测帧掩膜 7.3 万像素），行带黏成 399px 巨带，符文框行永远无法独立成带。
- 重写为连通域找环形边框：1px 膨胀桥接压缩断口 → 8 连通域 → 稠密包围盒剔晕点 → 环形过滤（近正方形 + 填充比 ≤0.65）→ 行聚类 + validate_quad。
- 面板深蓝底 (14,53,80) hue=204.5 恰好压进旧绿带上界 205：上界收到 196（反馈青环 187 与面板底分界中点），亮度下限 64→90。
- border_color 改取「亮度中位样本整体颜色」：逐通道中位数会拼出不存在的颜色（实测黄框 (134,140,130) diff=10 被误判白灰）。
- 新增回归样张 tests/fixtures/level3-cipher-crop-20261113.jpg（视频 t=94s 裁切区域，土风雷水/黄红黄红）。

### 四、裁切指示窗对截图隐身
- crop 覆盖窗 SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)（Win10 2004+）：橙色边框/角标不再进入截图，消除对识别管线的自污染。

### 验收
- cargo test 14/14（runes 4 + simon 5 + wave 5）；前端 build 通过；交付 release/GOC解题工具-1.2.8.exe（ProductVersion 1.2.8）。

## v1.2.9（2026-11-13）：解密策略改「候选优先、搏直接命中」

用户反馈不想要 minimax 探针步（如 空火土X 剩 4 候选时推荐 水风雷空），改为：
- choose() 只在候选（可能是答案的组合）里选，同时候选中取最坏剩余桶最小的（能区分剩局的候选天然胜出，且自带命中概率）；互相不可区分时按序猜，靠命中赌运气。
- 首猜仍固定 空火水风；移除递归可必胜验证与缓存（贪心即可）。
- 代价：最坏情况可能用满 4 次仍有残余概率失败（原策略第 3 次必中），为用户明确选择。
- 交付 release/GOC解题工具-1.2.9.exe。
