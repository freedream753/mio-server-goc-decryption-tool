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

// 内置默认答案词表（游戏答案全集，提取自官方前/后移对应表）：仅做词表反查与答案校验，不做暴力破解。
// 可被设置页的自定义词表覆盖（words.json），词表是分级输出「词表反查」兜底的唯一数据源。
const DEFAULT_ANSWERS: &[&str] = &[
    "ABANDON", "ACHIEVE", "BALANCE", "BUDGET", "CAPACITY", "CONTRAST", "DECLINE", "DEMONSTRATE",
    "EFFICIENT", "ELIMINATE", "FAMILIAR", "FLEXIBLE", "GENERATE", "GUARANTEE", "HARVEST",
    "IDENTIFY", "ILLUSTRATE", "INVOLVE", "JUSTIFY", "KNOWLEDGE", "LAUNCH", "MAINTAIN", "MOTIVATE",
    "NEGATIVE", "NEGOTIATE", "OBTAIN", "OCCUPY", "PARTICIPATE", "PERCEIVE", "QUOTE", "QUALITY",
    "REDUCE", "RESTRICT", "SIGNIFICANT", "SURVIVE", "TOLERATE", "TRANSFORM", "UPDATE", "URBAN",
    "VARY", "VITAL", "WITHDRAW", "WITNESS", "YIELD", "ZONE",
];

// 词表派生映射缓存：词表变更（保存/恢复默认）后失效，下次使用时按当前词表重建
static TABLE_CACHE: std::sync::RwLock<Option<std::sync::Arc<HashMap<String, (i32, u32, String)>>>> =
    std::sync::RwLock::new(None);

// 自定义词表持久化路径（与 answers.json 同目录）
fn words_path() -> Result<PathBuf, String> {
    let app = APP.get().ok_or("app not initialized")?;
    let dir = dirs::config_dir().ok_or("config dir not found")?;
    Ok(dir
        .join(&app.config().tauri.bundle.identifier)
        .join("words.json"))
}

// 当前生效词表：words.json 覆盖优先，否则内置默认
pub fn answer_words() -> Vec<String> {
    if let Ok(p) = words_path() {
        if let Ok(s) = std::fs::read_to_string(p) {
            if let Ok(v) = serde_json::from_str::<Vec<String>>(&s) {
                if !v.is_empty() {
                    return v;
                }
            }
        }
    }
    DEFAULT_ANSWERS.iter().map(|s| s.to_string()).collect()
}

// 当前词表是否为自定义（words.json 存在即视为自定义），
// 供设置页「恢复默认」按钮判定，避免前端按词数与 DEFAULT_ANSWERS.len() 硬耦合
pub fn is_custom_words() -> bool {
    words_path().map(|p| p.exists()).unwrap_or(false)
}

// 由词表派生密文映射（±1..13 位，n 升序 or_insert 保证最短表示）：
// mod 26 下 后移N ≡ 前移(26-N)，只派生到 13 避免给玩家显示 后移24位 这类误导标签
fn build_table(words: &[String]) -> HashMap<String, (i32, u32, String)> {
    let mut m = HashMap::new();
    for w in words {
        for n in 1u32..=13 {
            for dir in [1i32, -1] {
                // 解密恒等式：caesar(密文, dir, n) = 答案 => 密文 = caesar(答案, -dir, n)
                m.entry(caesar(w, -dir, n)).or_insert((dir, n, w.clone()));
            }
        }
    }
    m
}

// 映射表快照：缓存命中直接返回，否则按当前词表重建
fn cipher_table_snapshot() -> std::sync::Arc<HashMap<String, (i32, u32, String)>> {
    if let Ok(g) = TABLE_CACHE.read() {
        if let Some(t) = g.as_ref() {
            return t.clone();
        }
    }
    let table = std::sync::Arc::new(build_table(&answer_words()));
    if let Ok(mut w) = TABLE_CACHE.write() {
        *w = Some(table.clone());
    }
    table
}

fn invalidate_table() {
    if let Ok(mut w) = TABLE_CACHE.write() {
        *w = None;
    }
}

// 设置页：保存自定义词表（校验 A-Z、词长 3~12、去重、上限 500），持久化后失效缓存
pub fn save_answer_words(words: Vec<String>) -> Result<usize, String> {
    let mut seen = std::collections::HashSet::new();
    let mut clean = Vec::new();
    for w in words {
        let w = w.trim().to_uppercase();
        if w.is_empty() {
            continue;
        }
        if w.len() < WORD_MIN || w.len() > WORD_MAX {
            return Err(format!("词长须在 {}~{} 位：{}", WORD_MIN, WORD_MAX, w));
        }
        if !w.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(format!("仅支持 A-Z：{}", w));
        }
        if seen.insert(w.clone()) {
            clean.push(w);
        }
    }
    if clean.is_empty() {
        return Err("词表不能为空".into());
    }
    if clean.len() > 500 {
        return Err("词表过大（上限 500 词）".into());
    }
    let path = words_path()?;
    let data = serde_json::to_string_pretty(&clean).map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    invalidate_table();
    info!("[solver] 答案词表已更新：{} 词，映射表待重建", clean.len());
    Ok(clean.len())
}

// 设置页：删除自定义词表，恢复内置默认
pub fn reset_answer_words() -> Result<(), String> {
    if let Ok(p) = words_path() {
        let _ = std::fs::remove_file(p);
    }
    invalidate_table();
    info!("[solver] 答案词表已恢复内置默认（{} 词）", DEFAULT_ANSWERS.len());
    Ok(())
}

// 识别单飞保护：热键/托盘/HTTP 三入口可并发触发，并发截图会对同一缓存 PNG
// 交错执行非原子写入（写坏文件 -> 解码失败报错卡），进行中直接忽略新触发
static SOLVE_IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn spawn_solve(capture: bool, show: bool, manual: bool) {
    use std::sync::atomic::Ordering;
    if SOLVE_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        info!("[solver] 已有识别任务进行中，忽略本次触发");
        return;
    }
    std::thread::spawn(move || {
        solve_flow(capture, show, manual);
        SOLVE_IN_FLIGHT.store(false, Ordering::SeqCst);
    });
}

// 入口：热键 / 托盘 / HTTP 主路由。截图 + 弹结果窗口
pub fn solver_recognize() {
    spawn_solve(true, true, false);
}

// HTTP 隐藏验证入口：不截屏、不弹窗，直接对已有 pot_screenshot_full.png 跑管线，
// 第一个答案写剪贴板，供无感回归测试读取
pub fn solver_recognize_hidden() {
    spawn_solve(false, false, false);
}

