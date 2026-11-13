// 第一关「能量校准」波形求解管线（v1.2.0）。
// 算法整体移植自 GOC解题工具11.9/analysis/solver_lib.py 的 analyze()（视频逐帧
// 端到端验证通过：双正弦拟合 R²≈0.995~0.997），核心思路：
//   1. 橙/黄掩膜提取目标曲线（系统给定），其像素分布推出画布横向范围；
//   2. 画布内近黑背景行块推出纵向范围；
//   3. 逐列取曲线 y 中心，双正弦（基波+二次谐波）网格搜索 + 线性最小二乘拟合；
//   4. 基波参数逆映射回四个滑块（0~100）：振幅/频率/相位/偏移。
// 只识别黄色目标线 = 绝对值模式，输出即「照抄」的滑块位置。

use crate::solver::{capture_fullscreen, load_screenshot};
use crate::APP;
use log::{info, warn};
use serde_json::{json, Value};
use tauri::Manager;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

// 全屏截图缓存名（截图管线写入）
const FULL_SHOT: &str = "pot_screenshot_full.png";
// 手动框选区域截图缓存名（截图窗口 cut_image 命令写入）
const CUT_SHOT: &str = "pot_screenshot_cut.png";

// 最近一次求解结果：助手窗口前端挂载后经 get_wave_scan 拉取（防 emit 竞态）
pub static WAVE_SCAN: Mutex<Option<Value>> = Mutex::new(None);

// 扫描结果互斥锁的容错获取：持锁线程 panic 中毒后恢复内部数据继续服务，
// 避免连锁 panic 让求解线程静默死亡（仿 simon.rs steps_lock）
fn scan_lock() -> std::sync::MutexGuard<'static, Option<Value>> {
    WAVE_SCAN.lock().unwrap_or_else(|e| e.into_inner())
}

// 单飞保护：与符文识别（SCAN_IN_FLIGHT）同一理由（并发截图会写坏缓存 PNG）
static WAVE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

// 结果窗口真实可见性：Windows 上 tao 的 is_visible() 读内部 flag，而本窗口经
// Win32 SetWindowPos(SWP_SHOWWINDOW) 显示、ShowWindow(SW_HIDE) 隐藏（tao 陷阱，
// 内部 flag 恒 false），必须直接问系统 IsWindowVisible 才能拿到真实状态
fn result_window_visible(window: &tauri::Window) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        window
            .hwnd()
            .map(|h| unsafe { IsWindowVisible(HWND(h.0 as *mut _)).as_bool() })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        window.is_visible().unwrap_or(false)
    }
}

// 推送结果到结果窗口：窗口已可见时保持原位不动（识别中窗口重定向很打扰），
// 仅在窗口不存在/隐藏时创建并显示（定位到鼠标附近）。须在主线程调用。
fn notify_result(payload: Value) {
    let visible = APP
        .get()
        .and_then(|app| app.get_window("result"))
        .map(|w| result_window_visible(&w))
        .unwrap_or(false);
    if !visible {
        crate::window::get_or_create_result_window();
        crate::window::show_result_window();
    }
    if let Some(app) = APP.get() {
        if let Some(w) = app.get_window("result") {
            let _ = w.emit("wave_scan", payload);
        }
    }
}

// 热键 / 窗口按钮主入口：全屏截图 + 结构定位
pub fn wave_scan() {
    if WAVE_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        info!("[wave] 已有求解任务进行中，忽略本次触发");
        return;
    }
    std::thread::spawn(move || {
        let payload = analyze(true, FULL_SHOT, false);
        WAVE_IN_FLIGHT.store(false, Ordering::SeqCst);
        *scan_lock() = Some(payload.clone());
        if let Some(app) = APP.get().cloned() {
            let _ = app.run_on_main_thread(move || {
                // 结果窗口不存在时先创建（主线程内），再推事件；前端收到后切到波形页
                notify_result(payload.clone());
            });
        }
    });
}

