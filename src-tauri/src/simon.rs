// 第二关「西蒙符印」录制管线（v1.2.0）：
// 用户点开始 → 定位 2×3 符文格（火水风/土雷光）→ ~150-250ms 周期截屏检测亮起
// （暗→亮边沿，1 帧确认/3 帧清除）→ 逐步推送前端显示「第N步·符文名」。
// 只记边沿不适配固定次数，用户手动点停止结束录制。
// v1.2.4：截图管线去掉 PNG 编解码兜圈子后单帧 ~50-100ms。
// v1.2.5 滤镜适配（20261112 4K 用例逐帧实测）：①定位掩膜从绝对蓝色区间改为
// B 通道梯度边缘 + 强线配对（滤镜帧边框 B≈236 全在旧掩膜 B<150 之外；4K 内部
// 行会塌带）；②亮块判定改六格互比（未亮填充 B=135~175 不等，绝对阈值必错）；
// ③1 帧确认适配最短 ~0.2s 的闪亮。

use crate::solver::capture_monitor_to_mem;
use crate::APP;
use image::RgbaImage;
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

// 录制状态与已录步骤（前端挂载后经 get_simon_steps 拉取，防 emit 竞态）
static SIMON_RUNNING: AtomicBool = AtomicBool::new(false);
// 代际计数：start 不做 CAS（RUNNING 置位即启动），靠代际号让旧循环退出，
// 避免「stop→start 时间窗内旧循环读到新置位的 RUNNING」形成双循环重复记录
static SIMON_GEN: AtomicU64 = AtomicU64::new(0);
// 目标步数（0 = 自由录制不设限）：关卡按钮 4/5/6，足额自动暂停；
// 热键 toggle 沿用最近一次选择的目标
static SIMON_TARGET: AtomicUsize = AtomicUsize::new(0);
pub static SIMON_STEPS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

// 2×3 网格符文名（按格序：上排 0..2，下排 3..5）。第二关没有「空」，第 6 格是「光」
pub const SIMON_NAMES: [&str; 6] = ["火", "水", "风", "土", "雷", "光"];

// 步骤互斥锁的容错获取：持锁线程 panic 后中毒时恢复内部数据继续服务，
// 避免连锁 panic 让录制线程静默死亡
fn steps_lock() -> std::sync::MutexGuard<'static, Vec<Value>> {
    SIMON_STEPS.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy)]
