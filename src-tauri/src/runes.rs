// 符文自动识别管线（v1.1.0 起替代凯撒解密）：
// 全屏截图 → 定位「当前猜测」的 4 个符文框 → 逐框读边框反馈色 + 识别框内单字
// → 写静态 RUNE_SCAN → emit "rune_scan" → 弹结果窗口。
//
// 契约（PLAN-v1.1.0 第三节）：payload = { ok, slots:[{rune,color}], source, msg }
//   rune  0=空 1=火 2=水 3=风 4=土 5=雷（与 src/utils/runes.js 的 symbols 同序）
//   color 0=红 1=黄 2=绿（与 src/window/Runes 的 colors 同序），未识别为 null

use crate::solver::{capture_fullscreen, load_screenshot, ocr, to_png};
use crate::window::{result_window_notify, show_result_window};
use crate::APP;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, RgbaImage};
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

// 全屏截图缓存名（截图管线写入）；手动框选缓存名（截图窗口 cut_image 写入）
const FULL_SHOT: &str = "pot_screenshot_full.png";
const CUT_SHOT: &str = "pot_screenshot_cut.png";

// 符文名表：0=空 1=火 2=水 3=风 4=土 5=雷（与 src/utils/runes.js 的 symbols 同序）。
// rune_name 与 rune_from_text 共用这一份（原先两处各自内联、内容一致）；注意与
// simon.rs 的 SIMON_NAMES 语义不同（西蒙游戏是「火水风土雷光」六色，无「空」）
const RUNE_NAMES: [&str; 6] = ["空", "火", "水", "风", "土", "雷"];

// 最近一次扫描结果：结果窗口前端挂载后经 get_rune_scan 拉取，
// 避免 emit 早于前端监听注册时结果丢失（与旧管线同一竞态）
pub static RUNE_SCAN: Mutex<Option<Value>> = Mutex::new(None);

// 单飞保护：热键/托盘/HTTP/框选四入口可并发触发，并发截图会对同一缓存 PNG
// 交错写入（写坏文件 → 解码失败），进行中直接忽略新触发
static SCAN_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

// 手动框选去重：tauri v1 的 window.listen 不随窗口销毁清理，截图窗口反复新建
// 后旧监听器残留累积，同一次框选会触发多次 success；300ms 内的重复触发忽略
// （真实用户两次框选间隔远大于此）
static LAST_MANUAL_TRIGGER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

// 单个符文框的识别结果：两项都可独立缺失（缺失即 null，交前端人工纠正）
#[derive(Debug, Clone, Copy)]
struct Slot {
    rune: Option<usize>,
    color: Option<usize>,
}

// ---------------- 入口 ----------------

// 热键 / 托盘 / HTTP 主入口：全屏截图 + 弹结果窗口
pub fn rune_scan() {
    spawn(true, FULL_SHOT, "auto");
}

// 框选降级入口：结果窗口「OCR 框选」按钮 → window::ocr_recognize 拉框选窗 →
// 框选完成后对 pot_screenshot_cut.png 跑同一识别，payload source 标 manual
pub fn rune_scan_manual() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let prev = LAST_MANUAL_TRIGGER.swap(now, Ordering::SeqCst);
    if now.saturating_sub(prev) < 300 {
        return;
    }
    if SCAN_IN_FLIGHT.load(Ordering::SeqCst) {
        info!("[runes] 已有识别任务进行中，忽略本次手动框选触发");
        return;
    }
    spawn(false, CUT_SHOT, "manual");
}

fn spawn(capture: bool, shot: &'static str, source: &'static str) {
    if SCAN_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        info!("[runes] 已有识别任务进行中，忽略本次触发");
        return;
    }
    std::thread::spawn(move || {
        let payload = analyze(capture, shot, source);
        SCAN_IN_FLIGHT.store(false, Ordering::SeqCst);
        finish(payload);
    });
}

fn analyze(capture: bool, shot: &str, source: &str) -> Value {
    if capture {
        if let Err(e) = capture_fullscreen() {
            warn!("[runes] 全屏截图失败: {}", e);
            return payload(false, &[], source, &format!("全屏截图失败：{}", e));
        }
    }
    let img = match load_screenshot(shot) {
        Ok(v) => v,
        Err(e) => {
            warn!("[runes] 截图读取失败: {}", e);
            return payload(false, &[], source, &e);
        }
    };
    match scan_image(&img, source) {
        Ok(v) => v,
        Err(msg) => {
            warn!("[runes] 识别失败: {}", msg);
            payload(false, &[], source, &msg)
        }
    }
}

fn scan_image(img: &RgbaImage, source: &str) -> Result<Value, String> {
    let rects = locate(img)?;
    let mut slots = Vec::with_capacity(4);
    for (i, r) in rects.iter().enumerate() {
        let color = border_color(img, *r);
        let rune = identify_rune(img, *r);
        info!(
            "[runes] 框{} rect=({},{},{}x{}) 符文={:?} 反馈色={:?}",
            i,
            r.x,
            r.y,
            r.w,
            r.h,
            rune.map(rune_name),
            color.map(color_name)
        );
        slots.push(Slot { rune, color });
    }
    Ok(payload(true, &slots, source, ""))
}

fn payload(ok: bool, slots: &[Slot], source: &str, msg: &str) -> Value {
    json!({
        "ok": ok,
        "slots": slots
            .iter()
            .map(|s| json!({ "rune": s.rune, "color": s.color }))
            .collect::<Vec<Value>>(),
        "source": source,
        "msg": msg,
    })
}

fn finish(payload: Value) {
    // 锁中毒容错：某次扫描线程 panic 后锁永久中毒，unwrap 会让后续所有扫描失败；
    // unwrap_or_else 直接取回守门数据继续写（与 simon.rs 的 steps_lock 同策略）
    *RUNE_SCAN.lock().unwrap_or_else(|e| e.into_inner()) = Some(payload.clone());
    if let Some(app) = APP.get().cloned() {
        let _ = app.run_on_main_thread(move || {
            result_window_notify(payload);
            show_result_window();
        });
    }
}