// 手动框选完成入口（window.rs 按 SHOT_PURPOSE="wave" 派发）：对用户框选的
// 区域截图跑同一求解管线。结果窗口在框选前已被隐藏，此处恢复显示。
pub fn on_region_selected() {
    if WAVE_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        info!("[wave] 已有求解任务进行中，忽略本次框选触发");
        return;
    }
    std::thread::spawn(move || {
        let payload = analyze(false, CUT_SHOT, true);
        WAVE_IN_FLIGHT.store(false, Ordering::SeqCst);
        *scan_lock() = Some(payload.clone());
        if let Some(app) = APP.get().cloned() {
            let _ = app.run_on_main_thread(move || {
                notify_result(payload.clone());
            });
        }
    });
}

fn analyze(capture: bool, shot: &str, manual: bool) -> Value {
    if capture {
        if let Err(e) = capture_fullscreen() {
            warn!("[wave] 全屏截图失败: {}", e);
            return payload(false, None, &format!("全屏截图失败：{}", e));
        }
    }
    let img = match load_screenshot(shot) {
        Ok(v) => v,
        Err(e) => {
            warn!("[wave] 截图读取失败: {}", e);
            return payload(false, None, &e);
        }
    };
    match solve_image(&img, manual) {
        Ok(v) => v,
        Err(msg) => {
            warn!("[wave] 求解失败: {}", msg);
            payload(false, None, &msg)
        }
    }
}

fn payload(ok: bool, sliders: Option<Value>, msg: &str) -> Value {
    json!({ "ok": ok, "sliders": sliders, "msg": msg })
}

// ---------------- 求解主流程（移植 solver_lib.analyze 的目标线分支） ----------------