// 手动框选去重：tauri v1 的 window.listen 不会随窗口销毁清理，截图窗口反复
// 新建后旧监听器残留累积，同一次框选会触发多次 success。300ms 窗口内的
// 重复触发直接忽略（真实用户两次框选间隔远大于此）。
static LAST_MANUAL_TRIGGER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// 手动框选小图预放大阈值：完整对话框在 2.5K@150% 屏上物理宽约 1377px、
// 4K@100% 屏上约 918px（管线调优基准是前者）。框选图整体明显小于基准时
// 先 2 倍放大再交管线，使小字（4K 100%、高 DPI 缩小显示等）回到实测调优的尺寸区间
const MANUAL_SMALL_W: u32 = 1100;
const MANUAL_SMALL_H: u32 = 700;

// 手动框选前置步骤失败的用户可见反馈：ocr_recognize 命令先隐藏结果窗口再拉起
// 框选窗，此处只写日志的话用户点了按钮会毫无反应，须把错误卡片推回结果窗口
fn manual_fail_notice(msg: String) {
    error!("[solver] 手动框选失败: {}", msg);
    let payload = json!({
        "cards": [{
            "cipher": "--",
            "label": "",
            "answer": "",
            "status": "error",
            "msg": format!("手动识别失败：{}", msg),
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

// 手动框选入口：截图窗口框选完成（success 事件）后调用。
// 只复用 v1.0.1 的框选交互：用户选区图已在 pot_screenshot_cut.png，
// 拷贝为管线标准输入 pot_screenshot_full.png 后走 v1.0.3 自动管线
// （WinOCR + 相对色序定位），结果照常推送结果窗口展示
pub fn solver_recognize_manual() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let prev = LAST_MANUAL_TRIGGER.swap(now, std::sync::atomic::Ordering::SeqCst);
    if now.saturating_sub(prev) < 300 {
        return;
    }

    // in-flight 预检：识别进行中时新触发会被 spawn_solve 静默丢弃，
    // 若先执行下方拷贝/预放大，会覆盖进行中任务的数据源 pot_screenshot_full.png。
    // 文件操作前先挡掉；真正的权威判定仍在 spawn_solve 的 CAS（预检到触发的
    // 窗口极窄，残留竞态由 CAS 兜底丢弃）
    if SOLVE_IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst) {
        info!("[solver] 已有识别任务进行中，忽略本次手动框选触发");
        return;
    }

    let (from, to) = match (
        cache_file("pot_screenshot_cut.png"),
        cache_file("pot_screenshot_full.png"),
    ) {
        (Ok(f), Ok(t)) => (f, t),
        _ => {
            manual_fail_notice("缓存路径解析失败".into());
            return;
        }
    };
    if let Err(e) = std::fs::copy(&from, &to) {
        // 选区图缺失/拷贝失败时不回退旧全屏缓存，避免用陈旧图给出误导结果
        manual_fail_notice(format!("框选选区图读取失败: {}", e));
        return;
    }
    // 小图预放大：读取失败不阻塞主流程（按原图进管线，由管线自行兜底报错）。
    // 不能用 image::open——它按扩展名判格式，而选区缓存文件名固定 .png、
    // 内容可能是 JPG（测试拷图/部分截图源），须与 load_screenshot 一致按内容嗅探
    let decoded = std::fs::File::open(&to)
        .map_err(|e| e.to_string())
        .and_then(|f| {
            image::ImageReader::new(std::io::BufReader::new(f))
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .decode()
                .map_err(|e| e.to_string())
        });
    match decoded {
        Ok(img) => {
            let (w, h) = (img.width(), img.height());
            if w < MANUAL_SMALL_W && h < MANUAL_SMALL_H && w > 0 && h > 0 {
                let scaled = img.resize_exact(w * 2, h * 2, FilterType::CatmullRom);
                if let Err(e) = scaled.save_with_format(&to, image::ImageFormat::Png) {
                    warn!("[solver] 手动框选小图预放大失败，按原图识别: {}", e);
                } else {
                    info!("[solver] 手动框选小图预放大: {}x{} -> {}x{}", w, h, w * 2, h * 2);
                }
            } else {
                info!("[solver] 手动框选图尺寸 {}x{}，无需预放大", w, h);
            }
        }
        Err(e) => error!("[solver] 手动框选图读取失败，按原图识别: {}", e),
    }
    info!("[solver] 手动框选选区接入管线: {:?}", from);
    spawn_solve(false, true, true);
}

fn solve_flow(capture: bool, show: bool, manual: bool) {
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
            // 手动框选回退：完整管线失败（如只框了密文条）时，退化为
            // 「密文直读 + 词表反查」路径——密文唯一决定解法，无需标签
            if manual {
                match run_cipher_fallback() {
                    Ok(payload) => {
                        info!("[solver] 手动框选密文直读兜底成功");
                        if let Some(app) = APP.get().cloned() {
                            let _ = app.run_on_main_thread(move || {
                                result_window_notify(payload);
                            });
                        }
                        return;
                    }
                    Err(fe) => {
                        warn!("[solver] 密文直读兜底失败: {}", fe);
                    }
                }
            }
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

// 结果收尾（run_solve 与 run_cipher_fallback 共用）：取第一个有效答案写入
// 剪贴板，构造并缓存结果 payload，保证两条路径的复制/展示行为一致
fn finalize_rows(rows: Vec<Value>) -> Value {
    // 默认复制第一个有效答案（跳过空答案行：首卡为 input/manual 兜底时仍复制后续有效卡）
    let first = rows.iter().find(|r| {
        r["status"] != json!("error") && r["answer"].as_str().map_or(false, |a| !a.is_empty())
    });
    if let Some(ans) = first.and_then(|r| r["answer"].as_str()) {
        if let Some(app) = APP.get() {
            use tauri::ClipboardManager;
            if let Err(e) = app.clipboard_manager().write_text(ans) {
                warn!("[solver] 剪贴板写入失败: {}", e);
            }
        }
    }
    let payload = json!({ "cards": rows, "show": true });
    *SOLVE_RESULT.lock().unwrap() = Some(payload.clone());
    payload
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
    Ok(finalize_rows(rows))
}

// 手动框选「密文直读 + 词表反查」兜底：选区只有密文（无标签/输入框）时，
// 完整管线在卡片定位阶段即失败。密文唯一决定解法（词表反查），无需标签。
// Ok(payload) = 至少反查/标记出一条结果；Err = 选区里根本没识别出密文
fn run_cipher_fallback() -> Result<Value, String> {
    let img = load_screenshot()?;
    let rows = solve_cipher_only(&img)?;
    if rows.is_empty() {
        return Err("选区中未识别出任何密文".into());
    }
    Ok(finalize_rows(rows))
}

// 密文直读：列投影把选区分成若干文字段（游戏三卡密文同行、卡间大空隙），
// 每段独立放大 + OCR（整幅一次 OCR 会超 WinOCR 约 1000px 画布宽限制返回空串），
// 逐词走「自学习 > 词表反查 > 手动位移」分级输出。词表未收录的密文仍以
// manual 状态给出 +/- 按键，用户可手动选位移
fn solve_cipher_only(img: &RgbaImage) -> Result<Vec<Value>, String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = img;
        return Err("仅支持 Windows".into());
    }
    #[cfg(target_os = "windows")]
    {
        let (w, h) = img.dimensions();
        // 亮度自适应阈值：密文条内暗底(输入框)占多数，亮字占少数。
        // bg 取 60 分位（主色=框底色），hi 取 99 分位（文字亮峰），阈值居中偏下
        let mut gs: Vec<u32> = Vec::with_capacity(((w * h) / 4) as usize);
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                gs.push(img.get_pixel(x, y).0[1] as u32);
            }
        }
        if gs.is_empty() {
            return Err("选区为空".into());
        }
        gs.sort_unstable();
        let pct = |q: f64| gs[((gs.len() as f64 - 1.0) * q) as usize];
        let (bg, hi) = (pct(0.6), pct(0.99));
        let thr = if hi > bg + 40 { bg + (hi - bg) * 45 / 100 } else { return Err("选区中未发现文字".into()) };

        // 列投影分段：亮像素列计数，gap 桥接 12px（字母间隙），最小宽 6px
        let mut col = vec![0u32; w as usize];
        for x in 0..w {
            let mut c = 0u32;
            for y in 0..h {
                if img.get_pixel(x, y).0[1] as u32 >= thr {
                    c += 1;
                }
            }
            col[x as usize] = c;
        }
        let maxc = *col.iter().max().unwrap_or(&0);
        if maxc == 0 {
            return Err("选区中未发现文字".into());
        }
        let bands = merge_bands(&col, (maxc / 5).max(2), 12, 6);
        if bands.is_empty() {
            return Err("未能从选区中定位密文（请框选密文所在区域）".into());
        }
        info!("[solver] 密文直读定位到 {} 个文字段", bands.len());

        // 每段独立 OCR：按段高自适应放大 + 背景色边距填充
        let bgc = *img.get_pixel(0, 0);
        let en = ocr::en_engine();
        let zh = ocr::zh_engine();
        let answers = load_answers();
        let table = cipher_table_snapshot();
        let mut rows: Vec<Value> = Vec::new();
        for (i, b) in bands.iter().enumerate() {
            if rows.len() >= 6 {
                break; // 已达上限（游戏只有三卡），剩余段无需再 OCR
            }
            let x1 = (b.end + 1).min(w);
            let seg = img.view(b.start, 0, x1 - b.start, h).to_image();
            // 紧裁到文字外接框：输入框边框/底色会撑宽分段并稀释文字占比，
            // 导致 OCR 返回空串；裁后按真实文字尺寸计算放大倍率
            let seg = trim_to_text(&DynamicImage::ImageRgba8(seg), thr.min(255) as u8);
            // 按段高自适应放大；同时受段宽约束——放大后含边距不超过 900px，
            // 避免超 WinOCR 约 1000px 画布宽限制返回空串
            let by_h = if seg.height() < 20 {
                4
            } else if seg.height() < 40 {
                3
            } else if seg.height() < 80 {
                2
            } else {
                1
            };
            let by_w = (900 / (seg.width() + 32).max(1)).max(1) as u32;
            let scale = by_h.min(by_w);
            let up = DynamicImage::ImageRgba8(image::imageops::resize(
                &seg,
                seg.width() * scale,
                seg.height() * scale,
                FilterType::CatmullRom,
            ));
            let png = to_png(&pad_canvas(&up, 16, bgc))?;
            let raw = match en.as_ref().and_then(|e| ocr::recognize_png(e, &png).ok()) {
                Some(t) => t,
                None => match zh.as_ref() {
                    Some(z) => ocr::recognize_png(z, &png)?,
                    None => return Err("系统缺少 OCR 语言包".into()),
                },
            };
            // 隐私边界：只记长度，不落盘文本内容
            info!("[solver] 密文直读段{} OCR 文本长度: {}", i + 1, raw.chars().count());
            // 段内通常只有一个密文词：先整体去掉非字母（OCR 可能把字母用空格
            // 隔开，按词拆会只剩单字母）；整体长度异常时再按非字母分词兜底
            let whole = strip_ws_upper(&raw);
            let cands: Vec<String> = if (WORD_MIN..=WORD_MAX).contains(&whole.len()) {
                vec![whole]
            } else {
                raw.split(|c: char| !c.is_ascii_alphabetic())
                    .map(|tok| {
                        tok.chars()
                            .filter(|c| c.is_ascii_alphabetic())
                            .map(|c| c.to_ascii_uppercase())
                            .collect::<String>()
                    })
                    .filter(|t| (WORD_MIN..=WORD_MAX).contains(&t.len()))
                    .collect()
            };
            for t in cands {
                if rows.iter().any(|r| r["cipher"] == json!(t)) {
                    continue;
                }
                if rows.len() >= 6 {
                    break; // 游戏只有三卡，防 OCR 噪声词撑爆结果窗口
                }
                // 自学习优先（与主管线一致），其次词表反查，最后手动位移
                if let Some(a) = answers.get(&t) {
                    rows.push(json!({"cipher": t, "label": "", "answer": a, "status": "learned"}));
                    continue;
                }
                if let Some((d, n, word)) = table.get(&t) {
                    let dir_text = if *d > 0 { "前移" } else { "后移" };
                    rows.push(json!({
                        "cipher": t,
                        "label": format!("{}{}位", dir_text, n),
                        "answer": word,
                        "status": "table",
                    }));
                    continue;
                }
                // 隐私边界：只记词长，不落盘密文内容
                warn!("[solver] 密文直读段{} 词表未命中（词长 {}）", i + 1, t.len());
                rows.push(json!({"cipher": t, "label": "", "answer": "", "status": "manual", "msg": "手动选位移"}));
            }
        }
        if rows.is_empty() {
            return Err("未能从选区中识别出密文（请框选密文所在区域）".into());
        }
        Ok(rows)
    }
}

