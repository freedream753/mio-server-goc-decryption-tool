// 新版解题页面自动解题管线（v1.0.3 方案 A，全 Rust）：
//   触发 -> 全屏截图 -> 像素预筛定位三卡 -> 裁「标签行/密文行」ROI
//   -> Windows.Media.Ocr 识别 -> 按标签语义解密（后移N位→-N / 前移N位→+N）
//   -> 词表校验提示 -> 自学习历史优先 -> 结果窗口展示 + 默认复制第一个答案
// 失败处理：定位/识别失败或系统无中文 OCR 语言包时，直接在结果窗口提示错误
// （已放弃回退旧手动框选管线的托底，该方式意义不大）。
// 日志隐私边界：只记阶段与失败原因，不落盘识别出的文本内容。

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, RgbaImage};
use log::{error, info, warn};
use serde_json::{json, Value};

use crate::window::{result_window_notify, SOLVE_RESULT};
use crate::APP;

// 识别预处理参数（2026-11-03 全图适配重写：定位判定全部改为相对色序，禁用绝对 RGB 阈值）
// 背景：同一版对话框在不同渲染环境（英伟达动态亮丽 / 游戏内增亮滤镜 / JPG 压缩）
// 下颜色整体漂移（实测页面背景 (16,42,66)~(2,38,94)、输入框 (20,28,48)~(0,0,40)），
// 且实机常开滤镜继续推移亮度与饱和度——单调变换不改变颜色间的相对序，
// 故只允许「通道差 + 跨色相对序 + 模式色容差」三类判定。
const LABEL_UPSCALE: u32 = 4; // 标签行小字：4 倍放大 + G 通道二值化
const CIPHER_UPSCALE: u32 = 2; // 密文行大字：2 倍放大原样识别
const WORD_MIN: usize = 3; // 词长下限（旧管线 [A-Z]{3,8} 会截断 9 字母词）
const WORD_MAX: usize = 12; // 词长上限

// 词表仅做答案校验提示（✓ 命中 / ？未验证），不做暴力破解
const WORDS: &[&str] = &[
    "NEGATIVE", "POSITIVE", "BALANCE", "EFFICIENT", "AWARE", "BRAVE", "BREAK", "BITTER", "HURT",
    "AREA", "CALM", "CHAOS", "ORDER", "PEACE", "TRUST", "FAITH", "LOYAL", "HONEST", "KIND",
    "NOBLE", "PROUD", "SHAME", "GUILT", "PRIDE", "FEAR", "HOPE", "DOOM", "FATE", "LIFE", "DEATH",
    "LIGHT", "DARK", "SHADOW", "DAWN", "DUSK", "STORM", "FROST", "FLAME", "EMBER", "ASH", "DUST",
    "STONE", "STEEL", "IRON", "GOLD", "SILVER", "BLOOD", "WOUND", "HEAL", "CURE", "POISON",
    "VENOM", "TOXIC", "PURE", "CORRUPT", "TAINTED", "BLESSED", "CURSED", "SACRED", "PROFANE",
    "MORTAL", "DIVINE", "HUMAN", "MONSTER", "BEAST", "ANGEL", "DEMON", "GHOST", "SOUL", "MIND",
    "BODY", "HEART", "STRENGTH", "WEAKNESS", "COURAGE", "WISDOM", "FOLLY", "JUSTICE", "MERCY",
    "WRATH", "GRACE", "SILENCE", "ECHO", "VOICE", "SONG", "HUNGER", "SLEEP", "DREAM", "MEMORY",
    "LOSS", "GAIN", "PROFIT", "DEBT", "PRICE", "COST", "WORTH", "VALUE", "SEED", "ROOT", "BRANCH",
    "ILLUSTRATE", "RESTRICT", "ELIMINATE",
];

// 入口：热键 / 托盘 / HTTP 主路由。截图 + 弹结果窗口
pub fn solver_recognize() {
    std::thread::spawn(|| solve_flow(true, true));
}

// HTTP 隐藏验证入口：不截屏、不弹窗，直接对已有 pot_screenshot_full.png 跑管线，
// 第一个答案写剪贴板，供无感回归测试读取
pub fn solver_recognize_hidden() {
    std::thread::spawn(|| solve_flow(false, false));
}