// v1.2.4 滤镜适配重写：玩家可能叠加任意亮度/对比/饱和度/色调滤镜，绝对 RGB
// 阈值必然漂移（20261112 真机样张实测：蓝调滤镜把目标曲线从橙（R-B>18）洗到
// R-B≈-1，旧橙判据整簇丢失；画布底色 sum 79、面板 138，旧的 30/45/60 暗档
// 全部落空）。滤镜是全图单调变换，相对关系不漂——画布永远比周边面板暗、
// 目标曲线永远是画布里「最暖的亮线」（README「滤镜免疫=相对色序」的推广）：
//   定位 = 分级掩膜（绝对暗档 30/60/90/120 兜底极暗与中性画布 + 蓝主导像素
//   G 中位数的 0.3/0.4 倍相对档，适配滤镜提亮后的深蓝画布）→ 行块×列块围候选
//   → 区块内「暖亮像素」（比区块内 p75 亮且比区块暖度中位暖 8+）打分过滤；
//   提线 = 画布内逐列取「最暖的亮像素簇」（curve_points）；
//   每个候选拟合后 R²≥0.5 校验，不合格继续试下一候选。
// manual=true 手动框选模式：用户只圈住画布（或画布加少量周边），
// 几何门槛按小图放宽（全屏模式的门槛按 2560 宽标定，小图会全部落空）。
fn solve_image(img: &image::RgbaImage, manual: bool) -> Result<Value, String> {
    let (w, h) = img.dimensions();
    let min_bh = if manual { 40 } else { 60 };
    let min_cols = if manual { 60 } else { 200 };
    let min_span = if manual { 60 } else { 100 };

    // 1) 逐像素亮度 lum、暖度 rb（R-B，i32 有符号：蓝调滤镜下全图为负仍要比序）、G 通道。
    //    直接读底层字节切片（RGBA4 通道交错），避免逐像素 get_pixel 的边界检查开销
    //    （仿 simon.rs edge_mask；行主序遍历与原双重循环逐像素等价）
    let mut lum = vec![0u32; (w * h) as usize];
    let mut rb = vec![0i32; (w * h) as usize];
    let mut gch = vec![0u8; (w * h) as usize];
    let raw = img.as_raw();
    for (i, chunk) in raw.chunks_exact(4).enumerate() {
        lum[i] = chunk[0] as u32 + chunk[1] as u32 + chunk[2] as u32;
        rb[i] = chunk[0] as i32 - chunk[2] as i32;
        gch[i] = chunk[1];
    }

    // 2) 分级掩膜：
    //    绝对暗档 [30,60,90,120]——极暗/中性画布（看图软件深灰底、低亮度场景）；
    //    相对档 2 档——蓝主导像素（B>R+20）G 中位数的 0.3/0.4 倍且 B>R+20：
    //    滤镜整体提亮时分子分母同步上移，画布-面板的 G 比率不变
    //    （实测画布 G≈15 vs 面板 G≈44、机器 G≈49、场景 G≈100+）。
    let mut masks: Vec<Vec<bool>> = Vec::with_capacity(6);
    for &t in &[30u32, 60, 90, 120] {
        masks.push(lum.iter().map(|&v| v < t).collect());
    }
    let mut gs: Vec<u32> = Vec::new();
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let i = (y * w + x) as usize;
            if rb[i] < -20 {
                gs.push(gch[i] as u32);
            }
        }
    }
    let gmed = median_u32(&mut gs).max(1);
    for f in [3u32, 4] {
        let tg = (gmed * f / 10).max(20);
        masks.push(
            gch
                .iter()
                .zip(rb.iter())
                .map(|(&g, &r)| r < -20 && (g as u32) < tg)
                .collect(),
        );
    }

    // 3) 候选循环：行块 × 列范围围出候选区块 → 暖亮像素打分 → 分位收紧 → 纵向收边 → 拟合
    let row_min = ((w as f32 * 0.1).round().clamp(100.0, 250.0)) as u32;
    let mut best_err: Option<String> = None;
    for (ti, mask) in masks.iter().enumerate() {
        // 行块：整行掩膜像素数 > row_min，断续 > 8 行切开
        let mut blocks: Vec<(u32, u32)> = Vec::new();
        let mut start: Option<u32> = None;
        let mut prev = 0u32;
        for y in 0..h {
            // 整行掩膜取连续切片计数，替代逐像素闭包索引（免重复边界检查）
            let row = (y * w) as usize;
            let cnt = mask[row..row + w as usize].iter().filter(|&&v| v).count() as u32;
            if cnt > row_min {
                if start.is_none() {
                    start = Some(y);
                }
                prev = y;
            } else if let Some(s) = start {
                if y > prev + 8 {
                    blocks.push((s, prev));
                    start = None;
                }
            }
        }
        if let Some(s) = start {
            blocks.push((s, prev));
        }

        for (y0b, y1b) in blocks {
            let bh = y1b - y0b;
            if bh < min_bh {
                continue;
            }
            // 列范围：块内掩膜像素过半行高的列，取首尾
            let mut colc = vec![0u32; w as usize];
            for y in y0b..=y1b {
                for x in 0..w {
                    if mask[(y * w + x) as usize] {
                        colc[x as usize] += 1;
                    }
                }
            }
            let mut col_hits = 0u32;
            let (mut cx0, mut cx1) = (None, None);
            for x in 0..w {
                if colc[x as usize] * 2 > bh {
                    col_hits += 1;
                    cx0.get_or_insert(x);
                    cx1 = Some(x);
                }
            }
            let (Some(x0), Some(x1)) = (cx0, cx1) else {
                continue;
            };
            if col_hits < min_cols {
                continue;
            }

            // 区块内统计基线（隔 2 采样）：亮度 p75 + 暖度中位。
            // 曲线判据 = 比区块内大多数像素亮、且比区块中位暖度更暖——
            // 全图滤镜下单调变换保持这两个次序
            let mut bl: Vec<u32> = Vec::new();
            let mut brb: Vec<i32> = Vec::new();
            for y in (y0b..=y1b).step_by(2) {
                for x in (x0..=x1).step_by(2) {
                    let i = (y * w + x) as usize;
                    bl.push(lum[i]);
                    brb.push(rb[i]);
                }
            }
            let lum_p75 = percentile(bl, 75.0);
            let rb_med = median_i32(&mut brb);
            let mut cxs: Vec<u32> = Vec::new();
            let mut cys: Vec<u32> = Vec::new();
            for y in y0b..=y1b {
                for x in x0..=x1 {
                    let i = (y * w + x) as usize;
                    if lum[i] > lum_p75 && rb[i] > rb_med + 8 {
                        cxs.push(x);
                        cys.push(y);
                    }
                }
            }
            if cxs.len() < 150 {
                continue;
            }

            // 暖亮像素分位收紧（剔除块内孤立噪点），得到曲线实际横向范围
            let px0 = percentile(cxs.clone(), 0.3);
            let px1 = percentile(cxs.clone(), 99.7);
            if px1 <= px0 + min_span {
                continue;
            }
            let band_w = (px1 - px0 + 1) as u32;
            let ymid = percentile(cys.clone(), 50.0);

            // 画布纵向：x 带内掩膜行过半的连续行块，取包含曲线中位 y 的块
            let mut sub_rows: Vec<u32> = Vec::new();
            for y in 0..h {
                // x 带内连续切片计数，替代逐像素闭包索引（免重复边界检查）
                let row = (y * w) as usize;
                let cnt = mask[row + px0 as usize..=row + px1 as usize]
                    .iter()
                    .filter(|&&v| v)
                    .count() as u32;
                if cnt * 2 > band_w {
                    sub_rows.push(y);
                }
            }
            let Some((y0, y1)) = block_containing(&sub_rows, ymid).filter(|&(s, e)| e >= s + 40)
            else {
                continue;
            };
            info!(
                "[wave] 画布候选 mask#{} rect=({},{})-({},{}) 曲线像素 {}",
                ti,
                px0,
                y0,
                px1,
                y1,
                cxs.len()
            );

            // 曲线逐列 y 中心 + 双正弦拟合（curve_points 恒返回非空 Vec，无失败路径）
            let pts = curve_points(img, px0, px1, y0, y1);
            if pts.len() < 60 {
                continue;
            }
            let fit = fit_two(&pts);
            if fit.r2 >= 0.5 {
                let (amp, freq, phase, off) = to_sliders(&fit);
                info!(
                    "[wave] 拟合 R²={:.4} f1={:.3} f2={:.3} A1={:.3} → 滑块 振幅{} 频率{} 相位{} 偏移{}",
                    fit.r2, fit.f1, fit.f2, fit.a1, amp, freq, phase, off
                );
                return Ok(json!({
                    "ok": true,
                    "msg": "",
                    "sliders": json!({ "amp": amp, "freq": freq, "phase": phase, "off": off }),
                    // 完整双正弦参数（归一化坐标 y=off+a1·sin(2πf1x+p1)+a2·sin(2πf2x+p2)），
                    // 前端预览曲线与游戏目标线同构必需——只画基波会缺二次谐波波纹
                    "curve": json!({
                        "a1": fit.a1, "f1": fit.f1, "p1": fit.p1, "off": fit.off,
                        "a2": fit.a2, "f2": fit.f2, "p2": fit.p2,
                    }),
                }));
            }
            best_err = Some(format!(
                "曲线拟合质量不足（R²={:.3}），请重试或确认画布完整可见",
                fit.r2
            ));
        }
    }
    Err(best_err.unwrap_or_else(|| {
        "未定位到波形画布：请确认停留在第一关「能量校准」界面，且画布未被大面积遮挡".into()
    }))
}