// 自动路径降级兜底：卡片结构定位失败但对话框区域已定位时，对对话框裁区跑
// 「密文直读 + 词表反查」。只保留词表命中/自学习的高置信结果——manual 行多
// 来自标题/HUD 等 OCR 噪声且无标签约束，直接丢弃；全无置信结果时回退原
// 定位错误，保持报障信息可读
fn degraded_cipher_rows(img: &RgbaImage, dialog: Rect, orig_err: String) -> Result<Vec<Value>, String> {
    let x1 = (dialog.x + dialog.w).min(img.width());
    let y1 = (dialog.y + dialog.h).min(img.height());
    if x1 <= dialog.x || y1 <= dialog.y {
        return Err(orig_err);
    }
    let crop = img.view(dialog.x, dialog.y, x1 - dialog.x, y1 - dialog.y).to_image();
    // 裁区常被亮色场景包围（夜视/白昼下对话框蓝暗区域之外是大片亮色），
    // solve_cipher_only 的「暗底亮字」阈值模型会失效（bg 取 60 分位被白场
    // 抬高，密文反低于阈值）。对话框面板、密文/标签文字均满足蓝暗色序，
    // 先把非蓝暗色像素压黑再送直读，场景噪声同时被清除
    let mut masked = crop.clone();
    for px in masked.pixels_mut() {
        let (r, g, b) = (px.0[0] as u32, px.0[1] as u32, px.0[2] as u32);
        if !(b >= r + 20 && b >= g + 12) {
            *px = image::Rgba([0, 0, 0, 255]);
        }
    }
    // 先按行分出文字带（标题/标签/密文行），再逐带直读。整裁区直接列投影
    // 会把全息屏/竖向亮边框并成贯穿整屏的巨段；行带按高度过滤后只剩文本行
    // （密文 ~25px、标签 ~10px），全息屏亮带因行高超过上限被滤除
    let (cw, ch) = masked.dimensions();
    let mut gs: Vec<u32> = Vec::with_capacity(((cw * ch) / 4) as usize);
    for y in (0..ch).step_by(2) {
        for x in (0..cw).step_by(2) {
            gs.push(masked.get_pixel(x, y).0[1] as u32);
        }
    }
    gs.sort_unstable();
    let pct = |q: f64| gs[((gs.len() as f64 - 1.0) * q) as usize];
    let (bg, hi) = (pct(0.6), pct(0.99));
    if hi <= bg + 40 {
        return Err(orig_err);
    }
    let thr = (bg + (hi - bg) * 45 / 100).min(255) as u32;
    let mut row = vec![0u32; ch as usize];
    for y in 0..ch {
        let mut c = 0u32;
        for x in 0..cw {
            if masked.get_pixel(x, y).0[1] as u32 >= thr {
                c += 1;
            }
        }
        row[y as usize] = c;
    }
    if *row.iter().max().unwrap_or(&0) == 0 {
        return Err(orig_err);
    }
    // 高通：竖向亮结构（卡片边框/对话框亮边）给每行贡献近似恒定的底噪，
    // 会让文本行并成贯穿巨带（全息屏/白场尤其严重）；对行计数做 ±40 行
    // 窗口最小值高通，只留局部突起（文本行）。阈值按裁宽定：一行至少能
    // 容 3 个短词（w/12），避免被整屏大行抬高把密文行挤出
    let mut hp = vec![0u32; ch as usize];
    for y in 0..ch {
        let lo = y.saturating_sub(40);
        let hi2 = (y + 41).min(ch);
        let m = row[lo as usize..hi2 as usize].iter().copied().min().unwrap_or(0);
        hp[y as usize] = row[y as usize].saturating_sub(m);
    }
    let row_bands = merge_bands(&hp, (cw / 12).max(8), 12, 6);
    let mut rows: Vec<Value> = Vec::new();
    for b in row_bands {
        if rows.len() >= 6 {
            break; // 已达上限，剩余行带无需再 OCR
        }
        let bh = b.end - b.start + 1;
        // 文本行高度窗口：过滤全息屏巨带与噪点；密文/标签行均落在窗口内
        if !(8..=80).contains(&bh) {
            continue;
        }
        let y2 = (b.end + 1).min(ch);
        let line = masked.view(0, b.start, cw, y2 - b.start).to_image();
        if let Ok(mut found) = solve_cipher_only(&line) {
            for r in found.drain(..) {
                if rows.iter().any(|x| x["cipher"] == r["cipher"]) {
                    continue;
                }
                if rows.len() >= 6 {
                    break;
                }
                rows.push(r);
            }
        }
    }
    let confident: Vec<Value> = rows
        .into_iter()
        .filter(|r| r["status"] != json!("manual"))
        .collect();
    if confident.is_empty() {
        Err(orig_err)
    } else {
        info!(
            "[solver] 结构定位失败({})，降级密文直读得到 {} 个高置信结果",
            orig_err,
            confident.len()
        );
        Ok(confident)
    }
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
    cipher_h: u32, // 密文带高（不含边距），小UI档 OCR 倍率反推依据
    label_h: u32,  // 标签带高（不含边距）
    text_thr: u32, // 该卡的文字判定阈值（随卡片底色自适应）
    card_g: u8,    // 卡片底色 G 通道（灰度归一化的白场基准）
}