fn solve_flow(capture: bool, show: bool) {
    match run_solve(capture) {
        Ok(payload) => {
            if show {
                if let Some(app) = APP.get().cloned() {
                    let _ = app.run_on_main_thread(move || {
                        result_window_notify(payload);
                    });
                }
            }
        }
        Err(e) => {
            // 失败不再回退手动框选，直接在结果窗口提示错误
            error!("[solver] 自动管线失败: {}", e);
            if show {
                let payload = json!({
                    "cards": [{
                        "cipher": "--",
                        "label": "",
                        "answer": "",
                        "status": "error",
                        "msg": format!("自动识别失败：{}", e),
                    }],
                    "show": true,
                });
                *SOLVE_RESULT.lock().unwrap() = Some(payload.clone());
                if let Some(app) = APP.get().cloned() {
                    let _ = app.run_on_main_thread(move || {
                        result_window_notify(payload);
                    });
                }
            }
        }
    }
}

// Ok(payload) = 解题成功（或带错误卡片的提示 payload）
fn run_solve(capture: bool) -> Result<Value, String> {
    if capture {
        capture_fullscreen()?;
    }
    let img = load_screenshot()?;
    let zh = ocr::zh_engine().ok_or("系统缺少中文 OCR 语言包")?;
    let en = ocr::en_engine();
    let rows = solve_image(&img, &zh, en.as_ref())?;
    if rows.is_empty() {
        return Err("所有卡片识别失败".into());
    }
    // 默认复制第一个有效答案
    let first = rows.iter().find(|r| r["status"] != json!("error"));
    if let Some(r) = first {
        if let Some(ans) = r["answer"].as_str() {
            if !ans.is_empty() {
                if let Some(app) = APP.get() {
                    use tauri::ClipboardManager;
                    if let Err(e) = app.clipboard_manager().write_text(ans) {
                        warn!("[solver] 剪贴板写入失败: {}", e);
                    }
                }
            }
        }
    }
    let payload = json!({ "cards": rows, "show": true });
    *SOLVE_RESULT.lock().unwrap() = Some(payload.clone());
    Ok(payload)
}

// ---------------- 截图与加载 ----------------

fn cache_file(name: &str) -> Result<PathBuf, String> {
    let app = APP.get().ok_or("app not initialized")?;
    let dir = dirs::cache_dir().ok_or("cache dir not found")?;
    Ok(dir.join(&app.config().tauri.bundle.identifier).join(name))
}

fn capture_fullscreen() -> Result<(), String> {
    use mouse_position::mouse_position::Mouse;
    use screenshots::{Compression, Screen};

    let (mx, my) = match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => (x, y),
        Mouse::Error => (0, 0),
    };
    let screens = Screen::all().map_err(|e| e.to_string())?;
    for screen in screens {
        let d = screen.display_info;
        let (dx, dy) = (d.x, d.y);
        let (dw, dh) = (d.width as i32, d.height as i32);
        if mx >= dx && my >= dy && mx < dx + dw && my < dy + dh {
            let img = screen.capture().map_err(|e| e.to_string())?;
            let buf = img.to_png(Compression::Fast).map_err(|e| e.to_string())?;
            let path = cache_file("pot_screenshot_full.png")?;
            std::fs::write(&path, buf).map_err(|e| e.to_string())?;
            info!("[solver] 全屏截图完成: {:?}", path);
            return Ok(());
        }
    }
    Err(format!("mouse monitor not found: {},{}", mx, my))
}

fn load_screenshot() -> Result<RgbaImage, String> {
    let path = cache_file("pot_screenshot_full.png")?;
    if !path.exists() {
        return Err("全屏截图缓存不存在".into());
    }
    // 按内容嗅探格式（兼容测试时直接拷入的 JPG），不依赖扩展名
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let img = reader.decode().map_err(|e| format!("截图解码失败: {}", e))?;
    Ok(img.to_rgba8())
}

// ---------------- 像素预筛定位 ----------------

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Debug, Clone, Copy)]
struct Band {
    start: u32,
    end: u32, // 闭区间
}

#[derive(Debug)]
struct CardRoi {
    label: Rect,
    cipher: Rect,
    text_thr: u32, // 该卡的文字判定阈值（随卡片底色自适应）
    card_g: u8,    // 卡片底色 G 通道（灰度归一化的白场基准）
}