fn rune_name(i: usize) -> &'static str {
    RUNE_NAMES.get(i).copied().unwrap_or("?")
}

fn color_name(i: usize) -> &'static str {
    const NAMES: [&str; 3] = ["红", "黄", "绿"];
    NAMES.get(i).copied().unwrap_or("?")
}

// ---------------- 像素判据（样张标定） ----------------

// 色相 0..360。整幅样张被视频压缩洗淡（纯白 bilibili 图标实测 (205,210,208)、
// 纯红报错字实测 (244,135,140)），故所有判据都按色相/通道差而不是绝对 RGB 写。
#[inline]
fn hue_of(r: u8, g: u8, b: u8) -> f32 {
    let (rf, gf, bf) = (r as f32, g as f32, b as f32);
    let mx = rf.max(gf).max(bf);
    let mn = rf.min(gf).min(bf);
    let d = mx - mn;
    if d <= 0.0 {
        return 0.0;
    }
    let h = if mx == rf {
        ((gf - bf) / d) % 6.0
    } else if mx == gf {
        (bf - rf) / d + 2.0
    } else {
        (rf - gf) / d + 4.0
    } * 60.0;
    if h < 0.0 {
        h + 360.0
    } else {
        h
    }
}

// 反馈色边框候选像素：色相落在 红[330,360)∪[0,25) / 黄[25,95) / 绿[95,196)。
// 样张实测（框边中段最亮 60% 像素的中位色）：
//   黄框 (152,158,112) h=67 · 绿框 (124,188,196) h=187 · 红框 (185,136,131) h=6
// 绿带上界 196 是关键：面板深蓝底 hue ≈ 204.5~216（旧样张 (14,53,80) 实测
// h=204.5，恰好压在旧上界 205 之内整片混进掩膜——连通域被底色黏连、环 bbox 被
// 撑大，边框带采样采空）；反馈青环 187 与面板底 204.5+ 的分界中点 ≈196。
// 亮度下限 90 剔暗底噪声（面板底 luma≈47，反馈框 luma≥150）。
#[inline]
fn is_colored_border(r: u8, g: u8, b: u8) -> bool {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    if mx < 90 || mx - mn < 18 {
        return false;
    }
    let h = hue_of(r, g, b);
    h < 196.0 || h >= 330.0
}

// 白/灰边框（新局尚未提交反馈色时的兜底）：彩度低 + 亮度高。
// 样张里没有这种框，该分支按几何判据兜底，未经实拍验证。
#[inline]
fn is_neutral_border(r: u8, g: u8, b: u8) -> bool {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    mx >= 150 && mx - mn <= 16
}

fn mask_of(img: &RgbaImage, colored: bool) -> Vec<bool> {
    let (w, h) = img.dimensions();
    let px = img.as_raw();
    let mut mask = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b) = (px[i], px[i + 1], px[i + 2]);
            let hit = if colored {
                is_colored_border(r, g, b)
            } else {
                is_neutral_border(r, g, b)
            };
            mask[(y * w + x) as usize] = hit;
        }
    }
    mask
}

// ---------------- 定位 ----------------

// 定位「当前猜测」的 4 个符文框：先按彩色边框找，找不到再按白/灰边框找。
// 判据全部相对几何（框宽/行高/间距互相约束），不写死绝对像素。
fn locate(img: &RgbaImage) -> Result<Vec<Rect>, String> {
    for colored in [true, false] {
        if let Some(quad) = find_quad(img, colored) {
            info!(
                "[runes] 定位到 4 个符文框（{}边框）: {:?}",
                if colored { "彩色" } else { "白/灰" },
                quad
            );
            return Ok(quad.to_vec());
        }
        info!(
            "[runes] {}边框掩膜未凑出 4 框组",
            if colored { "彩色" } else { "白/灰" }
        );
    }
    Err("未定位到「当前猜测」的 4 个符文框".into())
}

// 连通域找环形边框（v1.2.8 重写）：旧「行投影聚带 → 带内找竖边」扛不住大面积
// 彩色 UI——游戏面板的青色装饰线、青色实心按钮、绿色关卡标签让每行都有 ≥阈值
// 的彩色像素，行带全部黏成一条巨带，符文框行永远无法独立成带（20261113 1080p
// 实测裁切帧：彩色掩膜 7.3 万像素，最大行带 399px 高，旧法必败）。
// 连通域天然抗污染：反馈色边框是紧凑的空心环（面积/外接框 ≈ 0.2~0.5），实心
// 按钮/标签填充比 ≈ 1.0、装饰线外接框巨大，一滤即走。视频压缩打断的边框先
// 1px 膨胀桥接（旧样张实测黄色边框碎成多段，膨胀 + 8 连通可桥 ≤3px 断口）。
fn find_quad(img: &RgbaImage, colored: bool) -> Option<[Rect; 4]> {
    let (w, h) = img.dimensions();
    let mut mask = mask_of(img, colored);
    dilate1(&mut mask, w, h);
    let rings: Vec<Rect> = components(&mask, w, h)
        .into_iter()
        .filter_map(|c| {
            let (dx0, dy0, dx1, dy1) = dense_bbox(&c, w)?;
            let bw = (dx1 - dx0 + 1) as f32;
            let bh = (dy1 - dy0 + 1) as f32;
            // 外接框 ≥12px（字形可读下限）、≤ 图宽/6（框宽约占图宽 2%~8%）、
            // 近正方形、空心（填充比 ≤0.65 剔实心按钮/标签）
            if !(bw >= 12.0
                && bh >= 12.0
                && bw <= (w / 6) as f32
                && bh <= (w / 6) as f32
                && (bw / bh) > 0.4
                && (bw / bh) < 2.5
                && (c.area as f32 / (bw * bh)) <= 0.65)
            {
                return None;
            }
            // 膨胀把环外扩了整整 1px，内缩还原：否则边框采样带
            // （border_band）采到的是膨胀出来的暗背景，ring_contrast 必败
            Some(Rect {
                x: dx0 + 1,
                y: dy0 + 1,
                w: (dx1 - dx0).saturating_sub(1),
                h: (dy1 - dy0).saturating_sub(1),
            })
        })
        .collect();
    // 按 y 中心聚类成行（相邻中心差 ≤ 0.6×框高），行内按 x 序取 4 框组合逐一过
    // validate_quad（尺寸/节距/环亮度校验），首个通过者即「当前猜测」行
    let mut sorted = rings;
    sorted.sort_by_key(|r| (r.y + r.h / 2, r.x));
    let mut rows: Vec<Vec<Rect>> = Vec::new();
    for r in sorted {
        let cy = (r.y + r.h / 2) as f32;
        let rh = r.h as f32;
        match rows.last_mut() {
            Some(row)
                if {
                    let last = row.last().unwrap();
                    let lcy = (last.y + last.h / 2) as f32;
                    (cy - lcy).abs() <= 0.6 * rh.max(last.h as f32)
                } =>
            {
                row.push(r);
            }
            _ => rows.push(vec![r]),
        }
    }
    for row in &mut rows {
        // 聚类按 cy 邻接粘连，但 cy 有 ±1px 量化差会把 x 序打乱
        //（旧样张实测 442/443 交错），节距校验要求严格递增——按 x 重排
        row.sort_by_key(|r| r.x);
    }
    for row in &rows {
        let n = row.len();
        if n < 4 {
            continue;
        }
        for a in 0..n {
            for b in (a + 1)..n {
                for c in (b + 1)..n {
                    for d in (c + 1)..n {
                        let quad = [row[a], row[b], row[c], row[d]];
                        if let Some(q) = validate_quad(&quad, img) {
                            return Some(q);
                        }
                    }
                }
            }
        }
    }
    None
}