fn median_u32(v: &mut Vec<u32>) -> u32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

fn median_i32(v: &mut Vec<i32>) -> i32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

// 画布内目标线逐列 y 中心 → 归一化曲线点。
// 滤镜适配：基线取画布内的亮度中位 bg 与暖度中位 wb（相对量），逐列在
// 「亮于 bg+Δ」的像素里优先取「最暖（R-B 相对最高）」的像素簇（目标线原色
// 黄/橙，任何全图单调滤镜下都保持相对最暖；玩家自己的曲线偏冷被排除）；
// 三级从严到宽，末级放弃暖度筛选退化为「全部亮像素均值」（两线靠近时仍可用）。
fn curve_points(
    img: &image::RgbaImage,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
) -> Vec<(f32, f32)> {
    let band_w = (x1 - x0 + 1) as usize;
    let hh = (y1 - y0) as f32 / 2.0;
    // 画布内基线（隔 2 采样统计）
    let mut lums: Vec<u32> = Vec::new();
    let mut rbs: Vec<i32> = Vec::new();
    for y in (y0..=y1).step_by(2) {
        for x in (x0..=x1).step_by(2) {
            let p = img.get_pixel(x, y).0;
            lums.push(p[0] as u32 + p[1] as u32 + p[2] as u32);
            rbs.push(p[0] as i32 - p[2] as i32);
        }
    }
    let bg = percentile(lums, 50.0) as i64;
    let wb = median_i32(&mut rbs) as i64;
    // 三级 (Δ亮, Δ暖)：从严到宽；末级 Δ暖 = MIN 哨兵表示不筛暖度。
    // 注意：MIN 哨兵下 wb + dw 必然溢出（深蓝画布 wb 为负，如 -66，wb + i64::MIN
    // 在 debug 下 panic、release 下回绕成巨大正数使兜底恒假），比较处必须用
    // saturating_add（见下方 r > wb.saturating_add(dw)）
    let tiers: [(i64, i64); 3] = [(60, 8), (40, 4), (25, i64::MIN)];
    let mut chosen: Option<Vec<Option<f32>>> = None;
    let mut best_cov = -1f32;
    let mut best_ys: Vec<Option<f32>> = Vec::new();
    for &(dl, dw) in tiers.iter() {
        let mut cur: Vec<Option<f32>> = Vec::with_capacity(band_w);
        for ci in 0..band_w {
            let x = x0 + ci as u32;
            // 列内「亮于背景且暖于基线+Δ」的像素均值；末级 dw=MIN 时暖条件恒真，
            // 退化为全部亮像素均值（旧行为兜底）
            let mut ysum = 0f32;
            let mut cnt = 0u32;
            for y in y0..=y1 {
                let p = img.get_pixel(x, y).0;
                let l = (p[0] as u32 + p[1] as u32 + p[2] as u32) as i64;
                if l <= bg + dl {
                    continue;
                }
                let r = (p[0] as i32 - p[2] as i32) as i64;
                // saturating_add：dw=MIN 哨兵时 wb+dw 会 i64 溢出（wb 为负时更甚），
                // 饱和后得到 i64::MIN，r 恒大于它 → 末级「不筛暖度」兜底保持恒真
                if r > wb.saturating_add(dw) {
                    ysum += (y - y0) as f32;
                    cnt += 1;
                }
            }
            cur.push(if cnt > 0 { Some(ysum / cnt as f32) } else { None });
        }
        let cov = cur.iter().filter(|v| v.is_some()).count() as f32 / band_w as f32;
        let std = col_std(&cur);
        if cov >= 0.2 && std > 2.0 {
            chosen = Some(cur);
            break;
        }
        if cov > best_cov {
            best_cov = cov;
            best_ys = cur;
        }
    }
    let ys = chosen.unwrap_or(best_ys);
    let band_wm1 = (band_w - 1).max(1) as f32;
    ys.iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|y| (i as f32 / band_wm1, -(y - hh) / hh)))
        .collect()
}