struct Scanner<'a> {
    w: u32,
    h: u32,
    px: &'a [u8],
}

// 页面背景众数 P 与输入框均值 in 的运行时测量值（u32 便于比较运算）
type Rgb = (u32, u32, u32);

impl<'a> Scanner<'a> {
    fn new(img: &'a RgbaImage) -> Self {
        let (w, h) = img.dimensions();
        Scanner { w, h, px: img.as_raw() }
    }

    #[inline]
    fn rgb(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * self.w + x) * 4) as usize;
        (self.px[i], self.px[i + 1], self.px[i + 2])
    }

    // 蓝暗色（B 为主导通道的中暗色）：对话框/UI 面板族的颜色特征。
    // 只用通道差，亮度/饱和度滤镜下稳定。纯灰、亮墙、铠甲等 B≈G 的颜色被排除。
    #[inline]
    fn is_blue_dark(&self, x: u32, y: u32) -> bool {
        let (r, g, b) = self.rgb(x, y);
        b >= r + 20 && b >= g + 12
    }

    // 输入框内部：显著比页面背景 P 更暗（B、G 双通道 ≤ 0.85×P）且保持蓝主导。
    // 滤镜漂移两端实测 (0,0,40)~(20,28,48) vs P(2,38,94)~(16,42,66) 均满足。
    #[inline]
    fn is_input_rel(&self, x: u32, y: u32, p: Rgb) -> bool {
        let (r, g, b) = self.rgb(x, y);
        b >= r + 12 && (b as u32) * 20 <= p.2 * 17 && (g as u32) * 20 <= p.1 * 17
    }

    // 亮色文字（相对页面背景 P 自适应，替代固定 G 阈值）
    #[inline]
    fn is_text(&self, x: u32, y: u32, thr: u32) -> bool {
        let (_, g, _) = self.rgb(x, y);
        g as u32 >= thr
    }
}

// 行/列投影聚带：count >= thr 视为命中，允许 gap 个连续未命中行桥接，最小高度 min_h
fn merge_bands(counts: &[u32], thr: u32, gap: u32, min_h: u32) -> Vec<Band> {
    let mut bands = Vec::new();
    let mut start: Option<u32> = None;
    let mut miss = 0u32;
    for (i, &c) in counts.iter().enumerate() {
        if c >= thr {
            if start.is_none() {
                start = Some(i as u32);
            }
            miss = 0;
        } else if start.is_some() {
            miss += 1;
            if miss > gap {
                let s = start.unwrap();
                let e = i as u32 - miss;
                if e + 1 - s >= min_h {
                    bands.push(Band { start: s, end: e });
                }
                start = None;
                miss = 0;
            }
        }
    }
    if let Some(s) = start {
        let e = counts.len() as u32 - 1 - miss;
        if e + 1 - s >= min_h {
            bands.push(Band { start: s, end: e });
        }
    }
    bands
}