// 1px 8 邻域膨胀（就地）：桥接视频压缩打断的细边框（≤3px 断口）
fn dilate1(mask: &mut Vec<bool>, w: u32, h: u32) {
    let src = mask.clone();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if src[i] {
                continue;
            }
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(h - 1);
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);
            'outer: for ny in y0..=y1 {
                for nx in x0..=x1 {
                    if src[(ny * w + nx) as usize] {
                        mask[i] = true;
                        break 'outer;
                    }
                }
            }
        }
    }
}

struct Comp {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    area: u32,
    pixels: Vec<u32>,
}

// 8 连通域标记：返回每个域的包围盒、像素数与像素列表
//（栈式 DFS，1080p 裁切帧 <10ms；像素列表供稠密包围盒剔除离群桥点）
fn components(mask: &[bool], w: u32, h: u32) -> Vec<Comp> {
    let mut seen = vec![false; (w * h) as usize];
    let mut out = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    for start in 0..(w * h) as u32 {
        if seen[start as usize] || !mask[start as usize] {
            continue;
        }
        stack.clear();
        stack.push(start);
        seen[start as usize] = true;
        let (mut x0, mut y0, mut x1, mut y1, mut area) =
            (start % w, start / w, start % w, start / w, 0u32);
        let mut pixels: Vec<u32> = Vec::new();
        while let Some(p) = stack.pop() {
            area += 1;
            pixels.push(p);
            let (px, py) = (p % w, p / w);
            x0 = x0.min(px);
            x1 = x1.max(px);
            y0 = y0.min(py);
            y1 = y1.max(py);
            let dy0 = py.saturating_sub(1);
            let dy1 = (py + 1).min(h - 1);
            let dx0 = px.saturating_sub(1);
            let dx1 = (px + 1).min(w - 1);
            for ny in dy0..=dy1 {
                for nx in dx0..=dx1 {
                    let q = ny * w + nx;
                    if !seen[q as usize] && mask[q as usize] {
                        seen[q as usize] = true;
                        stack.push(q);
                    }
                }
            }
        }
        out.push(Comp { x0, y0, x1, y1, area, pixels });
    }
    out
}

// 稠密包围盒：按行/列统计域内像素数，剔除稀疏散点（边框晕点经膨胀与环黏连，
// 会把原始 bbox 撑大数 px，环不贴边后边框带采样必采空——旧样张框2 实测 bbox
// 上沿比可见环高 5px）。阈值 = max(5, 跨度/8)：环的边框行/列计数 ≥2×边框厚，
// 桥点行/列只有 1~4 个像素。
fn dense_bbox(comp: &Comp, w: u32) -> Option<(u32, u32, u32, u32)> {
    let bw = comp.x1 - comp.x0 + 1;
    let bh = comp.y1 - comp.y0 + 1;
    let thr_x = (bw / 8).max(5);
    let thr_y = (bh / 8).max(5);
    let mut rows = vec![0u32; bh as usize];
    let mut cols = vec![0u32; bw as usize];
    for &p in &comp.pixels {
        rows[(p / w - comp.y0) as usize] += 1;
        cols[(p % w - comp.x0) as usize] += 1;
    }
    let y0 = rows.iter().position(|&c| c >= thr_y)? as u32 + comp.y0;
    let y1 = rows.iter().rposition(|&c| c >= thr_y)? as u32 + comp.y0;
    let x0 = cols.iter().position(|&c| c >= thr_x)? as u32 + comp.x0;
    let x1 = cols.iter().rposition(|&c| c >= thr_x)? as u32 + comp.x0;
    Some((x0, y0, x1, y1))
}