fn percentile(mut v: Vec<u32>, p: f32) -> u32 {
    v.sort_unstable();
    let n = v.len();
    if n == 0 {
        return 0;
    }
    let idx = ((p / 100.0) * (n - 1) as f32).round() as usize;
    v[idx.min(n - 1)]
}

// 连续行块（断续 > 8 行切开）中包含 ymid 的块
fn block_containing(rows: &[u32], ymid: u32) -> Option<(u32, u32)> {
    let mut blocks: Vec<(u32, u32)> = Vec::new();
    let mut start: Option<u32> = None;
    let mut prev = 0u32;
    for (k, &y) in rows.iter().enumerate() {
        if let Some(s) = start {
            if y > prev + 8 {
                blocks.push((s, prev));
                start = Some(y);
            }
        } else {
            start = Some(y);
        }
        prev = y;
        let _ = k;
    }
    if let Some(s) = start {
        blocks.push((s, prev));
    }
    blocks.into_iter().find(|&(s, e)| s <= ymid && ymid <= e)
}

fn col_std(ys: &[Option<f32>]) -> f32 {
    let v: Vec<f32> = ys.iter().filter_map(|v| *v).collect();
    if v.is_empty() {
        return 0.0;
    }
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / v.len() as f32;
    var.sqrt()
}

// ---------------- 线性最小二乘（列 = [1, sin/cos(2πf·x) × 频率数]） ----------------