fn detect_cards(img: &RgbaImage) -> Result<Vec<CardRoi>, String> {
    let sc = Scanner::new(img);
    let (w, h) = (sc.w, sc.h);
    if w < 200 || h < 200 {
        return Err("截图尺寸异常".into());
    }

    // 1) 蓝暗色行投影 -> 最强带 = 对话框纵向范围（场景中灰/铠甲等颜色不参与）
    let mut row_blue = vec![0u32; h as usize];
    for y in 0..h {
        let mut c = 0u32;
        for x in (0..w).step_by(2) {
            if sc.is_blue_dark(x, y) {
                c += 2;
            }
        }
        row_blue[y as usize] = c;
    }
    let blue_bands = merge_bands(&row_blue, w / 6, 40, 200);
    let dialog = blue_bands
        .iter()
        .max_by_key(|b| row_blue[b.start as usize..=b.end as usize].iter().sum::<u32>())
        .ok_or("未定位到对话框区域")?;
    let (dy0, dy1) = (dialog.start, dialog.end);

    // 2) 纵向带内列投影 -> 对话框横向包络（取最强带，排除同色场景区块）
    let mut col_blue = vec![0u32; w as usize];
    for y in dy0..=dy1 {
        for x in 0..w {
            if sc.is_blue_dark(x, y) {
                col_blue[x as usize] += 1;
            }
        }
    }
    let dlg_h = dy1 - dy0 + 1;
    let col_bands = merge_bands(&col_blue, dlg_h / 2, 20, dlg_h / 2);
    let dx = col_bands
        .iter()
        .max_by_key(|b| col_blue[b.start as usize..=b.end as usize].iter().sum::<u32>())
        .ok_or("未定位到对话框横向范围")?;
    let (dx0, dx1) = (dx.start, dx.end);
    let dlg_w = dx1 - dx0 + 1;

    // 3) 对话框内蓝暗色直方图众数 = 页面背景 P（内缩 8px 避开边框阴影），作为全流程自适应基准
    let mut hist: HashMap<u32, u32> = HashMap::new();
    for y in (dy0 + 8)..=dy1.saturating_sub(8) {
        for x in (dx0 + 8)..=dx1.saturating_sub(8) {
            if sc.is_blue_dark(x, y) {
                let (r, g, b) = sc.rgb(x, y);
                let key = ((r as u32 / 4) << 16) | ((g as u32 / 4) << 8) | (b as u32 / 4);
                *hist.entry(key).or_insert(0) += 1;
            }
        }
    }
    let (&pkey, _) = hist.iter().max_by_key(|(_, &c)| c).ok_or("对话框颜色统计失败")?;
    let p: Rgb = (
        ((pkey >> 16) & 0xff) * 4 + 2,
        ((pkey >> 8) & 0xff) * 4 + 2,
        (pkey & 0xff) * 4 + 2,
    );

    // 4) 输入框带：显著比页面背景更暗（B、G ≤ 0.85×P）。按「最长连续暗段」计行数——
    //    输入框是约 1/4 框宽的连续暗块，而对话框边框/卡间阴影只是细条，
    //    按总像素计数会被边框+阴影凑满阈值（JPG 特写实测踩坑）
    let mut row_dark = vec![0u32; h as usize];
    for y in dy0..=dy1 {
        let mut run = 0u32;
        let mut best = 0u32;
        for x in dx0..=dx1 {
            if sc.is_input_rel(x, y, p) {
                run += 1;
                if run > best {
                    best = run;
                }
            } else {
                run = 0;
            }
        }
        row_dark[y as usize] = best;
    }
    let dark_bands = merge_bands(&row_dark, dlg_w / 6, 6, 8);
    // 输入框带高度 ≈ 框宽的 4.4%（全图 53/1204、特写 40/902，跨缩放一致）。
    // 标题栏区、底部面板、场景暗块也会形成长暗段（特写图底部全宽暗区 run=899
    // 远大于输入框 run=232），故按「带高 ≤ 框宽/15」过滤后，按暗段质量降序
    // 逐带尝试聚簇，取第一个结构合法（1~3 簇、单簇 ≤35% 框宽）的带
    let mut input_xy: Option<(u32, u32)> = None;
    let mut clusters: Vec<Band> = Vec::new();
    let mut cands: Vec<&Band> = dark_bands.iter().collect();
    cands.sort_by_key(|b| {
        std::cmp::Reverse(row_dark[b.start as usize..=b.end as usize].iter().sum::<u32>())
    });
    for band in cands {
        let (y0, y1) = (band.start, band.end);
        let hh = y1 - y0 + 1;
        if hh < 10 || hh > dlg_w / 15 {
            continue;
        }
        let mut col_dark = vec![0u32; w as usize];
        for y in y0..=y1 {
            for x in dx0..=dx1 {
                if sc.is_input_rel(x, y, p) {
                    col_dark[x as usize] += 1;
                }
            }
        }
        let col_thr = (hh as f64 * 0.5) as u32;
        let cl = merge_bands(&col_dark, col_thr, 12, (hh as f64 * 0.3) as u32);
        let max_w = cl
            .iter()
            .map(|b| b.end - b.start + 1)
            .max()
            .unwrap_or(0);
        let cl: Vec<Band> = cl
            .iter()
            .filter(|b| {
                let bw = b.end - b.start + 1;
                bw >= max_w * 35 / 100 && bw <= dlg_w * 35 / 100
            })
            .copied()
            .collect();
        if cl.is_empty() || cl.len() > 3 {
            continue;
        }
        clusters = cl;
        input_xy = Some((y0, y1));
        break;
    }
    let (in_y0, in_y1) = input_xy.ok_or("未定位到输入框区域")?;
    let in_h = in_y1 - in_y0 + 1;
    if in_h < 10 {
        return Err("输入框区域高度异常".into());
    }

    // 6) 逐簇在输入框上方聚亮色文字带（标题 / 标签 / 密文）。
    //    扫描上界 = max(对话框顶 + 1/10框高, 输入框上沿 - 8×框高)：
    //    排除窗口标题栏文字/控制按钮（实测会以更高带抢走密文选择），
    //    同时保证覆盖卡内文字区（≈5~6.5×框高）。不做卡片顶边检测——
    //    部分配色下卡片底色与页面底色几乎同值（JPG 特写 B 通道仅差 ~4），顶边判定不可靠。
    //    密文 = 最低带（最靠近输入框；输入框顶边光晕 h≤3 被 min_h 过滤，
    //    分隔线 G 值低于阈值不形成带），标签 = 密文上方最近带。
    let text_thr = (p.1 + 65).clamp(100, 210);
    let card_g = (p.1 + 20).clamp(30, 120) as u8;
    let band_top = (dy0 + dlg_h / 10).max(in_y0.saturating_sub(in_h * 8));
    let mut rois = Vec::new();
    for b in &clusters {
        let (cx0, cx1) = (b.start, b.end);
        let card_w = cx1 - cx0 + 1;
        if card_w < 40 {
            continue;
        }
        let mut row_text = vec![0u32; (in_y0 - band_top) as usize];
        for ty in band_top..in_y0 {
            let mut c = 0u32;
            for x in cx0..=cx1 {
                if sc.is_text(x, ty, text_thr) {
                    c += 1;
                }
            }
            row_text[(ty - band_top) as usize] = c;
        }
        let bands = merge_bands(&row_text, 3, 3, 4);
        let bands: Vec<Band> = bands
            .iter()
            .map(|b| Band { start: b.start + band_top, end: b.end + band_top })
            .collect();
        // 密文 = 最低带（最靠近输入框），标签 = 密文上方最近的带
        let cipher = match bands.iter().max_by_key(|b| b.start) {
            Some(v) => *v,
            None => continue,
        };
        if cipher.end - cipher.start + 1 < in_h / 4 {
            continue; // 疑似边框光晕
        }
        let label = match bands
            .iter()
            .filter(|b| b.end + 2 < cipher.start)
            .max_by_key(|b| b.end)
        {
            Some(v) => *v,
            None => continue,
        };
        if cipher.end - cipher.start < label.end - label.start {
            continue; // 密文字号应大于标签，异常则此卡判废
        }

        let pad_x = 6u32;
        let pad_y = 5u32;
        let mk = |band: Band| -> Rect {
            let x = cx0.saturating_sub(pad_x).min(w - 1);
            let y = band.start.saturating_sub(pad_y).min(h - 1);
            let rw = (cx1 - cx0 + 1 + pad_x * 2).min(w - x);
            let rh = (band.end - band.start + 1 + pad_y * 2).min(h - y);
            Rect { x, y, w: rw, h: rh }
        };
        rois.push(CardRoi { label: mk(label), cipher: mk(cipher), text_thr, card_g });
    }

    if rois.is_empty() {
        return Err("未能从卡片中裁出有效 ROI".into());
    }
    info!("[solver] 预筛定位卡片数: {}", rois.len());
    Ok(rois)
}