// 4 连框的相对几何校验：尺寸相近、近正方形、同一行、节距均匀。
// 节距判据全相对 —— 框宽/节距 ∈[0.25,0.75]（样张 31/64≈0.48），节距互差 ≤35%，
// 分辨率/DPI 变化时这些比值不变。
fn validate_quad(win: &[Rect; 4], img: &RgbaImage) -> Option<[Rect; 4]> {
    let (wmin, wmax) = win.iter().fold((u32::MAX, 0u32), |(lo, hi), r| (lo.min(r.w), hi.max(r.w)));
    if wmax > wmin * 4 / 3 {
        return None;
    }
    let (hmin, hmax) = win.iter().fold((u32::MAX, 0u32), |(lo, hi), r| (lo.min(r.h), hi.max(r.h)));
    if hmax > hmin * 4 / 3 {
        return None;
    }
    // 近正方形：样张 31x34
    if !(0.65..=1.55).contains(&(wmax as f32 / hmax as f32)) {
        return None;
    }
    let side = wmax.max(hmax) as f32;
    let cy0 = (win[0].y + win[0].h / 2) as f32;
    // 便宜的算术校验（同行/节距）全部排在逐像素采样之前：find_quad 是 C(n,4)
    // 组合循环，ring_contrast 的逐像素开销只应花在通过全部算术校验的组合上
    //（校验彼此独立取与，调顺序不改变判定结果）
    for r in win.iter() {
        if ((r.y + r.h / 2) as f32 - cy0).abs() > 0.35 * side {
            return None;
        }
    }
    let mut pitches = [0i64; 3];
    for i in 0..3 {
        let p = win[i + 1].x as i64 - win[i].x as i64;
        if p <= 0 {
            return None;
        }
        pitches[i] = p;
    }
    let (pmin, pmax) = pitches.iter().fold((i64::MAX, 0i64), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    if pmax > pmin * 4 / 3 {
        return None;
    }
    for (_i, &p) in pitches.iter().enumerate() {
        if !(0.25..=0.75).contains(&(win[0].w as f32 / p as f32)) {
            return None;
        }
    }
    // 边框必须比框内亮：实心按钮/图标块在此被剔除（最贵的逐像素校验放最后）
    for r in win.iter() {
        if !ring_contrast(img, *r) {
            return None;
        }
    }
    Some(*win)
}

// ---------------- 反馈色 ----------------

// 边框带采样：四边中段（去掉角部 30%）的 bw 像素厚边沿。
// 带宽按框尺寸自适应（样张边框 2px，框宽 31px）
fn border_band(img: &RgbaImage, r: Rect) -> Vec<(u8, u8, u8)> {
    let bw = (r.w.min(r.h) / 16).clamp(1, 3);
    let sx = (r.w as f32 * 0.30) as u32;
    let sy = (r.h as f32 * 0.30) as u32;
    let mut out = Vec::new();
    let mut push = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        out.push((p[0], p[1], p[2]));
    };
    for x in (r.x + sx)..(r.x + r.w - sx) {
        for d in 0..bw {
            push(x, r.y + d);
            push(x, r.y + r.h - 1 - d);
        }
    }
    for y in (r.y + sy)..(r.y + r.h - sy) {
        for d in 0..bw {
            push(r.x + d, y);
            push(r.x + r.w - 1 - d, y);
        }
    }
    out
}

#[inline]
fn luma(p: (u8, u8, u8)) -> u32 {
    (p.0 as u32 * 2 + p.1 as u32 * 5 + p.2 as u32) / 8
}

// 边框比框内亮：边框带最亮 20% 分位亮度 vs 框内中央 50% 区域的中位亮度。
// 样张实测边框亮度 ~150、框内深蓝底 ~47；实心按钮两者相等，据此剔除。
fn ring_contrast(img: &RgbaImage, r: Rect) -> bool {
    let mut edge: Vec<u32> = border_band(img, r).into_iter().map(luma).collect();
    let mut inner: Vec<u32> = Vec::new();
    for y in (r.y + r.h / 4)..(r.y + r.h * 3 / 4) {
        for x in (r.x + r.w / 4)..(r.x + r.w * 3 / 4) {
            let p = img.get_pixel(x, y).0;
            inner.push(luma((p[0], p[1], p[2])));
        }
    }
    if edge.is_empty() || inner.is_empty() {
        return false;
    }
    edge.sort_unstable();
    inner.sort_unstable();
    let e = edge[edge.len() * 8 / 10];
    let i = inner[inner.len() / 2];
    e >= i + 20
}

// 边框反馈色：边框带丢掉最暗 40%（与暗底混合的抗锯齿外沿）后取亮度中位样本的
// 整体颜色 → 色相 → 红/黄/绿。必须是「中位样本」而非逐通道中位数——后者会拼出
// 画面里不存在的颜色（v1.2.8 回归：黄框逐通道中位 (134,140,130) diff=10 被判
// 白灰，实际像素 (134,140,105) h=87 是标准黄）。彩度不足 = 白灰边框（新局尚未
// 提交反馈色）→ null。
fn border_color(img: &RgbaImage, rect: Rect) -> Option<usize> {
    let mut samples = border_band(img, rect);
    if samples.is_empty() {
        return None;
    }
    samples.sort_by_key(|p| luma(*p));
    let (r, g, b) = samples[samples.len() * 2 / 5 + (samples.len() - samples.len() * 2 / 5) / 2];
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    // 样张三框彩度 46/72/54，白灰边框理论值 <12
    if mx - mn < 12 {
        return None;
    }
    let h = hue_of(r, g, b);
    let class = if h < 30.0 || h >= 330.0 {
        0 // 红：样张 (185,136,131) h=6
    } else if h < 95.0 {
        1 // 黄：样张 (152,158,112) h=67
    } else if h < 196.0 {
        2 // 绿/青：样张 (124,188,196) h=187；上界 196 与面板深蓝底分界（见 is_colored_border）
    } else {
        return None; // 蓝紫（面板底色）不可能是反馈色
    };
    info!(
        "[runes] 边框中位色 ({},{},{}) h={:.0} → {}",
        r,
        g,
        b,
        h,
        color_name(class)
    );
    Some(class)
}

// ---------------- 符文单字 ----------------

const GLYPH: usize = 16;
// 字形归一化放大倍数：框内单字原生只有 12~14px，先 4 倍双三次放大再二值紧裁，
// 避免 1px 量化噪声决定字形轮廓（与模板的生成流程必须一致，否则尺度对不上）
const GLYPH_UPSCALE: u32 = 4;
// 紧裁阈值（相对峰值）：0.30 实测能把字芯与底色分开（样张字芯峰值 ~150、
// 底色 ~10），同时保留笔画间的抗锯齿过渡
const GLYPH_CUT: f32 = 0.30;
// 模板匹配接受门限：样张 4 框的 Top-1 得分 0.71/0.84/0.83/0.82，
// 次高 0.49/0.47/0.47/0.46 —— 取 0.60 且领先次高 0.08 作为可用条件
const TEMPLATE_MIN: f32 = 0.60;
const TEMPLATE_MARGIN: f32 = 0.08;