// 界面尺寸度量：小UI档判定 + OCR 倍率反推依据。
// in_h（输入框带高）是跨配色/跨缩放稳定的尺寸基准（全图 53px、特写 40px）
struct UiMetrics {
    small_ui: bool, // 输入框带高显著低于调优基准（< 28px ≈ 基准的 70%）
    in_h: u32,
}

// 定位失败：msg 为用户可见原因；dialog 为已定位到的对话框区域（行/列带已
// 成功时携带），供自动路径降级「密文直读」兜底使用。
// 对话框是世界空间 UI，相机角度/动画帧会持续改变其屏幕几何与配色，任何
// 静态守卫都无法覆盖全部视角；结构定位失败时密文行通常仍在对话框内
struct LocateErr {
    msg: String,
    dialog: Option<Rect>,
}

fn lerr(msg: &str, dialog: Option<Rect>) -> LocateErr {
    LocateErr { msg: msg.to_string(), dialog }
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
        // 通道差用 u16 比较：u8 加法在 r>=236/g>=243 时回绕，会把近白像素
        // 误判为蓝暗色（release 静默回绕、debug 测试档直接 panic）
        let (r, g, b) = (r as u16, g as u16, b as u16);
        b >= r + 20 && b >= g + 12
    }

    // 输入框内部：显著比页面背景 P 更暗（B、G 双通道 ≤ 0.85×P）且保持蓝主导。
    // 滤镜漂移两端实测 (0,0,40)~(20,28,48) vs P(2,38,94)~(16,42,66) 均满足。
    #[inline]
    fn is_input_rel(&self, x: u32, y: u32, p: Rgb) -> bool {
        let (r, g, b) = self.rgb(x, y);
        // 同 is_blue_dark：u16 比较防 u8 加法回绕
        let b = b as u16;
        b >= r as u16 + 12 && (b as u32) * 20 <= p.2 * 17 && (g as u32) * 20 <= p.1 * 17
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

// small_ui = 小UI参数档：服务高分辨率截图上对话框/字号显著偏小的场景
// （4K 100%、游戏内界面缩放调低等），放宽「从场景里挑目标」的像素守卫。
// 常规档沿用 v1.0.3 实测调优值；调用方先跑常规档，失败且截图宽 ≥ 2560
// 时换本档重试一次（见 solve_image），正常尺寸路径零行为变化
fn detect_cards(img: &RgbaImage, small_ui: bool) -> Result<(Vec<CardRoi>, UiMetrics), LocateErr> {
    let sc = Scanner::new(img);
    let (w, h) = (sc.w, sc.h);
    if w < 200 || h < 200 {
        return Err(lerr(
            "框选区域过小（需至少 200x200 像素），请框选完整的解题对话框（含标签、密文与输入框）",
            None,
        ));
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
    // 行阈值 = 对话框须达到的像素宽度下限：常规档 w/6 假定对话框占屏宽 1/6 以上；
    // 小UI档放宽到 w/16（4K 上 918px 对话框仅占屏宽 24%，更低 UI 缩放还会更小）
    let (row_thr, row_gap, row_min_h) = if small_ui {
        ((w / 16).max(120), 20, 120)
    } else {
        (w / 6, 40, 200)
    };
    let blue_bands = merge_bands(&row_blue, row_thr, row_gap, row_min_h);
    let dialog = blue_bands
        .iter()
        .max_by_key(|b| row_blue[b.start as usize..=b.end as usize].iter().sum::<u32>())
        .copied()
        .ok_or_else(|| lerr("未定位到对话框区域", None))?;
    let (mut dy0, mut dy1) = (dialog.start, dialog.end);
    // 延伸合并：对话框内两类暗行会在纵向带中部撕出数十行低谷把带截断——
    //   a) 输入框整行是全宽暗色（非蓝暗色），4K@100% 实机该行 row_blue≈102、
    //      前后段≈800，带终止于输入框上沿，输入框检测永远扫不到真目标；
    //   b) 半透明对话框叠在很暗的场景上时，部分行 b-r 通道差不足 20 整行落空。
    //   选定最强带后按「低水位 row_thr/4 + 桥接 ≤70 行」向两端延伸接回被隔开的
    //   分段；floor 之下连续超过 70 行才终止，正常场景带外全是近零行，零影响
    let floor = (row_thr / 4).max(1);
    let mut gap = 0u32;
    let mut y = dy1 + 1;
    while y < h && gap <= 70 {
        if row_blue[y as usize] >= floor {
            dy1 = y;
            gap = 0;
        } else {
            gap += 1;
        }
        y += 1;
    }
    gap = 0;
    let mut y = dy0 as i64 - 1;
    while y >= 0 && gap <= 70 {
        if row_blue[y as usize] >= floor {
            dy0 = y as u32;
            gap = 0;
        } else {
            gap += 1;
        }
        y -= 1;
    }

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
    // 列阈值取「最大列值的 1/4」自适应，而非固定 dlg_h/2：纵向带可能被上方全息屏
    // 撑高（动画闪烁使带高逐帧波动），固定阈值会在带高偏高的帧整体落空
    // （实机日志 33 次该阶段失败），对话框列恒为最大值附近，比例阈值随帧自适应
    let max_col = *col_blue.iter().max().unwrap_or(&0);
    let col_bands = if small_ui {
        merge_bands(&col_blue, (max_col / 4).max(8), 12, (dlg_h / 6).max(30))
    } else {
        merge_bands(&col_blue, (max_col / 4).max(16), 30, (dlg_h / 6).max(60))
    };
    let dx = col_bands
        .iter()
        .max_by_key(|b| col_blue[b.start as usize..=b.end as usize].iter().sum::<u32>())
        .copied()
        .ok_or_else(|| lerr("未定位到对话框横向范围", None))?;
    let (dx0, dx1) = (dx.start, dx.end);
    let dlg_w = dx1 - dx0 + 1;
    let dlg_rect = Rect { x: dx0, y: dy0, w: dlg_w, h: dy1 - dy0 + 1 };

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
    let (&pkey, _) = hist
        .iter()
        .max_by_key(|(_, &c)| c)
        .ok_or_else(|| lerr("对话框颜色统计失败", Some(dlg_rect)))?;
    let p: Rgb = (
        ((pkey >> 16) & 0xff) * 4 + 2,
        ((pkey >> 8) & 0xff) * 4 + 2,
        (pkey & 0xff) * 4 + 2,
    );

    // 4) 输入框带：显著比页面背景更暗（B、G ≤ 0.85×P）。行值 = 「最长连续暗段」——
    //    输入框是约 1/4 框宽的连续暗块，而对话框边框/卡间阴影只是细条，
    //    按总像素计数会被边框+阴影凑满阈值（JPG 特写实测踩坑）。
    //    实机动画场景下该阶段曾大量失败，两类诱因：
    //    a) run 阈值 dlg_w/6 卡在刀刃上：框内细亮纹让暗段只有理论值的 ~40%；
    //    b) 光标悬停 / 滤镜噪声在暗段中间撕出亮缝。
    //    对策 = 分层回退（严格 → 放宽 run 阈值 → 桥接 ≤6px 亮隔断），
    //    簇校验统一用「2~3 个等宽簇」结构先验：三个输入框设计上像素级等宽，
    //    阴影/边框带的簇宽参差或单簇过宽，会被等宽校验拒绝；卡间窄条噪声
    //    按 <60% 最大簇剔除。
    let layers: [(bool, u32); 4] = [(false, 9), (false, 14), (true, 9), (true, 14)];
    let mut input_xy: Option<(u32, u32)> = None;
    let mut clusters: Vec<Band> = Vec::new();
    'layers: for (bridged, div) in layers {
        let mut row_dark = vec![0u32; h as usize];
        for y in dy0..=dy1 {
            let mut run = 0u32;
            let mut gap = 0u32;
            let mut best = 0u32;
            for x in dx0..=dx1 {
                if sc.is_input_rel(x, y, p) {
                    run += gap + 1;
                    gap = 0;
                    if run > best {
                        best = run;
                    }
                } else if bridged {
                    gap += 1;
                    if gap > 6 {
                        run = 0;
                        gap = 0;
                    }
                } else {
                    run = 0;
                }
            }
            row_dark[y as usize] = best;
        }
        let dark_bands = merge_bands(&row_dark, dlg_w / div, 6, 8);
        // 输入框带高度 ≈ 框宽的 4.4%（全图 53/1204、特写 40/902，跨缩放一致）。
        // 标题栏区、底部面板、场景暗块也会形成长暗段（特写图底部全宽暗区 run=899
        // 远大于输入框 run=232），故按「带高 ≤ 框宽/15」过滤后，按暗段质量降序
        // 逐带尝试聚簇，取第一个结构合法（2~3 个等宽簇）的带
        let mut cands: Vec<&Band> = dark_bands.iter().collect();
        cands.sort_by_key(|b| {
            std::cmp::Reverse(row_dark[b.start as usize..=b.end as usize].iter().sum::<u32>())
        });
        for band in cands {
            let (y0, y1) = (band.start, band.end);
            let hh = y1 - y0 + 1;
            let hh_min = if small_ui { (dlg_w / 150).max(5) } else { 10 };
            if hh < hh_min || hh > dlg_w / 15 {
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
            let cl = if small_ui {
                merge_bands(&col_dark, col_thr, 6, (hh as f64 * 0.3) as u32)
            } else {
                merge_bands(&col_dark, col_thr, 12, (hh as f64 * 0.3) as u32)
            };
            let max_w = cl
                .iter()
                .map(|b| b.end - b.start + 1)
                .max()
                .unwrap_or(0);
            let keep: Vec<Band> = cl
                .iter()
                .filter(|b| {
                    let min_w = if small_ui { dlg_w / 30 } else { 40 };
                    b.end - b.start + 1 >= std::cmp::max(min_w, max_w * 3 / 5)
                })
                .copied()
                .collect();
            if keep.len() < 2 || keep.len() > 3 {
                continue;
            }
            let kmin = keep
                .iter()
                .map(|b| b.end - b.start + 1)
                .min()
                .unwrap_or(0);
            let kmax = keep
                .iter()
                .map(|b| b.end - b.start + 1)
                .max()
                .unwrap_or(0);
            if kmin * 4 < kmax * 3 {
                continue; // 等宽校验：宽度比 < 0.75 说明混入阴影/噪声簇
            }
            clusters = keep;
            input_xy = Some((y0, y1));
            break 'layers;
        }
    }
    let (in_y0, in_y1) = input_xy.ok_or_else(|| lerr("未定位到输入框区域", Some(dlg_rect)))?;
    let in_h = in_y1 - in_y0 + 1;
    let in_h_min = if small_ui { 5 } else { 10 };
    if in_h < in_h_min {
        return Err(lerr("输入框区域高度异常", Some(dlg_rect)));
    }

    // 6) 逐簇在输入框上方聚亮色文字带（标题 / 标签 / 密文）。
    //    扫描上界 = max(对话框顶 + 1/10框高, 输入框上沿 - 8×框高)：
    //    排除窗口标题栏文字/控制按钮（实测会以更高带抢走密文选择），
    //    同时保证覆盖卡内文字区（≈5~6.5×框高）。不做卡片顶边检测——
    //    部分配色下卡片底色与页面底色几乎同值（JPG 特写 B 通道仅差 ~4），顶边判定不可靠。
    //    密文选择用跨簇共识：真实密文行在三张卡片处于同一像素行，光标悬在
    //    密文下方形成的更低的假带（实拍踩坑：亮条抢走密文带 → 卡片数 2）
    //    不会跨簇对齐，可安全剔除；无共识（仅 1 簇）时退回最低带。
    //    标签 = 密文上方最近带；主阈值失败时以 3/4 阈值兜底重试一次。
    let text_thr = (p.1 + 65).clamp(100, 210);
    let card_g = (p.1 + 20).clamp(30, 120) as u8;
    let band_top = (dy0 + dlg_h / 10).max(in_y0.saturating_sub(in_h * 8));
    let card_min_w = if small_ui { (dlg_w / 30).max(20) } else { 40 };
    let cards: Vec<&Band> = clusters
        .iter()
        .filter(|b| b.end - b.start + 1 >= card_min_w)
        .collect();

    let extract = |cx0: u32, cx1: u32, thr: u32| -> Vec<Band> {
        let mut row_text = vec![0u32; (in_y0 - band_top) as usize];
        for ty in band_top..in_y0 {
            let mut c = 0u32;
            for x in cx0..=cx1 {
                if sc.is_text(x, ty, thr) {
                    c += 1;
                }
            }
            row_text[(ty - band_top) as usize] = c;
        }
        merge_bands(&row_text, 3, 3, if small_ui { 3 } else { 4 })
            .into_iter()
            .map(|b| Band { start: b.start + band_top, end: b.end + band_top })
            .collect()
    };
    // 主阈值下先收集每簇文字带（共识比对基准）
    let primary: Vec<Vec<Band>> = cards
        .iter()
        .map(|b| extract(b.start, b.end, text_thr))
        .collect();

    let mut rois = Vec::new();
    // 跨簇共识行容差：常规档 8px；小UI档随 in_h 比例收缩，
    // 避免小界面上相对容差过大放进假带
    let consensus_tol = if small_ui { (in_h / 6).max(3) } else { 8 };
    for (i, b) in cards.iter().enumerate() {
        let (cx0, cx1) = (b.start, b.end);
        let fallback_thr = (text_thr * 3 / 4).max(60);
        let mut found: Option<(Band, Band)> = None;
        for thr in [text_thr, fallback_thr] {
            let fallback_bands;
            let bands = if thr == text_thr {
                &primary[i]
            } else {
                fallback_bands = extract(cx0, cx1, thr);
                &fallback_bands
            };
            if bands.is_empty() {
                continue;
            }
            // 密文 = 与其它簇行范围一致（±8px）的最低带；无共识则退回最低带
            let cipher = *bands
                .iter()
                .rev()
                .find(|bd| {
                    primary.iter().enumerate()
                        .filter(|(j, _)| *j != i)
                        .any(|(_, obs)| {
                            obs.iter().any(|ob| {
                                bd.start.abs_diff(ob.start) <= consensus_tol
                                    && bd.end.abs_diff(ob.end) <= consensus_tol
                            })
                        })
                })
                .unwrap_or_else(|| bands.iter().max_by_key(|bd| bd.start).unwrap());
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
            found = Some((cipher, label));
            break;
        }
        let Some((cipher, label)) = found else {
            continue;
        };

        let pad_x = 6u32;
        let pad_y = 5u32;
        let mk = |band: Band| -> Rect {
            let x = cx0.saturating_sub(pad_x).min(w - 1);
            let y = band.start.saturating_sub(pad_y).min(h - 1);
            let rw = (cx1 - cx0 + 1 + pad_x * 2).min(w - x);
            let rh = (band.end - band.start + 1 + pad_y * 2).min(h - y);
            Rect { x, y, w: rw, h: rh }
        };
        rois.push(CardRoi {
            label: mk(label),
            cipher: mk(cipher),
            cipher_h: cipher.end - cipher.start + 1,
            label_h: label.end - label.start + 1,
            text_thr,
            card_g,
        });
    }

    if rois.is_empty() {
        return Err(lerr("未能从卡片中裁出有效 ROI", Some(dlg_rect)));
    }
    // 界面度量日志：用户报障时据此判定是否尺寸问题（截图分辨率 + 对话框尺寸 + 输入框带高）
    let metrics = UiMetrics { small_ui: in_h < 28, in_h };
    info!(
        "[solver] 预筛定位卡片数: {}，截图 {}x{}，对话框 {}x{}，输入框带高 {}px，档位={}",
        rois.len(), w, h, dlg_w, dlg_h, in_h,
        if metrics.small_ui { "小UI" } else { "常规" }
    );
    Ok((rois, metrics))
}

// ---------------- ROI 裁剪与预处理 ----------------

// 小UI档放大倍率反推：目标把实测字高拉到 ~32px（WinOCR 最佳识别区间），
// 返回 [目标, 目标×1.5, 8x 封顶] 逐档兜底；lo 为该行倍率下限（标签小字不低于 4）
fn adaptive_scales(band_h: u32, lo: u32) -> Vec<u32> {
    let t = (32 / band_h.max(1)).clamp(lo, 8);
    let mut v = vec![t];
    let t2 = (t * 3 / 2).min(8);
    if t2 > t {
        v.push(t2);
    }
    if *v.last().unwrap() < 8 {
        v.push(8);
    }
    v
}

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
        // 定位：先常规档；失败且截图宽 ≥ 2560（2K/4K）自动换小UI档重试一次。
        // 高分辨率截图上对话框像素尺寸可能远小于调优基准（4K 100%、游戏内
        // UI 缩放调低），常规守卫会把真目标当噪声过滤掉，需放宽参数重试。
        // 两档都失败但对话框区域已定位时，降级「密文直读 + 词表反查」：
        // 对话框是世界空间 UI，相机角度/动画帧会让输入框结构守卫失守
        // （白色夜视实机报「未定位到输入框区域」），但密文行仍在对话框内，
        // 且密文唯一决定解法——只保留词表命中/自学习的高置信结果
        let (rois, ui) = 'locate: {
            match detect_cards(img, false) {
                Ok(v) => break 'locate v,
                Err(e) => {
                    let err = if img.width() >= 2560 {
                        info!(
                            "[solver] 高分辨率截图 {}x{} 常规定位失败({})，切换小UI档重试",
                            img.width(),
                            img.height(),
                            e.msg
                        );
                        match detect_cards(img, true) {
                            Ok(v) => break 'locate v,
                            Err(e2) => e2,
                        }
                    } else {
                        e
                    };
                    // 定位完全失败：已定位到对话框区域则降级「密文直读」兜底
                    match err.dialog {
                        Some(d) => return degraded_cipher_rows(img, d, err.msg),
                        None => return Err(err.msg),
                    }
                }
            }
        };
        let answers = load_answers();
        let mut rows = Vec::new();

        for (i, roi) in rois.iter().enumerate() {
            // ===== 密文优先：放大/预处理预算优先服务密文清晰度 =====
            // 2x 为主 3x 兜底，英文引擎优先中文兜底；任一组合得到合法词长即采纳
            let cipher_bg = *img.get_pixel(roi.cipher.x.min(img.width() - 1), roi.cipher.y.min(img.height() - 1));
            let mut cipher: Option<String> = None;
            // 常规档固定 2x/3x；小UI档按密文实测字高反推（目标字高 ~32px）
            let cipher_scales: Vec<u32> = if ui.small_ui {
                adaptive_scales(roi.cipher_h, 2)
            } else {
                vec![CIPHER_UPSCALE, 3]
            };
            for &scale in &cipher_scales {
                let mut up = crop_upscale(img, roi.cipher, scale);
                // 高倍放大（≥4x）前先紧裁到文字外接框，防止长密文画布超
                // WinOCR ~1000px 宽限制返回空串（常规档 2x/3x 保持原路径不动）
                if scale >= 4 {
                    up = trim_to_text(&up, roi.text_thr as u8);
                }
                let png = to_png(&pad_canvas(&up, 16, cipher_bg))?;
                let raw = match en.and_then(|e| ocr::recognize_png(e, &png).ok()) {
                    Some(t) => t,
                    None => match ocr::recognize_png(zh, &png) {
                        Ok(t) => t,
                        Err(e) => {
                            error!("[solver] 卡{} 密文 OCR 失败 ({}x): {}", i + 1, scale, e);
                            continue;
                        }
                    },
                };
                let c = strip_ws_upper(&raw);
                if c.len() >= WORD_MIN && c.len() <= WORD_MAX {
                    cipher = Some(c);
                    break;
                }
                error!("[solver] 卡{} 密文长度异常 ({}x): {} 字符", i + 1, scale, c.len());
            }
            // 优先级3：密文也识别不到 -> 留空位由用户在结果窗口手动输入密文
            let Some(cipher) = cipher else {
                rows.push(json!({"cipher": "", "label": "", "answer": "", "status": "input", "msg": "密文未识别"}));
                continue;
            };

            // 自学习优先：历史修正记录覆盖一切
            if let Some(a) = answers.get(&cipher) {
                rows.push(json!({"cipher": cipher, "label": "", "answer": a, "status": "learned"}));
                continue;
            }

            // ===== 标签行：预算让位密文 + 灰度归一化/二值化 =====
            // 常规档固定 4x/2x；小UI档按标签实测字高反推（下限 4x）
            let mut parsed: Option<(i32, u32)> = None;
            let label_scales: Vec<u32> = if ui.small_ui {
                adaptive_scales(roi.label_h, 4)
            } else {
                vec![LABEL_UPSCALE, 2]
            };
            for &scale in &label_scales {
                let up = trim_to_text(&crop_upscale(img, roi.label, scale), roi.text_thr as u8);
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

            // ===== 分级输出：词表反查（密文唯一决定解法）> 标签计算 > 手动位移 =====
            let table = cipher_table_snapshot();
            let (dir, n, answer, status) = if let Some((td, tn, ta)) = table.get(&cipher) {
                // 优先级1/2 共用：词表命中即定案（优先级1 标签也有时词表同样是真值）
                if parsed.is_none() {
                    info!("[solver] 卡{} 标签未识别，词表命中 {} -> {}", i + 1, cipher, ta);
                }
                (*td, *tn, ta.clone(), "table")
            } else if let Some((d, n)) = parsed {
                // 词表未收（密文 OCR 可能有误差）：按标签语义计算，无法校验
                (d, n, caesar(&cipher, d, n), "unknown")
            } else {
                // 优先级2末端：标签与词表都失败 -> 手动 +/- 按键
                error!("[solver] 卡{} 标签解析失败且词表未命中，密文 {} 待手动位移", i + 1, cipher);
                rows.push(json!({"cipher": cipher, "label": "", "answer": "", "status": "manual", "msg": "手动选位移"}));
                continue;
            };
            let dir_text = if dir > 0 { "前移" } else { "后移" };
            rows.push(json!({
                "cipher": cipher,
                "label": format!("{}{}位", dir_text, n),
                "answer": answer,
                "status": status,
            }));
        }
        // 小界面兜底：输入框带高 < 20px 时字号已逼近 WinOCR 物理可识别下限，
        // 三卡全部失败时不再给逐卡 input/manual 空位（用户无从填起），
        // 改报明确原因引导调大游戏内界面缩放
        if ui.small_ui
            && ui.in_h < 20
            && !rows.iter().any(|r| r["answer"].as_str().map_or(false, |a| !a.is_empty()))
        {
            return Err(format!(
                "做题界面过小（输入框仅 {}px 高，超出识别下限），请在游戏设置中调大界面缩放后重试",
                ui.in_h
            ));
        }
        Ok(rows)
    }
}