pub struct Tile {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

// 关卡按钮/热键入口：target = 4/5/6（第一/二/三关，录满自动暂停）；0 = 自由录制
pub fn simon_start_with_target(target: usize) {
    SIMON_TARGET.store(target, Ordering::SeqCst);
    simon_start_impl();
}

pub fn simon_stop() {
    simon_stop_impl();
}

// 热键入口（默认 Alt+C）：开始/停止录制切换，正在录制时停止，否则以最近一次
// 选择的目标步数开始（关卡按钮设置过 4/5/6 则沿用，未设置过 = 自由录制）
pub fn simon_toggle() {
    if SIMON_RUNNING.load(Ordering::SeqCst) {
        simon_stop_impl();
    } else {
        let target = SIMON_TARGET.load(Ordering::SeqCst);
        simon_start_with_target(target);
    }
}

pub fn get_simon_steps() -> Vec<Value> {
    steps_lock().clone()
}

// 录制运行状态（前端挂载后拉取，防 start 的 emit 早于监听注册的竞态：
// 拉到 running=true 时前端自行切到西蒙页签）
pub fn simon_running() -> bool {
    SIMON_RUNNING.load(Ordering::SeqCst)
}

pub fn simon_start_impl() {
    // 递增代际号：正在跑的旧 record_loop 检测到代际不符即自行退出
    let gen = SIMON_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    steps_lock().clear();
    SIMON_RUNNING.store(true, Ordering::SeqCst);
    // 热键触发时窗口可能尚未创建/隐藏：先拉起并显示（首次定位鼠标附近，
    // 之后原位），再推 simon_state——前端收到 running=true 切到西蒙页签。
    // 首帧 emit 可能早于前端监听注册，挂载后的 get_simon_state 拉取兜底
    if let Some(app) = APP.get().cloned() {
        let _ = app.run_on_main_thread(move || {
            crate::window::get_or_create_result_window();
            crate::window::show_result_window();
        });
    }
    emit_state(true, 0, None, None);
    std::thread::spawn(move || {
        record_loop(gen);
    });
}

pub fn simon_stop_impl() {
    SIMON_RUNNING.store(false, Ordering::SeqCst);
    let total = steps_lock().len();
    emit_state(false, total, None, None);
    info!("[simon] 录制结束，共 {} 步", total);
}

fn emit_state(running: bool, total: usize, error: Option<&str>, info: Option<&str>) {
    if let Some(app) = APP.get().cloned() {
        let payload = json!({ "running": running, "total": total, "error": error, "info": info });
        let _ = app.clone().run_on_main_thread(move || {
            use tauri::Manager;
            if let Some(w) = app.get_window("result") {
                let _ = w.emit("simon_state", payload);
            }
        });
    }
}

fn record_loop(gen: u64) {
    // 本循环是否仍然有效：RUNNING 且代际号未被新一次 start 递增
    fn alive(gen: u64) -> bool {
        SIMON_RUNNING.load(Ordering::SeqCst) && SIMON_GEN.load(Ordering::SeqCst) == gen
    }

    // 1) 首次定位网格：失败重试 ~10 帧（≈2s），仍失败则停
    let tiles = {
        let mut found = None;
        for i in 0..10 {
            if !alive(gen) {
                return;
            }
            match capture_monitor_to_mem() {
                Ok(img) => match locate_grid(&img) {
                    Ok(t) => {
                        found = Some(t);
                        break;
                    }
                    Err(e) => info!("[simon] 定位失败({}/10): {}", i + 1, e),
                },
                Err(e) => info!("[simon] 截图失败({}/10): {}", i + 1, e),
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        match found {
            Some(t) => t,
            None => {
                warn!("[simon] 未定位到 2×3 符文格，录制停止");
                emit_state(false, 0, Some("未定位到符文格，请确认停在第二关界面后重试"), None);
                SIMON_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        }
    };
    info!("[simon] 网格定位完成: {:?}", tiles);

    // 2) 亮起检测（v1.2.8 重写）：1 帧亮确认 + 1 帧暗清除 + 同格 300ms 不应期。
    //    旧「3 帧暗才清除」在实测中合并连亮：20261113 1080p 用例逐帧测量，第三关
    //    真实序列「土土火风水风」，同格连亮起始间隔仅 450ms（亮 150ms + 灭 300ms），
    //    采样周期 ~80ms 时 3 帧暗 ≈ 300ms 起才能清除——临界合并。改 1 帧暗即清除
    //    后，同格闪亮的过渡帧/编码抖动可能把一次亮拆成两次，由 300ms 不应期兜住
    //    （过渡抖动发生在记录后 ~100ms 内，真实连亮 ≥450ms）。
    let mut lit_state = [false; 6];
    let mut last_rec = [None; 6]; // 每格最近一次记录时刻（不应期用）
    const REFRACTORY: std::time::Duration = std::time::Duration::from_millis(300);
    let mut seq = 0usize;
    let target = SIMON_TARGET.load(Ordering::SeqCst);
    // 连续截图失败上限：显示器休眠/会话源丢失等场景不能 20ms 空转不停，
    // 超限自动停止并上报错误（定位阶段有 10 次上限，帧循环同样要有）
    const FAIL_LIMIT: u32 = 25;
    let mut fail_run = 0u32;
    let sleep = std::time::Duration::from_millis(20);
    while alive(gen) {
        std::thread::sleep(sleep);
        let img = match capture_monitor_to_mem() {
            Ok(v) => {
                fail_run = 0;
                v
            }
            Err(e) => {
                fail_run += 1;
                info!("[simon] 帧截图失败({}/{}): {}", fail_run, FAIL_LIMIT, e);
                if fail_run >= FAIL_LIMIT {
                    warn!("[simon] 连续截图失败，录制停止");
                    let total = steps_lock().len();
                    emit_state(
                        false,
                        total,
                        Some("连续截图失败，录制已停止，请确认游戏画面可见后重试"),
                        None,
                    );
                    SIMON_RUNNING.store(false, Ordering::SeqCst);
                    return;
                }
                continue;
            }
        };
        let meds = frame_meds(&img, &tiles);
        let lits = lit_from_meds(&meds);
        for i in 0..tiles.len() {
            let now_lit = lits[i];
            if now_lit && !lit_state[i] {
                // 上升沿：先查不应期——真实连亮 ≥450ms 间隔必过，过渡帧抖动
                // （发生在记录后 ~100ms 内）被拦下，不会把一次亮拆成两步
                let now = std::time::Instant::now();
                if last_rec[i].is_some_and(|t| now.duration_since(t) < REFRACTORY) {
                    // 被不应期拦下的上升沿视为「仍是同一次亮」：锁存亮态，否则
                    // 亮持续 >300ms 且中途混入暗毛刺时，过期后会再触发上升沿
                    // 把同一次亮拆成两步
                    lit_state[i] = true;
                    continue;
                }
                // 写入前复核代际：防止新 start 已清空步骤后旧循环再补入脏数据
                if !alive(gen) {
                    return;
                }
                last_rec[i] = Some(now);
                lit_state[i] = true;
                seq += 1;
                let step = json!({ "seq": seq, "index": i, "name": SIMON_NAMES[i] });
                steps_lock().push(step.clone());
                // 写入后复核代际：stop→start 已清空步骤的话，回滚这条脏数据
                if !alive(gen) {
                    steps_lock().pop();
                    return;
                }
                info!("[simon] 第{}步 {}", seq, SIMON_NAMES[i]);
                if let Some(app) = APP.get().cloned() {
                    let _ = app.clone().run_on_main_thread(move || {
                        use tauri::Manager;
                        if let Some(w) = app.get_window("result") {
                            let _ = w.emit("simon_step", step);
                        }
                    });
                }
                // 足额自动暂停（关卡模式 4/5/6）：录满即停，提示用户照抄
                if target > 0 && seq >= target {
                    info!("[simon] 已录满 {} 步，自动暂停", target);
                    let msg = format!("已录满 {} 步，自动暂停", target);
                    emit_state(false, seq, None, Some(&msg));
                    SIMON_RUNNING.store(false, Ordering::SeqCst);
                    return;
                }
            } else if !now_lit && lit_state[i] {
                // 1 帧暗即清除：同格连亮的 300ms 灭档足以触发下一轮上升沿
                lit_state[i] = false;
            }
        }
    }
}

// ---------------- 网格定位 ----------------

// B 通道梯度边缘掩膜：瓦片边框无论被滤镜调成什么颜色，本质都是「面板底色 ↔
// 瓦片填充」的跳变沿。旧 is_blue（60<B<150 绝对区间）在滤镜画面下把边框
// (B≈236) 和填充 (B≈139) 整体排除在掩膜外（20261112 4K 用例实测），定位必败。
// 亮度/对比/饱和/色调类滤镜都是全图单调变换，不改变跳变结构——一阶差分即可
// 通吃。阈值 24：实测滤镜帧边框梯度 40~110、旧样张 35~73、视频压缩噪点 <20。
fn edge_mask(img: &RgbaImage) -> Vec<bool> {
    let (w, h) = img.dimensions();
    // 直接读底层字节切片（RGBA4 通道交错），避免逐像素 get_pixel 的边界检查开销
    let raw = img.as_raw();
    let mut b = vec![0i32; (w * h) as usize];
    for (i, chunk) in raw.chunks_exact(4).enumerate() {
        b[i] = chunk[2] as i32;
    }
    let at = |x: u32, y: u32| b[(y * w + x) as usize];
    let mut mask = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            // ±1 窗口：旧样张边框仅 1~2px，窗口大了会把跳变摊薄到阈值以下
            let gx = if x >= 1 && x + 1 < w {
                (at(x + 1, y) - at(x - 1, y)).abs()
            } else {
                0
            };
            let gy = if y >= 1 && y + 1 < h {
                (at(x, y + 1) - at(x, y - 1)).abs()
            } else {
                0
            };
            mask[(y * w + x) as usize] = gx > 24 || gy > 24;
        }
    }
    mask
}

// 强边缘行聚线：行内边缘像素 ≥ w/40（瓦片顶边横跨 ~3×瓦片宽 ≈ 27% 图宽；
// 瓦片内部每行只有 6 根竖边 ≈ 0.5% 图宽）——把瓦片行的上/下边框提取成独立
// 细线带，相邻线带配对围出瓦片行区间，交给 row_tiles 找 3 框。
// 旧 bands() 要求瓦片内部行也过行阈值（w/150）：4K 下内部行只有 6 根 2~4px
// 竖边（12~24px）< 25，行带塌成 1px 线后全部因带高 <10 被丢弃——对分辨率
// 脆弱。强线只落在边框上，与分辨率无关。
fn strong_lines(mask: &[bool], w: u32, h: u32) -> Vec<(u32, u32)> {
    let thr = (w / 40).max(8);
    let mut out = Vec::new();
    let mut start: Option<u32> = None;
    let mut prev = 0u32;
    for y in 0..h {
        let row = (y * w) as usize;
        let c = mask[row..row + w as usize].iter().filter(|&&v| v).count() as u32;
        if c >= thr {
            if start.is_none() {
                start = Some(y);
            }
            prev = y;
        } else if let Some(s) = start {
            if y > prev + 3 {
                // 线带内允许 ≤3 行断续（边框抗锯齿 / 视频压缩模糊）
                out.push((s, prev));
                start = None;
            }
        }
    }
    if let Some(s) = start {
        out.push((s, prev));
    }
    out
}

// 列前缀和（扁平 (h+1)*w）：任意行区间内每列的掩膜计数 O(1) 可查，
// 供强线两两配对时快速求 bars（4K 全帧配对数可能上百，直接逐格累计太慢）
fn col_prefix(mask: &[bool], w: u32, h: u32) -> Vec<u32> {
    let mut ps = vec![0u32; ((h + 1) * w) as usize];
    for y in 0..h {
        let (row, prev) = (((y + 1) * w) as usize, (y * w) as usize);
        for x in 0..w as usize {
            ps[row + x] = ps[prev + x] + mask[prev + x] as u32;
        }
    }
    ps
}

// 行区间 [y0,y1] 内的列掩膜计数 ≥ thr 聚竖边（允许 5 列断续桥接：
// 放大/压缩会让同一边框的梯度带按相位碎成几段，中间出现 3-4 列低谷）
fn bars_ps(ps: &[u32], w: u32, y0: u32, y1: u32, thr: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut start: Option<u32> = None;
    let mut miss = 0u32;
    for x in 0..w {
        let c = ps[((y1 + 1) * w + x) as usize] - ps[(y0 * w + x) as usize];
        if c >= thr {
            if start.is_none() {
                start = Some(x);
            }
            miss = 0;
        } else if start.is_some() {
            miss += 1;
            if miss > 5 {
                out.push((start.unwrap(), x - miss));
                start = None;
                miss = 0;
            }
        }
    }
    if let Some(s) = start {
        out.push((s, w - 1 - miss));
    }
    out
}

// 带内收紧上下边：行覆盖率 ≥ 30% 框宽的首/末行。门槛取低是因为面板晕光会把
// 部分瓦片的横边梯度调暗（高分辨样张实测瓦片2顶边覆盖率仅 42%）；而扫描
// 从括区两端开始，首/末命中的必然是顶/底边框行（箱内文字笔画行覆盖率虽也
// 可能过阈，但只在两端边框行全部失守时才会被误取，且随后近方形校验会拒绝）。
fn refine_y(mask: &[bool], w: u32, x0: u32, x1: u32, y0: u32, y1: u32) -> Option<(u32, u32)> {
    let need = (((x1 - x0 + 1) * 3 / 10) as u32).max(4);
    let count = |y: u32| {
        let row = (y * w) as usize;
        mask[row + x0 as usize..row + x1 as usize + 1]
            .iter()
            .filter(|&&v| v)
            .count() as u32
    };
    let top = (y0..=y1).find(|&y| count(y) >= need)?;
    let bottom = (y0..=y1).rev().find(|&y| count(y) >= need)?;
    (bottom > top).then_some((top, bottom))
}

// 单行 3 框的几何校验：尺寸互差 ≤1/3、近正方形 0.6~1.7、节距均匀 ≤1/3、
// 框宽/节距 ∈ [0.3, 0.85]（亮块是实心填充，不放 ring_contrast）
fn validate_row(win: &[Tile; 3]) -> bool {
    let (wmin, wmax) = win.iter().fold((u32::MAX, 0u32), |(lo, hi), r| (lo.min(r.w), hi.max(r.w)));
    // wmin + wmin/3 与 wmin*4/3 在整数域恒等（floor(4n/3)=n+floor(n/3)），只留一个
    if wmax > wmin + wmin / 3 {
        return false;
    }
    let (hmin, hmax) = win.iter().fold((u32::MAX, 0u32), |(lo, hi), r| (lo.min(r.h), hi.max(r.h)));
    if hmax > hmin * 4 / 3 {
        return false;
    }
    let ratio = wmax as f32 / hmax as f32;
    if !(0.6..=1.7).contains(&ratio) {
        return false;
    }
    let mut pitches = [0i64; 2];
    for i in 0..2 {
        let p = win[i + 1].x as i64 - win[i].x as i64;
        if p <= 0 {
            return false;
        }
        pitches[i] = p;
    }
    let (pmin, pmax) = pitches.iter().fold((i64::MAX, 0i64), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    if pmax > pmin + pmin / 3 {
        return false;
    }
    let pw = pmin as f32;
    for r in win.iter() {
        if !(0.3..=0.85).contains(&(r.w as f32 / pw)) {
            return false;
        }
    }
    true
}

// 行区间内找 3 框组：每 6 条竖边窗口 → 3 框 → 校验。
// ps 为列前缀和，mask 供 refine_y 小范围复查
fn row_tiles(
    ps: &[u32],
    mask: &[bool],
    w: u32,
    by0: u32,
    by1: u32,
) -> Option<[Tile; 3]> {
    let band_h = by1 - by0 + 1;
    if band_h < 10 {
        return None;
    }
    // 竖边覆盖门槛分级放宽：面板晕光会把最外侧瓦片的竖边梯度削到 80% 之下
    //（高分辨样张实测土格左边只有 5 条 bar/6）——几何校验兜底防误配
    for mult in [4u32, 3, 2] {
        let mut bars = bars_ps(ps, w, by0, by1, (band_h * mult / 5).max(3));
        // 恰好 5 条竖边：最外侧边可能被晕光整条吃掉。按 [边,gap,边,gap,边] 的
        // 节距结构推断缺失端并合成（左右两种模式，箱宽/间隙一致性校验）
        if bars.len() == 5 {
            let wmid_a = (bars[2].1 - bars[1].0) as i64;
            let wmid_b = (bars[4].1 - bars[3].0) as i64;
            let gap_a = (bars[1].0 - bars[0].1) as i64;
            let gap_b = (bars[3].0 - bars[2].1) as i64;
            let near = |a: i64, b: i64, tol: i64| (a - b).abs() <= tol;
            if near(wmid_a, wmid_b, wmid_a / 4) && near(gap_a, gap_b, gap_a / 2 + 1) {
                // 缺左端：[R1, L2, R2, L3, R3]
                let l1 = bars[0].0 as i64 - wmid_a;
                if l1 >= 0 {
                    bars.insert(0, (l1 as u32, l1 as u32));
                }
            } else {
                let wl = (bars[1].1 - bars[0].0) as i64;
                let wr = (bars[3].1 - bars[2].0) as i64;
                let g1 = (bars[2].0 - bars[1].1) as i64;
                let g2 = (bars[4].0 - bars[3].1) as i64;
                if near(wl, wr, wl / 4) && near(g1, g2, g1 / 2 + 1) {
                    // 缺右端：[L1, R1, L2, R2, L3]
                    let r3 = bars[4].1 as i64 + wr;
                    if r3 < w as i64 {
                        bars.push((r3 as u32, r3 as u32));
                    }
                }
            }
        }
        if bars.len() < 6 {
            continue;
        }
        for win in bars.windows(6) {
            let mut rects = [Tile { x: 0, y: 0, w: 0, h: 0 }; 3];
            let mut ok = true;
            for k in 0..3 {
                let (x0, x1) = (win[k * 2].0, win[k * 2 + 1].1);
                let ys = if x1 > x0 {
                    refine_y(mask, w, x0, x1, by0, by1)
                } else {
                    None
                };
                match ys {
                    Some((y0, y1)) => {
                        rects[k] = Tile { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 }
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && validate_row(&rects) {
                return Some(rects);
            }
        }
    }
    None
}

// 2×3 网格定位：强线两两配对围行 → 各行找 3 框 → 两行配对（x 中心逐列对齐
// ±15%、行高相近、行间距 ∈ (0.1h, 1.2h)，重叠行因 gap=0 天然被拒）。
// 返回上排 0..2、下排 3..5。
// 配对不能只取相邻线对：字形笔画行也是强线，会插在瓦片顶/底边线之间，把
// 括区切碎（旧 fixtures 上实测瓦片行内插了 3 条文字线）——对每条上线 i 依次
// 尝试下方各线 j，首 个通过 row_tiles 几何校验的括区即瓦片行；行高超过 w/3
// 的括区直接放弃（瓦片高 ≈ 0.09 图宽，上限留足余量）。
pub fn locate_grid(img: &RgbaImage) -> Result<[Tile; 6], String> {
    let (w, h) = img.dimensions();
    let mask = edge_mask(img);
    let lines = strong_lines(&mask, w, h);
    let ps = col_prefix(&mask, w, h);
    let cap = (w / 3) as i64;
    let mut rows: Vec<[Tile; 3]> = Vec::new();
    for (i, &li) in lines.iter().enumerate() {
        // 自括区：低分辨率/文字线密集时，瓦片行内部行也会过强线阈值并进同一条
        // 厚线带——此时厚带本身就是完整瓦片行（如 lit 样张 811 宽实测 (209,312)）
        if li.1 as i64 - li.0 as i64 >= 40 && li.1 as i64 - li.0 as i64 <= cap {
            if let Some(r) = row_tiles(&ps, &mask, w, li.0, li.1) {
                rows.push(r);
            }
        }
        for &lj in lines.iter().skip(i + 1) {
            let (by0, by1) = (li.0, lj.1);
            let bh = by1 as i64 - by0 as i64;
            if bh < 40 {
                continue;
            }
            if bh > cap {
                break;
            }
            if let Some(r) = row_tiles(&ps, &mask, w, by0, by1) {
                rows.push(r);
                break;
            }
        }
    }
    for (ia, a) in rows.iter().enumerate() {
        for b in rows.iter().skip(ia + 1) {
            // 逐列 x 中心对齐（±15% 瓦片宽）
            let tw = a.iter().map(|t| t.w).min().unwrap_or(1) as f32;
            let aligned = (0..3).all(|k| {
                let ca = (a[k].x + a[k].w / 2) as f32;
                let cb = (b[k].x + b[k].w / 2) as f32;
                (ca - cb).abs() <= 0.15 * tw
            });
            if !aligned {
                continue;
            }
            let (hmin, hmax) = a
                .iter()
                .chain(b.iter())
                .fold((u32::MAX, 0u32), |(lo, hi), t| (lo.min(t.h), hi.max(t.h)));
            if hmax > hmin * 4 / 3 {
                continue;
            }
            let gap = (b[0].y as i64 - (a[0].y + a[0].h) as i64).max(0) as f32;
            // 行间距相对瓦片高：样张 gap≈0.27×瓦片高
            let th = a.iter().map(|t| t.h).min().unwrap_or(1) as f32;
            if !(0.1 * th..=1.2 * th).contains(&gap) {
                continue;
            }
            let mut out = [Tile { x: 0, y: 0, w: 0, h: 0 }; 6];
            out[..3].copy_from_slice(a);
            out[3..].copy_from_slice(b);
            return Ok(out);
        }
    }
    Err("未定位到 2×3 符文格".into())
}

// 亮块检测（相对判定）：取六格中心 50% 区域的 B 通道中位数，亮格 = 显著高于
// 六格中位数。绝对阈值（旧 B>140）在滤镜画面下失效：4K 用例实测未亮填充
// B=135~175（雷格因悬停高亮偏亮 35+），亮起 B=228~249——B>140 会把雷格永远
// 判亮。滤镜是全图单调变换，六格互比的中位数随之同步漂移，相对差不变。
// 亮起是高亮青色实心（旧样张 (1,156,213)、滤镜帧 B≈240），字形灰阶只占少数
// 像素，中位数仍落在填充色上。
fn frame_meds(img: &RgbaImage, tiles: &[Tile; 6]) -> [u8; 6] {
    let mut meds = [0u8; 6];
    for (i, t) in tiles.iter().enumerate() {
        let x0 = t.x + t.w / 4;
        let x1 = t.x + t.w * 3 / 4;
        let y0 = t.y + t.h / 4;
        let y1 = t.y + t.h * 3 / 4;
        let mut vals: Vec<u8> = Vec::new();
        if x1 > x0 && y1 > y0 {
            // 4K 瓦片中心 175×175，隔行采样把单帧六格开销压到 ~3 万像素
            let sx = ((x1 - x0) / 80).max(1);
            let sy = ((y1 - y0) / 80).max(1);
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    vals.push(img.get_pixel(x, y).0[2]);
                    x += sx;
                }
                y += sy;
            }
        }
        vals.sort_unstable();
        meds[i] = if vals.is_empty() { 0 } else { vals[vals.len() / 2] };
    }
    meds
}

// 六格互比 → 亮起位图。阈值 +55：滤镜帧未亮偏差最大 +35（雷 172 vs 中位 137）、
// 亮起最低 +91（228 vs 137）；旧样张未亮 39 / 亮 213。余量双向 ≥20。
// 基线取排序后次小值 s[1]：播放期同时亮的 ≤3 格 → 前 3 个值必为暗格，s[1]
// 恒为暗且是「典型暗值」。旧取 s[3]（上中位）在恰好 3 格同亮时被亮格拉高
// （s[3]=最小亮值，亮格极差 ~21 < 阈值 55 → 整帧漏判）；取 s[2]（最大暗值）
// 则会被雷格悬停高亮（172）压到亮侧余量只剩 ~1。s[1] 双向余量最均衡：
// 暗侧（悬停 172 vs s[1]≈135 → +37 < 55）、亮侧（228 vs s[1]≈135 → +91 > 55）。
fn lit_from_meds(meds: &[u8; 6]) -> [bool; 6] {
    let mut s = *meds;
    s.sort_unstable();
    let med = s[1] as i32; // 次小值：≤3 亮时恒为暗格，且免疫单格悬停高亮/噪点
    let mut out = [false; 6];
    for (i, &m) in meds.iter().enumerate() {
        out[i] = (m as i32) > med + 55;
    }
    out
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    // 样张：2×3 符文格（火水风/土雷光），应定位到 6 框
    #[test]
    fn simon_locate_fixture() {
        let img = image::load_from_memory(include_bytes!(
            "../tests/fixtures/level2-simon.png"
        ))
        .expect("样张解码失败")
        .to_rgba8();
        let tiles = locate_grid(&img).expect("样张上定位失败");
        assert_eq!(tiles.len(), 6);
        for (i, t) in tiles.iter().enumerate() {
            println!("格{} rect=({},{},{}x{})", i, t.x, t.y, t.w, t.h);
            assert!(t.w >= 10 && t.h >= 10, "格{}尺寸异常: {:?}", i, t);
        }
        // 上排 y 中心应小于下排
        assert!(tiles[0].y < tiles[3].y, "上下排顺序不对: {:?}", tiles);
    }

    // 高分辨率回归（用户真实 2560 宽屏上定位必败的根因）：样张 ×2.5 放大到
    // 2560 宽。旧版行带阈值 = 图宽/150 = 17，而瓦片内部每行只有 6 根 2~3px
    // 竖边（12~18px），行带碎成 1px 细线后全部因带高 <10 被丢弃——定位对
    // 分辨率脆弱。修复后判定必须与分辨率无关。
    #[test]
    fn simon_locate_highres() {
        let img = image::load_from_memory(include_bytes!(
            "../tests/fixtures/level2-simon.png"
        ))
        .expect("样张解码失败")
        .to_rgba8();
        let big = image::DynamicImage::ImageRgba8(img)
            .resize_exact(2560, 1760, image::imageops::FilterType::Nearest)
            .to_rgba8();
        let tiles = locate_grid(&big).expect("2560 宽下定位失败（分辨率脆弱）");
        assert_eq!(tiles.len(), 6);
        assert!(tiles[0].y < tiles[3].y, "上下排顺序不对: {:?}", tiles);
    }

    // 亮起样张：第 5 格（雷, index 4）应为亮，其余为暗
    #[test]
    fn simon_lit_fixture() {
        let img = image::load_from_memory(include_bytes!(
            "../tests/fixtures/level2-simon-lit.png"
        ))
        .expect("样张解码失败")
        .to_rgba8();
        let tiles = locate_grid(&img).expect("样张上定位失败");
        let meds = frame_meds(&img, &tiles);
        let lit: Vec<usize> = lit_from_meds(&meds)
            .iter()
            .enumerate()
            .filter(|(_, &v)| v)
            .map(|(i, _)| i)
            .collect();
        println!("亮起格: {:?} meds={:?}（期望 [4] 雷）", lit, meds);
        assert_eq!(lit, vec![4], "亮块判别不符样张实际内容");
    }

    // 4K 滤镜视频帧（20261112 用例，待机帧）：亮蓝滤镜下边框 B≈236、填充 B≈139
    // 全在旧 is_blue(B<150) 掩膜之外，旧版完全无法定位。梯度掩膜应照常定位，
    // 且整屏蓝色装饰线/主播摄像头等干扰不产生误配对。
    #[test]
    fn simon_locate_4k_filtered() {
        let img = image::load_from_memory(include_bytes!(
            "../tests/fixtures/level2-simon-4k-filtered.png"
        ))
        .expect("样张解码失败")
        .to_rgba8();
        let tiles = locate_grid(&img).expect("4K 滤镜帧上定位失败");
        // 视频实测：列中心 ≈1471/1920/2370，行中心 ≈995/1445（3840x2160）
        let expect = [
            (1471, 995),
            (1920, 995),
            (2370, 995),
            (1471, 1445),
            (1920, 1445),
            (2370, 1445),
        ];
        for (i, t) in tiles.iter().enumerate() {
            let (cx, cy) = (t.x + t.w / 2, t.y + t.h / 2);
            println!("格{} rect=({},{},{}x{}) 期望中心 {:?}", i, t.x, t.y, t.w, t.h, expect[i]);
            assert!(
                (cx as i64 - expect[i].0 as i64).abs() <= 40
                    && (cy as i64 - expect[i].1 as i64).abs() <= 40,
                "格{}中心 {:?} 偏离期望 {:?}",
                i,
                (cx, cy),
                expect[i]
            );
        }
        // 待机帧：六格互比不应有亮块（雷格悬停高亮 +35 也不该触发 +55 阈值）
        let meds = frame_meds(&img, &tiles);
        let lit = lit_from_meds(&meds);
        println!("待机帧 meds={:?} lit={:?}", meds, lit);
        assert!(lit.iter().all(|&v| !v), "待机帧不应有亮块: {:?}", meds);
    }

    // 4K 滤镜视频帧（t=7.1s）：土(index 3) 亮起 B≈249，其余未亮 135~175
    //（雷格悬停高亮 172）——相对判定应唯一判出土亮
    #[test]
    fn simon_lit_4k_filtered() {
        let img = image::load_from_memory(include_bytes!(
            "../tests/fixtures/level2-simon-4k-lit.png"
        ))
        .expect("样张解码失败")
        .to_rgba8();
        let tiles = locate_grid(&img).expect("4K 滤镜帧上定位失败");
        let meds = frame_meds(&img, &tiles);
        let lit: Vec<usize> = lit_from_meds(&meds)
            .iter()
            .enumerate()
            .filter(|(_, &v)| v)
            .map(|(i, _)| i)
            .collect();
        println!("亮起格: {:?} meds={:?}（期望 [3] 土）", lit, meds);
        assert_eq!(lit, vec![3], "亮块判别不符视频实际内容");
    }

    // 3 格同亮合成用例（设计假设内边界，旧 s[3] 基线在此整帧漏判）：
    // 暗格 135~150 + 悬停高亮 172，亮格 228~249
    #[test]
    fn simon_lit_three_simultaneous() {
        let assert_lit = |meds: &[u8; 6]| -> Vec<usize> {
            lit_from_meds(meds)
                .iter()
                .enumerate()
                .filter(|(_, &v)| v)
                .map(|(i, _)| i)
                .collect()
        };
        // 2 亮 3 暗 1 悬停：亮 = 格4、格5
        let meds = [135, 140, 150, 172, 228, 249];
        assert_eq!(assert_lit(&meds), vec![4, 5], "亮格判别错误: {:?}", meds);
        // 恰好 3 亮（乱序排列）+ 悬停在暗格：亮 = 格0、格2、格4
        let meds = [228, 135, 240, 172, 249, 150];
        assert_eq!(assert_lit(&meds), vec![0, 2, 4], "3 亮被暗基线拉高误判: {:?}", meds);
        // 待机帧（0 亮）：最高暗值（含悬停 175）不得误判
        let meds = [135, 140, 150, 160, 172, 175];
        assert!(assert_lit(&meds).is_empty(), "待机帧误判亮块: {:?}", meds);
    }
}