// 6 个字形模板（空/火/水/风/土/雷），裁剪来源：实拍样张
// tests/fixtures/rune-panel-fullscreen.png 的符文按钮行（「空 火 水 风 土 雷」
// 六个白字，x 666..977 / y 494..516），按与识别端完全相同的流程处理：
// 4 倍双三次放大 → 0.30×峰值紧裁 → 16×16 双线性 → 归一化，
// 因此模板与框内字形同字体、同归一化尺度。灰度量化到 10 级字符：
// ' '=0 '.':0.11 ':':0.22 '-':0.33 '=':0.44 '+':0.56 '*':0.67 '#'=0.78 '%':0.89 '@'=1
const GLYPH_RAMP: &str = " .:-=+*#%@";
const RUNE_TEMPLATES: [[&str; GLYPH]; 6] = [
    // 0 空
    [
        ".::::-===-:::::.",
        "::---=***=-----:",
        ":=+++*#%#*=+++=-",
        "-+***####***##*=",
        "=*##%%#*+**#%%*=",
        "-+*#%%*==+*%@%+-",
        "-=+##*+--=+#@%+:",
        ":=+**+====+#%#=:",
        ":=++++++*++**+-.",
        ".:---=+##*+==-:.",
        "....:-+##+-::...",
        "    .:=##+:...  ",
        "    .:=##+:.....",
        "....:-+##+-::::.",
        ".::::-=**+----::",
        ".:::::-==--::::.",
    ],
    // 1 火
    [
        "     .:-=-:.    ",
        "     .=+*+-:.   ",
        "   ..:+##*=:....",
        " ....:*%%#+-::..",
        "..:::-*%@#+=+=-:",
        ".:==--#@@#****-:",
        ".-+*=-*@@%###+-:",
        ":=**=-*@@%##*=:.",
        "-+*+--#@@%#*=:..",
        "-===-=#%%%*+-:..",
        ":----+##***+-::.",
        "..::=*#*=+**+-:.",
        "..:-+*+=:-+**=-:",
        ".:-+++-:.:-+**=-",
        ":-===-:....-=+=-",
        "----:..   ..:---",
    ],
    // 2 水
    [
        "    .:-=-..  .. ",
        "   ..:=+=:......",
        " ...:-+**-:::::.",
        ".:::-=*##=--=-:.",
        "--==+*#%%*+++=:.",
        "-=+*#%%@@##*+-:.",
        ":-=+#%%@@%#+-:..",
        ".:-+###%@%#+:...",
        "..-+***#%%#+:...",
        ".:=**+*###*+=::.",
        ":-+++=+**+**+=::",
        "-=+=-=+**==+*+=:",
        "-==---+*+-:=++=-",
        "---::-++=:.:===-",
        "::..:-++-:..::::",
        ".....:--:.......",
    ],
    // 3 风
    [
        ".:-=---:----:.. ",
        ":-+++==-==++=:. ",
        ":=##*+==+*##+:. ",
        ":+%%#+=+*#%%*-..",
        ":+%@#*+*#%@%*-..",
        ":+%@%#*#%@@%*-..",
        ":+%%%#%@@%%%*-..",
        ":+%%##%@@#%%*-..",
        ":+%#**%@%##%*-..",
        ":+%#**#%%##%#-:.",
        ":+%#**###%%%#=::",
        ":+%#**++*##%#+-:",
        "-*#**=--=+**#*+-",
        "=+*+=-..:--+**+=",
        "=+=-:.   ..-=*+=",
        "--:..      .-==-",
    ],
    // 4 土
    [
        "    .:-==:..    ",
        "    .-+*+-..    ",
        "   .:-*#*=...   ",
        "   .:=#%#=...   ",
        "   .:=#%#=:.....",
        "....-=#%#+-:::..",
        ".::-=+%@%*=---:.",
        "::--=*%@%*==--:.",
        ":::-=*%@%*=---:.",
        "..::-+%@%+-:::..",
        "...:-+#@%+-::...",
        "....-=#%#+:::::.",
        "...:-=#%#+-:::::",
        ":::--+###+----=-",
        "----==***+======",
        "------====------",
    ],
    // 5 雷
    [
        " ..::---=---::..",
        "..:--==+*+==--:.",
        ".:-===+##*+==--:",
        ":-===+*%%#++===-",
        "-+****#%@#***++=",
        "=+*###%@@%%##*=-",
        "-=+**#%%@%###+-:",
        ".:-=++#%%#**+=-:",
        ".:-==+*#%***+=-:",
        ".:=+**#%%#**#*=:",
        ".:=*#*#%%%##%#=:",
        ".:+##*#%%###%#=:",
        ".:=****##***#*=:",
        " :=**++**++*#*-:",
        " :=++=--=-==*+-.",
        " .-=-::::::-==:.",
    ],
];

fn template(idx: usize) -> [[f32; GLYPH]; GLYPH] {
    let mut t = [[0f32; GLYPH]; GLYPH];
    let denom = (GLYPH_RAMP.len() - 1) as f32;
    for (y, row) in RUNE_TEMPLATES[idx].iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            t[y][x] = GLYPH_RAMP.find(c).unwrap_or(0) as f32 / denom;
        }
    }
    t
}

// 框内字形的分析结果：紧裁框（原图坐标）、底色白度、16×16 归一化签名
struct Glyph {
    bbox: Rect,
    bg: u8,
    sig: [[f32; GLYPH]; GLYPH],
}