// 返回 (sse, coef)；sse = Σy² - coefᵀXᵀy（正规方程精确解）
fn solve_lin(pts: &[(f32, f32)], freqs: &[f32]) -> (f32, Vec<f32>) {
    let k = 1 + freqs.len() * 2;
    let mut xt_x = vec![vec![0f32; k]; k];
    let mut xty = vec![0f32; k];
    let mut yy = 0f32;
    let ws: Vec<f32> = freqs
        .iter()
        .map(|f| std::f32::consts::TAU * f)
        .collect();
    for &(x, y) in pts {
        let mut cols = Vec::with_capacity(k);
        cols.push(1.0);
        for w in ws.iter() {
            cols.push((w * x).sin());
            cols.push((w * x).cos());
        }
        for i in 0..k {
            xty[i] += cols[i] * y;
            for j in i..k {
                xt_x[i][j] += cols[i] * cols[j];
            }
        }
        yy += y * y;
    }
    for i in 0..k {
        for j in i..k {
            xt_x[j][i] = xt_x[i][j];
        }
    }
    let coef = gauss_solve(&xt_x, &xty);
    let mut sse = yy;
    for i in 0..k {
        sse -= coef[i] * xty[i];
    }
    (sse.max(0.0), coef)
}

// 通用高斯消元（部分主元，n ≤ 5）
fn gauss_solve(a: &[Vec<f32>], b: &[f32]) -> Vec<f32> {
    let n = b.len();
    let mut m = vec![vec![0f32; n + 1]; n];
    for i in 0..n {
        m[i][..n].copy_from_slice(&a[i]);
        m[i][n] = b[i];
    }
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if m[r][col].abs() > m[piv][col].abs() {
                piv = r;
            }
        }
        m.swap(col, piv);
        let d = m[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for r in 0..n {
            if r != col && m[r][col] != 0.0 {
                let f = m[r][col] / d;
                for c in col..n + 1 {
                    m[r][c] -= f * m[col][c];
                }
            }
        }
    }
    (0..n)
        .map(|i| if m[i][i].abs() < 1e-12 { 0.0 } else { m[i][n] / m[i][i] })
        .collect()
}

fn sse_of(pts: &[(f32, f32)], f1: f32, f2: f32) -> f32 {
    solve_lin(pts, &[f1, f2]).0
}

// 单频拟合 SSE：搜基波时用（双频率占位会让设计矩阵近奇异，高斯消元解发散，
// 峰位被带偏；单频 3 列方程组条件数良好）
fn sse1(pts: &[(f32, f32)], f: f32) -> f32 {
    solve_lin(pts, &[f]).0
}

// 频率细化：3 轮，每轮在 ±span 内以 span/10 步长搜峰，span 每轮 /4。
// other = Some(o) 时按 双频率(o 固定) 评估（联合微调阶段），None 时按单频评估
fn refine_f(pts: &[(f32, f32)], f: f32, span: f32, other: Option<f32>) -> f32 {
    let mut f = f;
    let mut span = span;
    for _ in 0..3 {
        let lo = (f - span).max(0.05);
        let step = span / 10.0;
        let mut best = (f32::INFINITY, f);
        let mut v = lo;
        while v <= f + span {
            let s = match other {
                Some(o) => sse_of(pts, v, o),
                None => sse1(pts, v),
            };
            if s < best.0 {
                best = (s, v);
            }
            v += step;
        }
        f = best.1;
        span /= 4.0;
    }
    f
}