// 临时回归测试（真实管线，非仿真）：对 新用例/ 下的实机截图直接跑
// detect_cards 定位、逐卡密文直读（含真实 WinOCR + 词表反查）与
// degraded_cipher_rows 降级兜底。运行方式见 regression_locate 注释。
#[cfg(test)]
mod regression_tests {
    use super::*;

    fn load_case(name: &str) -> RgbaImage {
        // 用例目录随仓库位置自适应：CARGO_MANIFEST_DIR = src-tauri，上一级为项目根
        let p = format!("{}/../新用例/{}", env!("CARGO_MANIFEST_DIR"), name);
        let f = std::fs::File::open(&p).unwrap_or_else(|e| panic!("打开 {} 失败: {}", p, e));
        image::ImageReader::new(std::io::BufReader::new(f))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8()
    }

    /// 定位回归：cargo +stable-x86_64-pc-windows-gnu test regression_locate -- --nocapture --test-threads=1
    #[test]
    fn regression_locate() {
        let cases = [
            "4K分辨率下的实机画面.png",
            "白色夜视下的做题画面.png",
            "用例1.png",
            "用例2.png",
            "测试画面（全）.png",
            "新解题画面.jpg",
        ];
        for c in cases {
            let img = load_case(c);
            let (w, h) = img.dimensions();
            match detect_cards(&img, w >= 2560) {
                Ok((rois, ui)) => {
                    println!("PASS {} {}x{} rois={} in_h={} small_ui={}", c, w, h, rois.len(), ui.in_h, ui.small_ui);
                    assert_eq!(rois.len(), 3, "{}: 期望定位 3 卡", c);
                }
                Err(e) => panic!("FAIL {} {}x{} 定位失败: {}（dialog={:?}）", c, w, h, e.msg, e.dialog.map(|d| (d.x, d.y, d.w, d.h))),
            }
        }
    }