// ---------------- ROI 裁剪与预处理 ----------------

fn crop_upscale(img: &RgbaImage, r: Rect, scale: u32) -> DynamicImage {
    let view = img.view(r.x, r.y, r.w, r.h);
    let cropped = view.to_image();
    // 近似 GDI+ HighQualityBicubic：配合 pad_canvas 边距实测三卡标签全对
    DynamicImage::ImageRgba8(image::imageops::resize(
        &cropped,
        r.w * scale,
        r.h * scale,
        FilterType::CatmullRom,
    ))
}

// G 通道二值化：亮色文字 -> 黑字白底（可行性实测 bin0 形态）
fn binarize_g(img: &DynamicImage, threshold: u8) -> DynamicImage {
    let mut out = img.to_rgba8();
    for p in out.pixels_mut() {
        let v = if p[1] >= threshold { 0u8 } else { 255u8 };
        *p = image::Rgba([v, v, v, 255]);
    }
    DynamicImage::ImageRgba8(out)
}

// 裁到亮色文字外接框（+4px 边距）：ROI 两侧空白会在高倍放大后把图宽推过
// Windows OCR 的隐性工作上限（实测 >1000px 宽时引擎返回空串），紧裁后消除
fn trim_to_text(img: &DynamicImage, thr: u8) -> DynamicImage {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in rgba.enumerate_pixels() {
        if p[1] >= thr {
            if x < x0 { x0 = x; }
            if x > x1 { x1 = x; }
            if y < y0 { y0 = y; }
            if y > y1 { y1 = y; }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return DynamicImage::ImageRgba8(rgba);
    }
    let m = 4u32;
    let nx0 = x0.saturating_sub(m);
    let ny0 = y0.saturating_sub(m);
    let nx1 = (x1 + m + 1).min(w);
    let ny1 = (y1 + m + 1).min(h);
    DynamicImage::ImageRgba8(image::imageops::crop_imm(&rgba, nx0, ny0, nx1 - nx0, ny1 - ny0).to_image())
}

// 灰度归一化：卡片底色 -> 白场、文字亮度 -> 黑场，保留抗锯齿灰阶。
// 硬二值化会把过渡像素打成粗块（部分滤镜配色下底色/文字对比度偏低，笔画易糊），
// 平滑灰度字形更接近正常渲染文本，OCR 引擎识别率显著更好。
fn normalize_gray(img: &DynamicImage, bg_g: u8) -> DynamicImage {
    let mut out = img.to_rgba8();
    // 文字亮度估计：G 通道 98 分位（避免孤立亮点把黑场拉飞）
    let mut gs: Vec<u8> = out.pixels().map(|p| p[1]).collect();
    gs.sort_unstable();
    let text_g = (gs[(gs.len() as usize * 98 / 100).min(gs.len() - 1)] as i32).max(bg_g as i32 + 40);
    let bg = bg_g as i32;
    let span = (text_g - bg).max(1);
    for p in out.pixels_mut() {
        let v = (255 - (p[1] as i32 - bg) * 255 / span).clamp(0, 255) as u8;
        *p = image::Rgba([v, v, v, 255]);
    }
    DynamicImage::ImageRgba8(out)
}

fn to_png(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {}", e))?;
    Ok(buf)
}

// 在图像四周加 margin 白边画布：文字贴边的 ROI OCR 识别率极差（实测主因）
fn pad_canvas(img: &DynamicImage, margin: u32, bg: image::Rgba<u8>) -> DynamicImage {
    let mut out = image::RgbaImage::from_pixel(
        img.width() + margin * 2,
        img.height() + margin * 2,
        bg,
    );
    image::imageops::overlay(&mut out, img, margin as i64, margin as i64);
    DynamicImage::ImageRgba8(out)
}

// ---------------- Windows.Media.Ocr ----------------

#[cfg(target_os = "windows")]
mod ocr {
    use windows::core::{Interface, HSTRING};
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{
        DataWriter, InMemoryRandomAccessStream, IOutputStream, IRandomAccessStream,
    };

    // solve_image 的引擎类型在两种平台下统一叫 Engine
    pub type Engine = OcrEngine;

    fn try_engine(tag: &str) -> Option<OcrEngine> {
        let lang = Language::CreateLanguage(&HSTRING::from(tag)).ok()?;
        OcrEngine::TryCreateFromLanguage(&lang).ok()
    }

    // 中文引擎：优先 zh-Hans-CN，失败则扫描系统语言表中的 zh-Hans 变体。
    // 全部失败 = 系统无中文 OCR 语言包（结果窗口报错触发条件）
    pub fn zh_engine() -> Option<OcrEngine> {
        if let Some(e) = try_engine("zh-Hans-CN") {
            return Some(e);
        }
        let langs = OcrEngine::AvailableRecognizerLanguages().ok()?;
        for l in langs.into_iter() {
            let tag = match l.LanguageTag() {
                Ok(t) => String::from_utf16_lossy(t.as_wide()),
                Err(_) => continue,
            };
            if tag.starts_with("zh-Hans") {
                if let Some(e) = try_engine(&tag) {
                    return Some(e);
                }
            }
        }
        None
    }

    pub fn en_engine() -> Option<OcrEngine> {
        try_engine("en-US")
    }

    // PNG 字节 -> 内存流 -> BitmapDecoder -> SoftwareBitmap -> 识别文本
    pub fn recognize_png(engine: &OcrEngine, png: &[u8]) -> Result<String, String> {
        let err = |e: windows::core::Error| e.to_string();
        let stream = InMemoryRandomAccessStream::new().map_err(err)?;
        let rs: IRandomAccessStream = stream.cast().map_err(err)?;
        let os: IOutputStream = stream.cast().map_err(err)?;
        let writer = DataWriter::CreateDataWriter(&os).map_err(err)?;
        writer.WriteBytes(png).map_err(err)?;
        writer.StoreAsync().map_err(err)?.get().map_err(err)?;
        writer.FlushAsync().map_err(err)?.get().map_err(err)?;
        stream.Seek(0).map_err(err)?;
        let decoder = BitmapDecoder::CreateAsync(&rs)
            .map_err(err)?
            .get()
            .map_err(err)?;
        let bmp = decoder
            .GetSoftwareBitmapAsync()
            .map_err(err)?
            .get()
            .map_err(err)?;
        let result = engine
            .RecognizeAsync(&bmp)
            .map_err(err)?
            .get()
            .map_err(err)?;
        let text = result.Text().map_err(err)?;
        Ok(String::from_utf16_lossy(text.as_wide()))
    }
}

#[cfg(not(target_os = "windows"))]
mod ocr {
    // 非 Windows 平台占位：管线不可用，结果窗口报错
    pub struct Engine;
    pub fn zh_engine() -> Option<Engine> {
        None
    }
    pub fn en_engine() -> Option<Engine> {
        None
    }
    pub fn recognize_png(_engine: &Engine, _png: &[u8]) -> Result<String, String> {
        Err("仅支持 Windows".into())
    }
}

// ---------------- 解密与解析 ----------------

// 标签解析：取「前/后」之后的第一个数字，避免标题「密文 01」中的 0/1 干扰
// 返回 (方向符号, 移位数)：+1 = 前移（每字母 +N），-1 = 后移（每字母 -N）
fn parse_label(s: &str) -> Option<(i32, u32)> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let pos = t.find(['前', '后'])?;
    let ch = t[pos..].chars().next()?;
    let dir = if ch == '前' { 1 } else { -1 };
    let after = &t[pos + ch.len_utf8()..];
    let n = after.chars().find(|c| c.is_ascii_digit())?.to_digit(10)?;
    if n == 0 {
        return None;
    }
    Some((dir, n))
}