// fit_two：基波粗搜 0.3~3.3 → 细化 → 二次谐波在 [1.5f1, min(9.5, 3.3f1)] 粗搜 →
// 细化 → 两轮联合微调（全部移植自 solver_lib.fit_two）
fn fit_two(pts: &[(f32, f32)]) -> Fit {
    let mut best1 = (f32::INFINITY, 0.8f32);
    let mut f0 = 0.3f32;
    while f0 <= 3.31 {
        let s = sse1(pts, f0);
        if s < best1.0 {
            best1 = (s, f0);
        }
        f0 += 0.1;
    }
    let mut f1 = refine_f(pts, best1.1, 0.06, None);

    let lo = (1.5 * f1).max(0.3);
    let hi = (3.3 * f1).min(9.5);
    let mut best2 = (f32::INFINITY, f1 * 2.0);
    let mut f2 = lo;
    while f2 <= hi {
        if (f2 - f1).abs() >= 0.15 {
            let s = sse_of(pts, f1, f2);
            if s < best2.0 {
                best2 = (s, f2);
            }
        }
        f2 += 0.05;
    }
    let mut f2 = refine_f(pts, best2.1, 0.04, Some(f1));

    // 坐标下降式联合微调：交替固定另一频率细化当前频率
    for _ in 0..2 {
        let f1n = refine_f(pts, f1, 0.012, Some(f2));
        let f2n = refine_f(pts, f2, 0.012, Some(f1n));
        if sse_of(pts, f1n, f2n) < sse_of(pts, f1, f2) {
            f1 = f1n;
            f2 = f2n;
        }
    }

    let (_, coef) = solve_lin(pts, &[f1, f2]);
    let yy: f32 = pts.iter().map(|&(_, y)| y * y).sum();
    let sse = sse_of(pts, f1, f2);
    let a1 = coef[1].hypot(coef[2]);
    let mut p1 = coef[2].atan2(coef[1]);
    if p1 < 0.0 {
        p1 += std::f32::consts::TAU;
    }
    // 二次谐波参数（coef = [off, sin1, cos1, sin2, cos2]）：预览曲线与游戏画布
    // 完全同构必需，只推基波会缺谐波波纹
    let a2 = coef[3].hypot(coef[4]);
    let p2 = coef[4].atan2(coef[3]);
    let off = coef[0];
    let r2 = if yy > 0.0 { 1.0 - sse / yy } else { 0.0 };
    // R² 校验上移到 solve_image 候选循环（低质量候选记入 best_err 后继续试下一候选）
    Fit { f1, f2, a1, p1, off, r2, a2, p2 }
}

struct Fit {
    f1: f32,
    f2: f32,
    a1: f32,
    p1: f32,
    off: f32,
    r2: f32,
    a2: f32,
    p2: f32,
}