    /// 逐卡密文直读（真实 OCR + 词表反查）：
    /// cargo +stable-x86_64-pc-windows-gnu test regression_cipher -- --nocapture --test-threads=1
    #[test]
    fn regression_cipher() {
        let cases = [
            "4K分辨率下的实机画面.png",
            "白色夜视下的做题画面.png",
            "用例1.png",
            "用例2.png",
            "测试画面（全）.png",
            "新解题画面.jpg",
        ];
        for c in cases {
            let img = load_case(c);
            let (w, _) = img.dimensions();
            let (rois, _) = detect_cards(&img, w >= 2560).unwrap_or_else(|e| panic!("{} 定位失败: {}", c, e.msg));
            for (i, roi) in rois.iter().enumerate() {
                let line = img.view(roi.cipher.x, roi.cipher.y, roi.cipher.w, roi.cipher.h).to_image();
                match solve_cipher_only(&line) {
                    Ok(rows) => println!("CASE {} 卡{} 直读: {:?}", c, i + 1, rows),
                    Err(e) => println!("CASE {} 卡{} 直读失败: {}", c, i + 1, e),
                }
            }
        }
    }

    /// 降级兜底全链路（掩膜 -> 行带 -> OCR -> 词表反查）：对话框区域用
    /// 已定位 3 卡 ROI 的包围盒外扩 80px 近似（真实 LocateErr.dialog 由
    /// 行列带得出，此处仅为触发路径）。
    /// cargo +stable-x86_64-pc-windows-gnu test regression_degraded -- --nocapture --test-threads=1
    #[test]
    fn regression_degraded() {
        for c in ["白色夜视下的做题画面.png", "4K分辨率下的实机画面.png"] {
            let img = load_case(c);
            let (w, h) = img.dimensions();
            let (rois, _) = detect_cards(&img, w >= 2560).unwrap_or_else(|e| panic!("{} 定位失败: {}", c, e.msg));
            let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
            for roi in &rois {
                for r in [&roi.label, &roi.cipher] {
                    x0 = x0.min(r.x);
                    y0 = y0.min(r.y);
                    x1 = x1.max(r.x + r.w);
                    y1 = y1.max(r.y + r.h);
                }
            }
            let m = 80u32;
            let dialog = Rect {
                x: x0.saturating_sub(m),
                y: y0.saturating_sub(m),
                w: (x1 - x0 + 2 * m).min(w - x0.saturating_sub(m)),
                h: (y1 - y0 + 2 * m).min(h - y0.saturating_sub(m)),
            };
            match degraded_cipher_rows(&img, dialog, "测试触发".into()) {
                Ok(rows) => {
                    println!("DEGRADED PASS {} -> {:?} 条: {:?}", c, rows.len(), rows);
                    assert!(!rows.is_empty(), "{}: 降级兜底应产出高置信结果", c);
                    assert!(rows.iter().all(|r| r["status"] == json!("table") || r["status"] == json!("learned")), "{}: 仅允许 table/learned", c);
                }
                Err(e) => panic!("DEGRADED FAIL {}: {}", c, e),
            }
        }
    }
}