// 框内字形定位 + 归一化。
// 白度取 min(R,G,B)：深蓝底上的浅色字（样张字芯 (179,192,215)）与白灰底上的
// 深色字都能拉开对比，比只看 G 通道稳。底色取框内中位数（字只占少量像素）。
// 峰值与紧裁都在「中央 20%~80% 窗口」内做：内缩后仍可能残留 1~2 行边框
// （样张各框边框+外发光宽 2~5px 不等），残留只在窗口外，不会被当成字形。
fn glyph_analysis(img: &RgbaImage, rect: Rect) -> Option<Glyph> {
    let inset = (rect.w.min(rect.h) / 8).clamp(3, 8);
    if rect.w <= inset * 2 + 8 || rect.h <= inset * 2 + 8 {
        return None;
    }
    let r = Rect {
        x: rect.x + inset,
        y: rect.y + inset,
        w: rect.w - inset * 2,
        h: rect.h - inset * 2,
    };
    let white = |x: u32, y: u32| -> u8 {
        let p = img.get_pixel(x, y).0;
        p[0].min(p[1]).min(p[2])
    };
    let mut vals: Vec<u8> = Vec::with_capacity((r.w * r.h) as usize);
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            vals.push(white(x, y));
        }
    }
    vals.sort_unstable();
    let bg = vals[vals.len() / 2];
    let mut luma = image::GrayImage::new(r.w, r.h);
    for y in 0..r.h {
        for x in 0..r.w {
            luma.put_pixel(x, y, image::Luma([white(r.x + x, r.y + y).saturating_sub(bg)]));
        }
    }
    let big = image::imageops::resize(
        &luma,
        r.w * GLYPH_UPSCALE,
        r.h * GLYPH_UPSCALE,
        FilterType::CatmullRom,
    );
    let (bw, bh) = big.dimensions();
    let wx0 = bw * 20 / 100;
    let wx1 = bw * 80 / 100;
    let wy0 = bh * 20 / 100;
    let wy1 = bh * 80 / 100;
    let mut peak = 0u8;
    for y in wy0..=wy1 {
        for x in wx0..=wx1 {
            peak = peak.max(big.get_pixel(x, y).0[0]);
        }
    }
    if peak < 24 {
        return None; // 框内几乎没有字（空框/糊掉）
    }
    let thr = (peak as f32 * GLYPH_CUT) as u8;
    let (mut x0, mut y0, mut x1, mut y1) = (wx1, wy1, wx0, wy0);
    let mut hits = 0u32;
    for y in wy0..=wy1 {
        for x in wx0..=wx1 {
            if big.get_pixel(x, y).0[0] >= thr {
                hits += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if hits < 8 || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let crop = image::imageops::crop_imm(&big, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    let small = image::imageops::resize(&crop, GLYPH as u32, GLYPH as u32, FilterType::Triangle);
    let mut sig = [[0f32; GLYPH]; GLYPH];
    let mut mx = 0f32;
    for (x, y, p) in small.enumerate_pixels() {
        let v = p.0[0] as f32;
        sig[y as usize][x as usize] = v;
        mx = mx.max(v);
    }
    if mx <= 0.0 {
        return None;
    }
    for row in sig.iter_mut() {
        for v in row.iter_mut() {
            *v /= mx;
        }
    }
    // 紧裁框映射回原图坐标（UI 后端只用于 log/OCR 裁图）
    let bbox = Rect {
        x: r.x + x0 / GLYPH_UPSCALE,
        y: r.y + y0 / GLYPH_UPSCALE,
        w: (x1 - x0) / GLYPH_UPSCALE + 1,
        h: (y1 - y0) / GLYPH_UPSCALE + 1,
    };
    Some(Glyph { bbox, bg, sig })
}

fn ncc(a: &[[f32; GLYPH]; GLYPH], b: &[[f32; GLYPH]; GLYPH]) -> f32 {
    let n = (GLYPH * GLYPH) as f32;
    let ma = a.iter().flatten().sum::<f32>() / n;
    let mb = b.iter().flatten().sum::<f32>() / n;
    let mut num = 0f32;
    let mut da = 0f32;
    let mut db = 0f32;
    for y in 0..GLYPH {
        for x in 0..GLYPH {
            let pa = a[y][x] - ma;
            let pb = b[y][x] - mb;
            num += pa * pb;
            da += pa * pa;
            db += pb * pb;
        }
    }
    if da <= 0.0 || db <= 0.0 {
        return 0.0;
    }
    num / (da.sqrt() * db.sqrt())
}

// 模板匹配 → (符文下标, Top-1 得分, 领先次高分的差值)
fn match_template(sig: &[[f32; GLYPH]; GLYPH]) -> (usize, f32, f32) {
    let mut scored: Vec<(f32, usize)> = (0..RUNE_TEMPLATES.len())
        .map(|i| (ncc(sig, &template(i)), i))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let best = scored[0];
    let second = scored.get(1).map(|s| s.0).unwrap_or(0.0);
    (best.1, best.0, best.0 - second)
}

// OCR 文本 → 6 字集合中的唯一下标（多字/集合外字符一律丢弃）
fn rune_from_text(text: &str) -> Option<usize> {
    let hits: Vec<usize> = text
        .chars()
        .filter_map(|c| RUNE_NAMES.iter().position(|n| n.starts_with(c)))
        .collect();
    if hits.len() == 1 {
        Some(hits[0])
    } else {
        None
    }
}

// 三种预处理候选喂 WinOCR：小字 OCR 对极性/对比度极敏感（旧管线实测同一张图
// 换预处理后识别率差别很大），逐一尝试直到命中 6 字集合。
// 实测结论（fixtures/rune-panel-fullscreen.png）：WinOCR 对框内 12~13px 的孤立
// 单字，在 2/4/8/12 倍放大、黑字白底/白字深底两种极性、8~32px 边距下全部返回空串；
// 同一引擎对多字文本行（「破解失败，请重试」）却能出字 —— 缺行内上下文是硬伤。
// 故模板匹配是主判据，OCR 只作一致性校验（见 identify_rune）。
fn ocr_candidates(img: &RgbaImage, g: &Glyph) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    if g.bbox.w < 4 || g.bbox.h < 4 {
        return out;
    }
    let crop: RgbaImage = img
        .view(g.bbox.x, g.bbox.y, g.bbox.w, g.bbox.h)
        .to_image();
    // 候选 1：白度归一化（底→白、字→黑），黑字白底是 OCR 的偏好极性
    let mut norm = crop.clone();
    let mut peak = 1u32;
    for p in norm.pixels() {
        peak = peak.max(p[0].min(p[1]).min(p[2]).saturating_sub(g.bg) as u32);
    }
    for p in norm.pixels_mut() {
        let w = p[0].min(p[1]).min(p[2]).saturating_sub(g.bg) as u32;
        let v = (255 - (w * 255 / peak).min(255)) as u8;
        *p = image::Rgba([v, v, v, 255]);
    }
    let norm = DynamicImage::ImageRgba8(norm).resize_exact(
        g.bbox.w * GLYPH_UPSCALE,
        g.bbox.h * GLYPH_UPSCALE,
        FilterType::CatmullRom,
    );
    let white = image::Rgba([255, 255, 255, 255]);
    if let Ok(png) = to_png(&crate::solver::pad_canvas(&norm, 16, white)) {
        out.push(png.clone());
        // 候选 2：同一极性的硬二值（去掉抗锯齿灰阶，笔画更实）。
        // 旧实现误反相，WinOCR 偏好黑字白底；norm 已是黑字白底，保持同极性：
        // 背景(≥128)→白 255、笔画(<128)→黑 0
        let mut bin = norm.to_rgba8();
        for p in bin.pixels_mut() {
            let v = if p[0] >= 128 { 255u8 } else { 0u8 };
            *p = image::Rgba([v, v, v, 255]);
        }
        if let Ok(png) = to_png(&crate::solver::pad_canvas(
            &DynamicImage::ImageRgba8(bin),
            16,
            white,
        )) {
            out.push(png);
        }
    }
    // 候选 3：原样放大 + 同底色边距（浅字深底，交给引擎自己处理极性）
    let bg = *img.get_pixel(g.bbox.x, g.bbox.y);
    let raw = DynamicImage::ImageRgba8(crop).resize_exact(
        g.bbox.w * GLYPH_UPSCALE,
        g.bbox.h * GLYPH_UPSCALE,
        FilterType::CatmullRom,
    );
    if let Ok(png) = to_png(&crate::solver::pad_canvas(&raw, 16, bg)) {
        out.push(png);
    }
    out
}

fn ocr_rune(img: &RgbaImage, g: &Glyph) -> Option<usize> {
    let engine = ocr::zh_engine()?;
    for (i, png) in ocr_candidates(img, g).iter().enumerate() {
        match ocr::recognize_png(&engine, png) {
            Ok(text) => {
                if let Some(idx) = rune_from_text(&text) {
                    info!("[runes] OCR 候选{} 命中: {}", i + 1, rune_name(idx));
                    return Some(idx);
                }
                // 隐私边界：只记文本长度，不落盘 OCR 原始内容
                info!(
                    "[runes] OCR 候选{} 未归一（{} 字）",
                    i + 1,
                    text.chars().count()
                );
            }
            Err(e) => info!("[runes] OCR 候选{} 失败: {}", i + 1, e),
        }
    }
    None
}

// 框内符文识别：模板匹配为主判据（样张实测 4/4 正确，Top-1 得分 0.75~0.85、
// 领先次高 0.23~0.36），WinOCR 作为一致性校验——两者都有结果且不一致时置 null
// 交人工纠正；模板置信度不足时反而回落到 OCR。实测 WinOCR 对孤立单字返回空串，
// 因此本轮实际由模板独自给出结论（见 ocr_candidates 的实测记录）。
fn identify_rune(img: &RgbaImage, rect: Rect) -> Option<usize> {
    let glyph = glyph_analysis(img, rect)?;
    let (tidx, score, margin) = match_template(&glyph.sig);
    let tpl_hit = if score >= TEMPLATE_MIN && margin >= TEMPLATE_MARGIN {
        info!(
            "[runes] 模板 Top-1 {} 得分 {:.3} 领先 {:.3}",
            rune_name(tidx),
            score,
            margin
        );
        Some(tidx)
    } else {
        info!(
            "[runes] 模板 Top-1 {} 得分 {:.3} 领先 {:.3}（低于门限 {}/{}）",
            rune_name(tidx),
            score,
            margin,
            TEMPLATE_MIN,
            TEMPLATE_MARGIN
        );
        None
    };
    let ocr = ocr_rune(img, &glyph);

    match (tpl_hit, ocr) {
        (Some(a), Some(b)) if a != b => {
            warn!(
                "[runes] 模板({}) 与 OCR({}) 不一致，置 null 交人工纠正",
                rune_name(a),
                rune_name(b)
            );
            None
        }
        (Some(a), _) => Some(a),
        // 模板置信度不足时用 OCR 兜底
        (None, Some(b)) => {
            info!("[runes] 模板不足信，采用 OCR 结果 {}", rune_name(b));
            Some(b)
        }
        (None, None) => None,
    }
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> RgbaImage {
        let bytes = include_bytes!("../tests/fixtures/rune-panel-fullscreen.png");
        image::load_from_memory(bytes)
            .expect("样张解码失败")
            .to_rgba8()
    }

    // 模板 Top-3 得分（人工核对置信度用）
    fn template_top3(g: &Glyph) -> Vec<String> {
        let mut v: Vec<(f32, usize)> = (0..RUNE_TEMPLATES.len())
            .map(|k| (ncc(&g.sig, &template(k)), k))
            .collect();
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter()
            .take(3)
            .map(|(s, k)| format!("{}:{:.2}", rune_name(k), s))
            .collect()
    }

    // OCR 三个候选的原始文本（对比模板判据用）
    fn ocr_raw(img: &RgbaImage, g: &Glyph) -> Vec<String> {
        match ocr::zh_engine() {
            Some(e) => ocr_candidates(img, g)
                .iter()
                .enumerate()
                .map(|(i, png)| match ocr::recognize_png(&e, png) {
                    Ok(t) => format!("c{}={:?}", i + 1, t),
                    Err(err) => format!("c{}=ERR({})", i + 1, err),
                })
                .collect(),
            None => vec!["无中文 OCR 引擎".to_string()],
        }
    }

    // 20261113 用户实拍（1080p 视频 t=94s 裁切区域）：四个符文框边框全彩
    // （土=黄 风=红 雷=绿 水=红），旧样张全部判据应照常工作
    #[test]
    fn rune_scan_live_crop_20261113() {
        let img = image::load_from_memory(include_bytes!(
            "../tests/fixtures/level3-cipher-crop-20261113.jpg"
        ))
        .expect("样张解码失败")
        .to_rgba8();
        let rects = locate(&img).expect("实测裁切帧定位失败");
        let mut runes = Vec::new();
        let mut colors = Vec::new();
        for (i, r) in rects.iter().enumerate() {
            let color = border_color(&img, *r);
            let rune = identify_rune(&img, *r);
            println!("框{} rect=({},{},{}x{}) 符文={:?} 色={:?}", i, r.x, r.y, r.w, r.h,
                rune.map(rune_name), color.map(color_name));
            runes.push(rune.map(|v| rune_name(v).to_string()));
            colors.push(color.map(|v| color_name(v).to_string()));
        }
        assert_eq!(runes, vec![Some("土".into()), Some("风".into()), Some("雷".into()), Some("水".into())]);
        // 边框色实测（逐框最亮边框像素）：土 (241,255,176) h=71 黄 ·
        // 风 (205,156,162) h=353 红 · 雷 (239,255,180) h=73 黄 · 水 (199,157,161) h=354 红
        assert_eq!(colors, vec![Some("黄".into()), Some("红".into()), Some("黄".into()), Some("红".into())]);
    }

    // 样张：1645×926 全屏实拍，「当前猜测」= 雷 风 风 风。    // 真值是 黄 绿 黄 红，不是早期文档写的「黄 黄 红 红」——依据：四个框边框逐像素
    // 采样（四边中段、去掉最暗 40% 的抗锯齿外沿）中位色分别为
    //   (152,158,112) h=67 黄 · (124,188,196) h=187 青绿 · (124,135,111) h=87 黄 · (185,136,131) h=6 红
    // 第 2 框青色通道 B=196 明显高于 R=124，不可能是黄；且「雷风风风 + 黄绿黄红」在
    // 按数量(count)判定下自洽（pos2 绿=位置正确；pos3 黄=另一个风存在但错位；
    // pos4 红=风已用尽），在 presence 判定下无解。改断言前请先复测这三个色相的像素值。
    #[test]
    fn rune_scan_fixture() {
        let img = fixture();
        let rects = locate(&img).expect("样张上定位失败");
        assert_eq!(rects.len(), 4, "应定位到 4 个符文框");
        println!("== 定位结果 ==");
        for (i, r) in rects.iter().enumerate() {
            println!("框{} rect=({},{},{}x{})", i, r.x, r.y, r.w, r.h);
        }
        // 相对几何自检：框宽相近 + 近正方形
        let side = rects[0].w as f32;
        for r in rects.iter() {
            assert!((r.w as f32 / side) < 1.35, "框宽差异过大: {:?}", rects);
            assert!((r.h as f32 / r.w as f32) > 0.7, "框不是近正方形: {:?}", r);
        }

        println!("== 逐框识别 ==");
        let mut runes = Vec::new();
        let mut colors = Vec::new();
        for (i, r) in rects.iter().enumerate() {
            let color = border_color(&img, *r);
            let glyph = glyph_analysis(&img, *r);
            let scored = glyph.as_ref().map(template_top3).unwrap_or_default();
            let raw = glyph
                .as_ref()
                .map(|g| ocr_raw(&img, g))
                .unwrap_or_else(|| vec!["无字形".to_string()]);
            let rune = identify_rune(&img, *r);
            println!(
                "框{} 符文={} 反馈色={} | 模板Top3={:?} OCR={:?}",
                i,
                rune.map(rune_name).unwrap_or("null"),
                color.map(color_name).unwrap_or("null"),
                scored,
                raw
            );
            runes.push(rune.map(|v| rune_name(v).to_string()));
            colors.push(color.map(|v| color_name(v).to_string()));
        }
        println!("符文序列: {:?}", runes);
        println!("反馈色序: {:?}", colors);

        // 断言只钉住样张实测到的真实内容（不按任务书期望值反向凑）
        assert_eq!(
            runes,
            vec![
                Some("雷".to_string()),
                Some("风".to_string()),
                Some("风".to_string()),
                Some("风".to_string())
            ],
            "符文识别不符样张实际内容"
        );
        assert_eq!(
            colors,
            vec![
                Some("黄".to_string()),
                Some("绿".to_string()),
                Some("黄".to_string()),
                Some("红".to_string())
            ],
            "反馈色识别不符样张实际内容"
        );
    }

    // 模板自洽性：每个模板与自身 NCC = 1 且互为最优
    #[test]
    fn rune_templates_self_consistent() {
        for (i, rows) in RUNE_TEMPLATES.iter().enumerate() {
            assert_eq!(rows.len(), GLYPH, "模板{} 行数不对", i);
            for row in rows.iter() {
                assert_eq!(row.chars().count(), GLYPH, "模板{} 行宽不对: {}", i, row);
                assert!(
                    row.chars().all(|c| GLYPH_RAMP.contains(c)),
                    "模板{} 含非法字符: {}",
                    i,
                    row
                );
            }
            let t = template(i);
            let (idx, score, _) = match_template(&t);
            assert_eq!(idx, i, "模板{} 与自身匹配落到 {}", i, idx);
            assert!(score > 0.99, "模板{} 自匹配得分 {}", i, score);
        }
    }

    // 未提交反馈色时只有白/灰边框：色相判据不能把它当成三色之一
    #[test]
    fn rune_neutral_border_not_colored() {
        for v in [150u8, 200, 255] {
            assert!(!is_colored_border(v, v, v), "白灰 {} 被判成彩色边框", v);
            assert!(is_neutral_border(v, v, v), "白灰 {} 未进白灰掩膜", v);
        }
        // 面板深蓝不能进彩色掩膜
        assert!(!is_colored_border(23, 49, 87));
    }
}