// 滑块逆映射（11.9 实测标定；偏移滑块左=上，方向已含在公式内）
fn to_sliders(fit: &Fit) -> (i64, i64, i64, i64) {
    let amp = ((fit.a1 - 0.2) / 0.4 * 100.0).round() as i64;
    let freq = ((fit.f1 - 0.3) / 2.7 * 100.0).round() as i64;
    let phase = (fit.p1 / std::f32::consts::TAU * 100.0).round() as i64;
    let off = ((0.15 - fit.off) / 0.36 * 100.0).round() as i64;
    let clamp = |v: i64| v.clamp(0, 100);
    (clamp(amp), clamp(freq), clamp(phase), clamp(off))
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    // 用例图：817x559 第一关对话框特写，期望四滑块均落在 0~100
    #[test]
    fn wave_scan_fixture() {
        let bytes = include_bytes!("../tests/fixtures/level1-wave.png");
        let img = image::load_from_memory(bytes).expect("样张解码失败").to_rgba8();
        match solve_image(&img, false) {
            Ok(v) => {
                println!("wave 结果: {}", v);
                assert!(v["ok"].as_bool().unwrap(), "应求解成功: {}", v["msg"]);
                let s = v["sliders"].clone();
                for k in ["amp", "freq", "phase", "off"] {
                    let val = s[k].as_i64().expect("滑块应为整数");
                    assert!((0..=100).contains(&val), "滑块 {} 越界: {}", k, val);
                }
            }
            Err(e) => panic!("样张求解失败: {}", e),
        }
    }

    // v1.2.2 回归：2560x1440 全屏截图，游戏截图显示在 Honeyview 里（标题栏图标、
    // 浅灰界面、深灰背板全是橙色/暗块干扰源），且右下角工具窗口遮挡画布右缘。
    // 旧算法在此场景必失败（全屏橙像素分位把分带拉到整屏）；新算法应正确定位画布。
    #[test]
    fn wave_scan_fullscreen_overlay() {
        let bytes = include_bytes!("../tests/fixtures/level1-wave-fullscreen-overlay.png");
        let img = image::load_from_memory(bytes).expect("样张解码失败").to_rgba8();
        match solve_image(&img, false) {
            Ok(v) => {
                println!("wave 结果: {}", v);
                assert!(v["ok"].as_bool().unwrap(), "应求解成功: {}", v["msg"]);
                let s = v["sliders"].clone();
                for k in ["amp", "freq", "phase", "off"] {
                    let val = s[k].as_i64().expect("滑块应为整数");
                    assert!((0..=100).contains(&val), "滑块 {} 越界: {}", k, val);
                }
            }
            Err(e) => panic!("全屏干扰场景求解失败: {}", e),
        }
    }

    // 手动框选模式回归：从全屏干扰样张中裁出画布附近区域（模拟玩家框选，
    // 543x520 小图），放宽门槛后应求解成功且滑块与全屏模式一致
    #[test]
    fn wave_scan_manual_region() {
        let bytes = include_bytes!("../tests/fixtures/level1-wave-fullscreen-overlay.png");
        let img = image::load_from_memory(bytes).expect("样张解码失败").to_rgba8();
        // 画布实测在 x[1001,1544] y[627,774]，框选外扩一圈
        use image::GenericImageView;
        let crop: image::RgbaImage = img.view(900, 500, 700, 400).to_image();
        match solve_image(&crop, true) {
            Ok(v) => {
                println!("wave manual 结果: {}", v);
                assert!(v["ok"].as_bool().unwrap(), "手动框选模式应求解成功: {}", v["msg"]);
                let s = v["sliders"].clone();
                for k in ["amp", "freq", "phase", "off"] {
                    let val = s[k].as_i64().expect("滑块应为整数");
                    assert!((0..=100).contains(&val), "滑块 {} 越界: {}", k, val);
                }
            }
            Err(e) => panic!("手动框选模式求解失败: {}", e),
        }
    }

    // 真实游戏全屏截图（2560x1439，20261112 用例「第一关2」）：第一关面板悬浮在
    // 游戏场景中，画布外全是干扰（场景贴图、bilibili 水印、HUD），画布内有
    // 白/橙两条曲线（只认黄/橙目标线）
    #[test]
    fn wave_scan_real_fullscreen() {
        let bytes = include_bytes!("../tests/fixtures/level1-wave-real-fullscreen.png");
        let img = image::load_from_memory(bytes).expect("样张解码失败").to_rgba8();
        match solve_image(&img, false) {
            Ok(v) => {
                println!("wave 结果: {}", v);
                assert!(v["ok"].as_bool().unwrap(), "应求解成功: {}", v["msg"]);
                let s = v["sliders"].clone();
                for k in ["amp", "freq", "phase", "off"] {
                    let val = s[k].as_i64().expect("滑块应为整数");
                    assert!((0..=100).contains(&val), "滑块 {} 越界: {}", k, val);
                }
            }
            Err(e) => panic!("真实全屏截图求解失败: {}", e),
        }
    }


    // 合成信号自检：已知参数的双正弦应还原出对应滑块
    #[test]
    fn wave_synthetic_roundtrip() {
        let n = 500;
        let (a1, f1, p1, off, a2, f2, p2) =
            (0.35f32, 1.4f32, 1.1f32, -0.05f32, 0.08f32, 3.9f32, 2.2f32);
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let x = i as f32 / (n - 1) as f32;
                let y = off
                    + a1 * (std::f32::consts::TAU * f1 * x + p1).sin()
                    + a2 * (std::f32::consts::TAU * f2 * x + p2).sin();
                (x, y)
            })
            .collect();
        let fit = fit_two(&pts);
        println!(
            "synth: f1={:.3}(期望1.4) f2={:.3}(期望3.9) A1={:.3}(期望0.35) p1={:.3}(期望1.1) off={:.3}(期望-0.05)",
            fit.f1, fit.f2, fit.a1, fit.p1, fit.off
        );
        assert!((fit.f1 - f1).abs() < 0.03, "基波频率偏差过大");
        assert!((fit.a1 - a1).abs() < 0.02, "基波振幅偏差过大");
        assert!((fit.off - off).abs() < 0.02, "偏移偏差过大");
        let (amp, freq, _phase, _off) = to_sliders(&fit);
        println!("synth 滑块: amp={} freq={}", amp, freq);
        assert!((amp - 37).abs() <= 6, "振幅滑块偏差过大: {}", amp);
        assert!((freq - 41).abs() <= 3, "频率滑块偏差过大: {}", freq);
    }
}
