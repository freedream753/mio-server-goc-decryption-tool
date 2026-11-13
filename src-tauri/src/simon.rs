// 第二关「西蒙符印」录制管线（v1.2.0）：
// 用户点开始 → 定位 2×3 符文格（火水风/土雷光）→ ~150-250ms 周期截屏检测亮起
// （暗→亮边沿，1 帧确认/3 帧清除）→ 逐步推送前端显示「第N步·符文名」。
// 只记边沿不适配固定次数，用户手动点停止结束录制。
// v1.2.4：截图管线去掉 PNG 编解码兜圈子后单帧 ~50-100ms。
// v1.2.5 滤镜适配（20261112 4K 用例逐帧实测）：①定位掩膜从绝对蓝色区间改为
// B 通道梯度边缘 + 强线配对（滤镜帧边框 B≈236 全在旧掩膜 B<150 之外；4K 内部
// 行会塌带）；②亮块判定改六格互比（未亮填充 B=135~175 不等，绝对阈值必错）；
// ③1 帧确认适配最短 ~0.2s 的闪亮。

use crate::solver::{capture_monitor_to_mem, ocr, pad_canvas, to_png};
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
// 目标步数（0 = 自由录制不设限）：仅自由模式入口传 0；原三关式 4/5/6 已由
// 自动模式（状态区 OCR 相位机）取代，target 参数保留兼容 free 按钮
static SIMON_TARGET: AtomicUsize = AtomicUsize::new(0);
pub static SIMON_STEPS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

// 2×3 网格符文名（按格序：上排 0..2，下排 3..5）。第二关没有「空」，第 6 格是「光」
pub const SIMON_NAMES: [&str; 6] = ["火", "水", "风", "土", "雷", "光"];

// ---------------- 自动模式相位 ----------------
// 状态机由面板底部状态区文字驱动（系统 WinOCR，沿用 solver::ocr 管线）：
// 「开始」按钮 → 待机；「正在播放...」→ 清空步骤重新采集；「轮次X/3·请复现顺序」
// → 按轮次 4/5/6 步截断并停止采集（玩家复现的按压亮起不计入，防止误算）；
// 「错误！点击开始重试」→ 清空步骤回到待机；文字回到「正在播放」→ 下一轮重新采集。
const PHASE_STANDBY: usize = 0; // 待机：等待玩家点「开始」
const PHASE_PLAYING: usize = 1; // 「正在播放...」：采集亮起
const PHASE_REPRODUCE: usize = 2; // 「轮次X/3」：停止采集等玩家复现
static SIMON_PHASE: AtomicUsize = AtomicUsize::new(PHASE_STANDBY);

// 状态文字分类结果（Unknown = 都没匹配上，下轮强制重 OCR 直到读出已知状态）
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Standby,
    Playing,
    Reproduce,
    Error,
    Unknown,
}