// 后移N位 -> -N；前移N位 -> +N（三卡自洽验证过的唯一语义）
fn caesar(cipher: &str, dir: i32, n: u32) -> String {
    let k = ((dir * n as i32) % 26 + 26) % 26;
    cipher
        .chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                ((c as u8 - b'A' + k as u8) % 26 + b'A') as char
            } else {
                c
            }
        })
        .collect()
}

fn strip_ws_upper(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

// ---------------- 自学习（本地 JSON 修正记录） ----------------

fn answers_path() -> Result<PathBuf, String> {
    let app = APP.get().ok_or("app not initialized")?;
    let dir = dirs::config_dir().ok_or("config dir not found")?;
    Ok(dir
        .join(&app.config().tauri.bundle.identifier)
        .join("answers.json"))
}

fn load_answers() -> HashMap<String, String> {
    match answers_path().and_then(|p| std::fs::read_to_string(p).map_err(|e| e.to_string())) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

pub fn save_correction(cipher: &str, answer: &str) -> Result<(), String> {
    let cipher = cipher.trim().to_uppercase();
    let answer = answer.trim().to_uppercase();
    if cipher.is_empty() || answer.is_empty() {
        return Err("密文或答案为空".into());
    }
    let mut map = load_answers();
    map.insert(cipher, answer);
    write_answers(&map)
}

fn write_answers(map: &HashMap<String, String>) -> Result<(), String> {
    let path = answers_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(())
}

// 设置页：读取自学习词库（密文 -> 答案）
pub fn get_corrections() -> HashMap<String, String> {
    load_answers()
}

// 设置页：删除一条自学习记录
pub fn del_correction(cipher: &str) -> Result<(), String> {
    let mut map = load_answers();
    map.remove(cipher.trim().to_uppercase().as_str());
    write_answers(&map)?;
    info!("[solver] 答案修正已删除");
    Ok(())
}

// ---------------- 主识别流程 ----------------

fn solve_image(
    img: &RgbaImage,
    zh: &ocr::Engine,
    en: Option<&ocr::Engine>,
) -> Result<Vec<Value>, String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (img, zh, en);
        return Err("仅支持 Windows".into());
    }
    #[cfg(target_os = "windows")]
    {
        let rois = detect_cards(img)?;
        let answers = load_answers();
        let mut rows = Vec::new();

        for (i, roi) in rois.iter().enumerate() {
            // 标签行：多级放大（4x/2x/6x）+ G 通道二值化（阈值随卡片底色自适应）+ 白边画布 -> 中文引擎，
            // 解题图可能被系统缩放显示，固定倍率会失配，逐级尝试直到解析出「前/后移 N 位」
            let mut parsed: Option<(i32, u32)> = None;
            for &scale in &[LABEL_UPSCALE, 2, 6] {
                // 紧裁到文字外接框后再放大，避免高倍下图宽超过 OCR 引擎工作上限
                let up = trim_to_text(&crop_upscale(img, roi.label, scale), roi.text_thr as u8);
                // 先灰度归一化（平滑字形），再硬二值化兜底
                let label_gray = normalize_gray(&up, roi.card_g);
                let label_bin = binarize_g(&up, roi.text_thr as u8);
                let mut parsed_now = None;
                for prep in [&label_gray, &label_bin] {
                    let label_png = to_png(&pad_canvas(prep, 24, image::Rgba([255, 255, 255, 255])))?;
                    let label_text = match ocr::recognize_png(zh, &label_png) {
                        Ok(t) => t,
                        Err(e) => {
                            error!("[solver] 卡{} 标签 OCR 失败 ({}x): {}", i + 1, scale, e);
                            continue;
                        }
                    };
                    if let Some(v) = parse_label(&label_text) {
                        parsed_now = Some(v);
                        break;
                    }
                }
                if parsed_now.is_some() {
                    parsed = parsed_now;
                    break;
                }
            }
            let (dir, n) = match parsed {
                Some(v) => v,
                None => {
                    error!("[solver] 卡{} 标签解析失败", i + 1);
                    rows.push(json!({"cipher": "", "label": "", "answer": "", "status": "error", "msg": "标签解析失败"}));
                    continue;
                }
            };

            // 密文行：2x 放大原样 -> 加同色边距画布 -> 英文引擎优先，中文引擎兜底
            let cipher_bg = *img.get_pixel(roi.cipher.x.min(img.width() - 1), roi.cipher.y.min(img.height() - 1));
            let cipher_up = crop_upscale(img, roi.cipher, CIPHER_UPSCALE);
            let cipher_png = to_png(&pad_canvas(&cipher_up, 16, cipher_bg))?;
            let cipher_raw = match en.and_then(|e| ocr::recognize_png(e, &cipher_png).ok()) {
                Some(t) => t,
                None => match ocr::recognize_png(zh, &cipher_png) {
                    Ok(t) => t,
                    Err(e) => {
                        error!("[solver] 卡{} 密文 OCR 失败: {}", i + 1, e);
                        rows.push(json!({"cipher": "", "label": "", "answer": "", "status": "error", "msg": "密文识别失败"}));
                        continue;
                    }
                },
            };
            let cipher = strip_ws_upper(&cipher_raw);
            if cipher.len() < WORD_MIN || cipher.len() > WORD_MAX {
                error!("[solver] 卡{} 密文长度异常: {} 字符", i + 1, cipher.len());
                rows.push(json!({"cipher": "", "label": "", "answer": "", "status": "error", "msg": "密文识别失败"}));
                continue;
            }

            // 按标签语义解密
            let plain = caesar(&cipher, dir, n);
            // 自学习优先：历史修正记录覆盖计算结果
            let (answer, status) = if let Some(a) = answers.get(&cipher) {
                (a.clone(), "learned")
            } else if WORDS.contains(&plain.as_str()) {
                (plain, "ok")
            } else {
                (plain, "unknown")
            };
            let dir_text = if dir > 0 { "前移" } else { "后移" };
            rows.push(json!({
                "cipher": cipher,
                "label": format!("{}{}位", dir_text, n),
                "answer": answer,
                "status": status,
            }));
        }
        Ok(rows)
    }
}