// 关键词子串匹配（容忍 OCR 噪声；WinOCR 常在字间插空格，先去空白再匹配）。
// 顺序敏感：「错误！点击开始重试」含「开始」，必须先判「错误」；轮次号取
// 「数字+紧跟 /」的分子——「轮次X/3」的分母 3 恒在分子之后，直接找首个数字
// 会把分母当轮次（如 OCR 漏掉分子时的「轮次/3」）
fn classify_status(text: &str) -> (Phase, usize) {
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.contains("正在播放") {
        return (Phase::Playing, 0);
    }
    if clean.contains("轮次") {
        let chars: Vec<char> = clean.chars().collect();
        let round = chars
            .windows(2)
            .find_map(|w| {
                if w[1] == '/' && w[0].is_ascii_digit() {
                    w[0].to_digit(10).map(|d| d as usize)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        return (Phase::Reproduce, round);
    }
    if clean.contains("错误") {
        return (Phase::Error, 0);
    }
    if clean.contains("开始") {
        return (Phase::Standby, 0);
    }
    (Phase::Unknown, 0)
}

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

// 热键入口（默认 Alt+C）：开始/停止录制切换，正在录制时停止，否则进自动模式
// （状态文字驱动，见 auto_loop 注释）
pub fn simon_toggle() {
    if SIMON_RUNNING.load(Ordering::SeqCst) {
        simon_stop_impl();
    } else {
        if crate::window::crop_region().is_none() {
            // v1.4.0 识别只认裁切区域：热键未开裁切时先拉框选，完成后自动进自动模式
            crate::window::begin_crop_selection("simon");
        } else {
            simon_start_auto();
        }
    }
}

// 自动模式入口：切到西蒙页签 / Alt+C 热键自动启动
pub fn simon_start_auto() {
    SIMON_PHASE.store(PHASE_STANDBY, Ordering::SeqCst);
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
    emit_state(true, 0, None, Some("待机：等待玩家点「开始」"));
    std::thread::spawn(move || auto_loop(gen));
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

// 本循环是否仍然有效：RUNNING 且代际号未被新一次 start 递增
fn alive(gen: u64) -> bool {
    SIMON_RUNNING.load(Ordering::SeqCst) && SIMON_GEN.load(Ordering::SeqCst) == gen
}

// 定位网格：失败重试 ~10 帧（≈2s），仍失败返回 None（调用方停止并报错）
fn locate_grid_retry(gen: u64) -> Option<[Tile; 6]> {
    for i in 0..10 {
        if !alive(gen) {
            return None;
        }
        match capture_monitor_to_mem() {
            Ok(img) => match locate_grid(&img) {
                Ok(t) => return Some(t),
                Err(e) => info!("[simon] 定位失败({}/10): {}", i + 1, e),
            },
            Err(e) => info!("[simon] 截图失败({}/10): {}", i + 1, e),
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    None
}

fn record_loop(gen: u64) {
    // 1) 首次定位网格
    let tiles = match locate_grid_retry(gen) {
        Some(t) => t,
        None => {
            if !alive(gen) {
                return;
            }
            warn!("[simon] 未定位到 2×3 符文格，录制停止");
            emit_state(false, 0, Some("未定位到符文格，请确认停在第二关界面后重试"), None);
            SIMON_RUNNING.store(false, Ordering::SeqCst);
            return;
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

// ---------------- 自动模式（状态区 OCR 相位机）----------------

// 状态区 ROI：由网格几何推导（纯相对量，免绝对坐标、免分辨率适配）。
// 底排下沿下方 0.1~1.3 瓦高、水平居中 ±1.4 瓦宽的带——20261112 视频逐帧核实，
// 「开始」按钮、「正在播放...」、「轮次X/3」与错误文案都在此带内轮换；
// ±1.4 瓦宽收窄避开右下角主播摄像头入镜区。
fn status_region(tiles: &[Tile; 6], iw: u32, ih: u32) -> (u32, u32, u32, u32) {
    let bottom = &tiles[3..6];
    let x0 = bottom.iter().map(|t| t.x).min().unwrap_or(0);
    let x1 = bottom.iter().map(|t| t.x + t.w).max().unwrap_or(0);
    let yb = bottom.iter().map(|t| t.y + t.h).max().unwrap_or(0) as i64;
    let tw = bottom.iter().map(|t| t.w).min().unwrap_or(1) as i64;
    let th = bottom.iter().map(|t| t.h).min().unwrap_or(1) as i64;
    let cx = (x0 as i64 + x1 as i64) / 2;
    let half = tw * 7 / 5;
    let x = (cx - half).max(0) as u32;
    // 贴边收拢：屏幕边缘裁掉越界半边即可，ROI 至少保留 1px（极端贴底时
    // 状态文字本身已出屏，采集线程拿到的将是无效窄带，不影响正常场景）
    let right = ((cx + half).min(iw as i64)).max(x as i64 + 1) as u32;
    let top = (yb + th / 10).min(ih as i64 - 1).max(0) as u32;
    let bot = ((yb + th * 13 / 10).min(ih as i64)).max(top as i64 + 1) as u32;
    (x.min(iw), top.min(ih), right - x.min(iw), bot - top.min(ih))
}

fn crop_img(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = img.dimensions();
    let (x, y) = (x.min(iw.saturating_sub(1)), y.min(ih.saturating_sub(1)));
    image::imageops::crop_imm(img, x, y, w.min(iw - x).max(1), h.min(ih - y).max(1)).to_image()
}

// 亮度（Rec.601）：文字白/青高、面板深蓝底低。滤镜是全图单调变换，不改变
// 相对序——下文的分位/相对阈值全部只依赖亮度序，禁绝对阈值（既有约定）
fn luma(img: &RgbaImage) -> Vec<u8> {
    img.pixels()
        .map(|p| {
            let d = p.0;
            ((d[0] as u32 * 299 + d[1] as u32 * 587 + d[2] as u32 * 114) / 1000).min(255) as u8
        })
        .collect()
}

// 状态区变化签名：灰度按 24×6 块取均值、量化 16 级。变化块 ≥3 才触发 OCR
//（光标飘过只动 1~2 块）——OCR 只发生在文字状态切换的瞬间，播放期零 OCR 开销
fn region_signature(img: &RgbaImage) -> Vec<u8> {
    const BW: u32 = 24;
    const BH: u32 = 6;
    let (w, h) = img.dimensions();
    let lum = luma(img);
    let mut sig = Vec::with_capacity((BW * BH) as usize);
    for by in 0..BH {
        for bx in 0..BW {
            let (x0, x1) = (bx * w / BW, (bx + 1) * w / BW);
            let (y0, y1) = (by * h / BH, (by + 1) * h / BH);
            let mut sum = 0u32;
            let mut n = 0u32;
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    sum += lum[(y * w + x) as usize] as u32;
                    n += 1;
                    x += 1;
                }
                y += 1;
            }
            sig.push((if n > 0 { sum / n } else { 0 } / 16) as u8);
        }
    }
    sig
}

fn sig_changed(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b.iter()).filter(|(x, y)| x.abs_diff(**y) >= 2).count() >= 3
}

// 状态区 → OCR 画布。沿用既有 OCR 约定：灰度归一化优先（禁硬二值化）、紧裁
// 至文字外接框防超 WinOCR 1000px 宽返空、按字高自适应放大（目标 40px）、
// 24px 白边距防贴边识别失败。p5/p98 分位取底/字灰度（相对量），中点阈值出
// 文字 bbox；空带（无文字状态）或对比不足返回 None（不送 OCR）。
// 输出「底白字黑」灰度 PNG——WinOCR 偏好黑字白底极性。
fn ocr_prep(img: &RgbaImage) -> Option<Vec<u8>> {
    ocr_prep_scale(img, 40.0)
}

// target_h：归一化后目标字高 px；40 = 默认，更大值用于低分辨文本放大
fn ocr_prep_scale(img: &RgbaImage, target_h: f32) -> Option<Vec<u8>> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let lum = luma(img);
    let mut sorted = lum.clone();
    sorted.sort_unstable();
    let lo = sorted[sorted.len() / 20] as i32; // p5 底色
    let hi = sorted[sorted.len() * 98 / 100] as i32; // p98 字色
    if hi - lo < 40 {
        return None;
    }
    let mid = lo + (hi - lo) / 2;
    // 文字 bbox（密度紧裁）：逐列/行统计文字像素，只保留密度 ≥ 2% 维度的行列——
    // 按钮底色块、面板亮边这类大块亮内容会撑大 bbox 导致画布超限 OCR 返空
    let mut rows = vec![0u32; h as usize];
    let mut cols = vec![0u32; w as usize];
    for (i, &v) in lum.iter().enumerate() {
        if v as i32 > mid {
            let (x, y) = (i as u32 % w, i as u32 / w);
            cols[x as usize] += 1;
            rows[y as usize] += 1;
        }
    }
    let row_thr = (w / 50).max(2);
    let col_thr = (h / 50).max(2);
    let first = |v: &Vec<u32>, thr: u32| v.iter().position(|&c| c >= thr);
    let last = |v: &Vec<u32>, thr: u32| v.iter().rposition(|&c| c >= thr);
    let (Some(cx0), Some(cx1), Some(ry0), Some(ry1)) = (first(&cols, col_thr), last(&cols, col_thr), first(&rows, row_thr), last(&rows, row_thr)) else {
        return None;
    };
    let (x0, y0) = (
        ((cx0 as u32).saturating_sub(4)).min(w - 1),
        ((ry0 as u32).saturating_sub(4)).min(h - 1),
    );
    let (x1, y1) = (((cx1 as u32) + 4).min(w - 1), ((ry1 as u32) + 4).min(h - 1));
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    // 目标字高 target_h：scale = target_h/字高（不低于 1.0，降采样会丢笔画导致
    // OCR 丢字；仅画布超宽时按宽反推降采样——WinOCR >1000px 宽返空，既有教训）
    let mut scale = (target_h / bh as f32).clamp(1.0, 6.0);
    if bw as f32 * scale > 912.0 {
        scale = 912.0 / bw as f32;
    }
    let (ow, oh) = (((bw as f32 * scale) as u32).max(1), ((bh as f32 * scale) as u32).max(1));
    let mut gray = image::GrayImage::new(bw, bh);
    for y in 0..bh {
        for x in 0..bw {
            let v = lum[((y0 + y) * w + x0 + x) as usize] as i32;
            let g = 255 - (((v - lo) * 255 / (hi - lo)).clamp(0, 255)) as u8;
            gray.put_pixel(x, y, image::Luma([g]));
        }
    }
    let big = image::imageops::resize(&gray, ow, oh, image::imageops::FilterType::CatmullRom);
    let padded = pad_canvas(
        &image::DynamicImage::ImageLuma8(big),
        24,
        image::Rgba([255, 255, 255, 255]),
    );
    to_png(&padded).ok()
}

// 「开始」按钮检测区：底排下沿下方 0.10~0.50 瓦高、水平居中 ±0.65 瓦宽。
// 实测按钮（浅蓝填充）占此区主体；播放/轮次/错误状态的文字都在该区之下
fn button_region(tiles: &[Tile; 6], iw: u32, ih: u32) -> (u32, u32, u32, u32) {
    let bottom = &tiles[3..6];
    let x0 = bottom.iter().map(|t| t.x).min().unwrap_or(0);
    let x1 = bottom.iter().map(|t| t.x + t.w).max().unwrap_or(0);
    let yb = bottom.iter().map(|t| t.y + t.h).max().unwrap_or(0) as i64;
    let tw = bottom.iter().map(|t| t.w).min().unwrap_or(1) as i64;
    let th = bottom.iter().map(|t| t.h).min().unwrap_or(1) as i64;
    let cx = (x0 as i64 + x1 as i64) / 2;
    let half = tw * 13 / 20;
    let x = (cx - half).max(0) as u32;
    let right = ((cx + half).min(iw as i64)).max(x as i64 + 1) as u32;
    let top = (yb + th / 10).min(ih as i64 - 1).max(0) as u32;
    let bot = ((yb + th / 2).min(ih as i64)).max(top as i64 + 1) as u32;
    (x.min(iw), top.min(ih), right - x.min(iw), bot - top.min(ih))
}

// 「开始」按钮像素快检（纯相对量，无 OCR）：按钮浅蓝填充占该区主体，播放/
// 轮次/错误状态该区只有深色底。p98 字色、p5 底色、中点阈值以上像素占比
// >0.35 判为按钮存在——用于待机↔播放的快速切换，不等 OCR（「正在播放」
// WinOCR 偶发返空会拖到播放结束才采集，实机实测教训）
fn button_present(roi: &RgbaImage) -> bool {
    ink_frac(roi) > 0.35
}

// 带内亮内容像素占比（p5/p98 相对中点阈值以上）：同一文案下稳定，可作文字
// 体量的相对量。对比不足（hi-lo<40，纯色空带）返回 0
fn ink_frac(roi: &RgbaImage) -> f32 {
    let (w, h) = roi.dimensions();
    if w == 0 || h == 0 {
        return 0.0;
    }
    let lum = luma(roi);
    let mut sorted = lum.clone();
    sorted.sort_unstable();
    let lo = sorted[sorted.len() / 20] as i32;
    let hi = sorted[sorted.len() * 98 / 100] as i32;
    if hi - lo < 40 {
        return 0.0;
    }
    let mid = lo + (hi - lo) / 2;
    lum.iter().filter(|&&v| (v as i32) > mid).count() as f32 / lum.len() as f32
}

// 自动模式主循环（状态监视线程）：
// - 待机→播放：「开始」按钮像素快检（~0.1s 响应），按钮消失立即开始采集
// - 轮次/错误：状态区 OCR 分类（变化签名触发，播放期零 OCR 开销）
// - 复现态兜底：轮次文字消失而 OCR 连续读不出（≥4 次）时按新一轮播放处理——
//   宁可多采（错误会再清空）也不漏整轮
// 亮起采集在独立线程（auto_flash_loop）——OCR 的 100~300ms 阻断不能拖慢
// 播放期 ~0.2s 的闪亮采样，两线程各管各的截图。
fn auto_loop(gen: u64) {
    let tiles = match locate_grid_retry(gen) {
        Some(t) => t,
        None => {
            if !alive(gen) {
                return;
            }
            warn!("[simon] 未定位到 2×3 符文格，自动模式停止");
            emit_state(false, 0, Some("未定位到符文格，请确认停在第二关界面后重试"), None);
            SIMON_RUNNING.store(false, Ordering::SeqCst);
            return;
        }
    };
    info!("[simon] 网格定位完成: {:?}", tiles);
    let Some(engine) = ocr::zh_engine() else {
        warn!("[simon] 系统无中文 OCR 语言包，自动模式停止");
        emit_state(false, 0, Some("系统无中文 OCR 语言包，自动模式不可用"), None);
        SIMON_RUNNING.store(false, Ordering::SeqCst);
        return;
    };
    std::thread::spawn(move || auto_flash_loop(gen, tiles));

    let mut cur = Phase::Standby;
    let mut base_sig: Option<Vec<u8>> = None; // 当前相位文字签名基线（相位切换时重建）
    let mut base_ink = 0f32; // 复现态基线文字体量（轮次文案占比）
    let mut unknown_run = 0usize; // 连续未识别次数（≥2 在复现态兜底切轮）
    let mut drop_run = 0usize; // 体量骤降连续帧数（≥2 才切轮，防单帧带内污染）
    // 体量骤降阈值（20261112 4K 视频帧实测）：轮次文案占比 ~0.036，正在播放
    // ~0.022~0.025，真实转换最差比 0.68 → 0.75 稳定触发且留 ~10% 余量；错误屏
    // 文案更长不触发（且错误态「开始」按钮会被按钮快检先行接管回待机）
    const REPRO_INK_DROP: f32 = 0.75;
    while alive(gen) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let img = match capture_monitor_to_mem() {
            Ok(v) => v,
            Err(e) => {
                info!("[simon] 状态区截图失败: {}", e);
                continue;
            }
        };
        let (iw, ih) = img.dimensions();
        let (bx, by, bw, bh) = button_region(&tiles, iw, ih);
        let btn = button_present(&crop_img(&img, bx, by, bw, bh));

        if cur == Phase::Standby {
            if btn {
                continue;
            }
            // 按钮消失 = 玩家点了开始（开局不在待机也按播放中处理）：
            // 立即开始采集，不等 OCR
            steps_lock().clear();
            SIMON_PHASE.store(PHASE_PLAYING, Ordering::SeqCst);
            cur = Phase::Playing;
            base_sig = None;
            unknown_run = 0;
            emit_state(true, 0, None, Some("开始播放，正在采集亮起顺序…"));
            continue;
        }

        if btn {
            // 播放/复现中「开始」按钮回来了：错误屏或复现完成回待机
            SIMON_PHASE.store(PHASE_STANDBY, Ordering::SeqCst);
            cur = Phase::Standby;
            base_sig = None;
            emit_state(true, steps_lock().len(), None, Some("待机：等待玩家点「开始」"));
            continue;
        }

        let (rx, ry, rw, rh) = status_region(&tiles, iw, ih);
        let roi = crop_img(&img, rx, ry, rw, rh);
        let sig = region_signature(&roi);
        if base_sig.is_none() {
            // 新相位首帧：建立文字基线，本轮不 OCR
            base_sig = Some(sig);
            if cur == Phase::Reproduce {
                base_ink = ink_frac(&roi);
            }
            continue;
        }
        let ink = ink_frac(&roi);
        // 复现态体量骤降：轮次文案(11字)切到正在播放(7字)/文字消失，纯像素
        // 判定即刻进下一轮——不等 OCR（实机「正在播放」OCR 常年读不出会拖到
        // 播放结束才切轮，第二轮 0 采集的根因）。错误屏文案更长不会误触发；
        // 若误入错误屏，随后的按钮快检/错误 OCR 会清空回待机
        // v1.3.1 改连续 2 帧确认：真实文案切换体量持续走低，两帧（~200ms）才
        // 切轮；单帧带内像素突变（窗外窗口内容变化）不再立即误判。首帧骤降
        // 仍落入 OCR 路径——签名变化触发识别，污染守卫会重建基线体量
        let drop_now = cur == Phase::Reproduce && base_ink > 0.0 && ink < REPRO_INK_DROP * base_ink;
        if drop_now {
            drop_run += 1;
        } else {
            drop_run = 0;
        }
        if drop_now && drop_run >= 2 {
            steps_lock().clear();
            SIMON_PHASE.store(PHASE_PLAYING, Ordering::SeqCst);
            cur = Phase::Playing;
            base_sig = None;
            unknown_run = 0;
            emit_state(true, 0, None, Some("下一轮播放，正在采集…"));
            continue;
        }
        let changed = base_sig.as_ref().is_some_and(|s| sig_changed(s, &sig));
        if !changed && unknown_run == 0 {
            continue;
        }
        let Some(png) = ocr_prep(&roi) else {
            continue;
        };
        let text = match ocr::recognize_png(&engine, &png) {
            Ok(t) => t,
            Err(e) => {
                info!("[simon] 状态区 OCR 失败: {}", e);
                unknown_run += 1;
                continue;
            }
        };
        // 窗外污染守卫：游戏状态文字是纯中文+数字，OCR 结果含拉丁字母必然是
        // 其他窗口（IDE/聊天等）压进了识别带——忽略该帧且不计 unknown_run
        // （连续污染若累计会误触 ≥2 兜底切轮，实机教训：IDE 中文文字压进带内
        // 导致玩家复现中被清空重录），并重建体量/签名基线（持续污染不会累计
        // 骤降确认，污染消失后签名变化自然恢复识别）
        if text.chars().any(|c| c.is_ascii_alphabetic()) {
            base_sig = Some(sig);
            base_ink = ink;
            unknown_run = 0;
            info!("[simon] 状态区识别带被窗外内容污染，忽略: {}", text);
            continue;
        }
        let (phase, round) = classify_status(&text);
        if phase == Phase::Unknown {
            // WinOCR 对同一文案存在偶发返空/丢字（视频实测「正在播放...」约 1/4
            // 帧失败，实机更甚）：保留基线连续重试，复现态连续 2 次未识别视为
            // 已切到读不出的文案（下一轮播放）——否则会拖到播放结束
            unknown_run += 1;
            if cur == Phase::Reproduce && unknown_run >= 2 {
                steps_lock().clear();
                SIMON_PHASE.store(PHASE_PLAYING, Ordering::SeqCst);
                cur = Phase::Playing;
                base_sig = None;
                unknown_run = 0;
                emit_state(true, 0, None, Some("下一轮播放，正在采集…"));
            }
            info!("[simon] 状态区 {:?} 未识别({})", text, unknown_run);
            continue;
        }
        unknown_run = 0;
        info!("[simon] 状态区 {:?} → {:?}", text, phase);
        match phase {
            Phase::Playing => {
                if cur == Phase::Playing {
                    // 播放中文案微动（省略号动画）：重建基线停掉重复 OCR
                    base_sig = Some(sig);
                } else {
                    // 复现完成回播放 = 下一轮：清空必须在置相位前（采集线程
                    // 只在 PLAYING 相位写入，先清后置无竞态）
                    steps_lock().clear();
                    SIMON_PHASE.store(PHASE_PLAYING, Ordering::SeqCst);
                    cur = Phase::Playing;
                    base_sig = None;
                    emit_state(true, 0, None, Some("下一轮播放，正在采集…"));
                }
            }
            Phase::Reproduce => {
                if cur != Phase::Reproduce {
                    SIMON_PHASE.store(PHASE_REPRODUCE, Ordering::SeqCst);
                    cur = Phase::Reproduce;
                    base_sig = None;
                    let mut msg = format!("轮次{}：已录{}步，请照抄复现", round, steps_lock().len());
                    if (1..=3).contains(&round) {
                        let cap = 3 + round;
                        if steps_lock().len() > cap {
                            steps_lock().truncate(cap);
                        }
                        let n = steps_lock().len();
                        msg = if n < cap {
                            format!("轮次{}/3：录得{}步（应{}步，可能漏采）", round, n, cap)
                        } else {
                            format!("轮次{}/3：已录{}步，请照抄复现", round, n)
                        };
                    }
                    info!("[simon] {}", msg);
                    emit_state(true, steps_lock().len(), None, Some(&msg));
                } else {
                    // 仍在复现（光标飘过/文案微动误触发变化）：重建基线
                    base_sig = Some(sig);
                    base_ink = ink;
                }
            }
            Phase::Error => {
                steps_lock().clear();
                SIMON_PHASE.store(PHASE_STANDBY, Ordering::SeqCst);
                cur = Phase::Standby;
                base_sig = None;
                emit_state(true, 0, None, Some("错误！已清空采集，点游戏「开始」重来"));
            }
            // 播放/复现中读到「开始」文案（过渡帧残留）：不切相，按钮像素判定为准
            Phase::Standby => {}
            Phase::Unknown => unreachable!(),
        }
    }
}

// 自动模式的亮起采集：与自由录制同款边沿判定（1 帧亮确认 + 1 帧暗清除 +
// 同格 300ms 不应期），差别只在记录门控——仅 PLAYING 相位记录，「轮次X/3」
// 出现后玩家复现的按压亮起不再计入（相位由 auto_loop 的 OCR 切换）。
fn auto_flash_loop(gen: u64, tiles: [Tile; 6]) {
    let mut lit_state = [false; 6];
    let mut last_rec = [None; 6];
    let mut in_round = false;
    const REFRACTORY: std::time::Duration = std::time::Duration::from_millis(300);
    const FAIL_LIMIT: u32 = 25;
    let mut fail_run = 0u32;
    while alive(gen) {
        if SIMON_PHASE.load(Ordering::SeqCst) != PHASE_PLAYING {
            in_round = false;
            std::thread::sleep(std::time::Duration::from_millis(30));
            continue;
        }
        if !in_round {
            // 新一轮：亮态清零（相位离开期间残留的亮态会吞掉同格上升沿）
            lit_state = [false; 6];
            last_rec = [None; 6];
            in_round = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
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
                let now = std::time::Instant::now();
                if last_rec[i].is_some_and(|t| now.duration_since(t) < REFRACTORY) {
                    lit_state[i] = true;
                    continue;
                }
                if !alive(gen) {
                    return;
                }
                // 写入前复核相位：轮次判定已切走就不再记录玩家复现的按压
                if SIMON_PHASE.load(Ordering::SeqCst) != PHASE_PLAYING {
                    continue;
                }
                last_rec[i] = Some(now);
                lit_state[i] = true;
                let seq = steps_lock().len() + 1;
                let step = json!({ "seq": seq, "index": i, "name": SIMON_NAMES[i] });
                steps_lock().push(step.clone());
                if !alive(gen) {
                    steps_lock().pop();
                    return;
                }
                // 写入后复核：与轮次截断的竞态回滚（写入间隙相位切走则丢弃）
                if SIMON_PHASE.load(Ordering::SeqCst) != PHASE_PLAYING {
                    steps_lock().pop();
                    continue;
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
            } else if !now_lit && lit_state[i] {
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

    // 自动模式状态文字分类：关键词子串 + 顺序敏感（错误文案含「开始」须先判错误）
    #[test]
    fn simon_classify_status() {
        assert_eq!(classify_status("开始"), (Phase::Standby, 0));
        assert_eq!(classify_status("点击「开始」开始播放"), (Phase::Standby, 0));
        assert_eq!(classify_status("正在播放..."), (Phase::Playing, 0));
        assert_eq!(classify_status("轮次1/3·请复现顺序"), (Phase::Reproduce, 1));
        assert_eq!(classify_status("轮次2/3·请复现顺序"), (Phase::Reproduce, 2));
        assert_eq!(classify_status("轮次3/3·请复现顺序"), (Phase::Reproduce, 3));
        // OCR 噪声容错：轮次数字缺失 → round=0（不截断，不误伤）
        assert_eq!(classify_status("轮次/3·请复现顺序"), (Phase::Reproduce, 0));
        // 「错误！点击开始重试」含「开始」，错误判定必须优先
        assert_eq!(classify_status("错误！点击开始重试"), (Phase::Error, 0));
        // 全不匹配 = Unknown（下轮强制重 OCR）
        assert_eq!(classify_status(""), (Phase::Unknown, 0));
        assert_eq!(classify_status("看不清的噪声文字"), (Phase::Unknown, 0));
    }

    // 状态区 ROI：由网格几何推导的相对量（底排下沿下方 0.1~1.3 瓦高、居中 ±1.4 瓦宽）
    #[test]
    fn simon_status_region_relative() {
        let t = |x: u32, y: u32, w: u32, h: u32| Tile { x, y, w, h };
        let tiles = [t(100, 100, 80, 80), t(200, 100, 80, 80), t(300, 100, 80, 80), t(100, 200, 80, 80), t(200, 200, 80, 80), t(300, 200, 80, 80)];
        let (x, y, w, h) = status_region(&tiles, 1920, 1080);
        // 居中：ROI 中心 ≈ 网格中心 (240)
        assert!((x + w / 2) as i64 - 240 <= 2, "水平居中偏移: {}", x + w / 2);
        // 上沿 = 底排下沿 (280) + 0.1 瓦高 (8)
        assert_eq!(y, 288, "上沿应在底排下沿下方 0.1 瓦高");
        // 下沿 = 底排下沿 + 1.3 瓦高 (104)
        assert_eq!(y + h, 384, "下沿应在底排下沿下方 1.3 瓦高");
        // 半宽 = 1.4 瓦宽 (112)
        assert_eq!(w, 224, "带宽应为 2.8 瓦宽");
        // 边界收拢：网格贴屏幕右下时不得越界
        let tiles_edge = [t(1800, 980, 80, 80), t(1900, 980, 80, 80), t(2000, 980, 80, 80), t(1800, 1080, 80, 80), t(1900, 1080, 80, 80), t(2000, 1080, 80, 80)];
        let (x, y, w, h) = status_region(&tiles_edge, 1920, 1080);
        assert!(x + w <= 1920 && y + h <= 1080, "ROI 越界: x={} y={} w={} h={}", x, y, w, h);
        assert!(w > 0 && h > 0);
    }

    // 变化签名：文字状态切换（块均值大变）应触发 ≥3 块差异；光标微动不应触发
    #[test]
    fn simon_sig_changed() {
        let base = vec![5u8; 144];
        assert!(!sig_changed(&base, &base));
        // 单块抖动（光标飘过）：不触发
        let mut one = base.clone();
        one[60] = 9;
        assert!(!sig_changed(&base, &one));
        // 文字切换：多块均值跨 2 级
        let mut many = base.clone();
        for i in 0..30 {
            many[i] = 8;
        }
        assert!(sig_changed(&base, &many));
    }

    // 「开始」按钮像素快检：按钮填充占主体判存在；文字占比小/纯色底判不存在
    #[test]
    fn simon_button_present() {
        let img = |frac_bright: f32| -> RgbaImage {
            let mut im = image::RgbaImage::new(100, 40);
            let n = (100 * 40) as f32 * frac_bright;
            for (i, p) in im.pixels_mut().enumerate() {
                if (i as f32) < n {
                    *p = image::Rgba([150, 180, 220, 255]); // 按钮浅蓝填充
                } else {
                    *p = image::Rgba([30, 40, 70, 255]); // 面板深蓝底
                }
            }
            im
        };
        assert!(button_present(&img(0.7)), "按钮填充应判存在");
        assert!(!button_present(&img(0.1)), "纯文字占比不应判存在");
        // 纯色底（无对比，hi-lo<40）：判不存在
        let flat = image::RgbaImage::from_pixel(100, 40, image::Rgba([40, 50, 80, 255]));
        assert!(!button_present(&flat));
    }
}
