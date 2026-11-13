// 截图 / OCR 引擎封装等复用工具。
// v1.1.0 起整条凯撒解密链路（卡片像素定位 + 标签解析 + 词表反查 + 自学习修正）
// 已移除，本模块只保留符文识别管线仍需要的基础能力。

use crate::APP;
use image::DynamicImage;
use log::{info, warn};
use screenshots::Screen;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Mutex;

// 缓存截图文件的磁盘 IO 互斥：符文（SCAN_IN_FLIGHT）与波形（WAVE_IN_FLIGHT）两
// 管线的单飞保护互相独立，但共用 pot_screenshot_full.png，Alt+V 识别进行中按
// Alt+X 会交叉写读同一 PNG（写坏/读到对方的帧）。所有缓存截图 IO 在此串行化。
pub static SHOT_IO: Mutex<()> = Mutex::new(());

// 缓存文件绝对路径：<cache>/<bundle identifier>/<name>
pub fn cache_file(name: &str) -> Result<PathBuf, String> {
    let app = APP.get().ok_or("app not initialized")?;
    let dir = dirs::cache_dir().ok_or("cache dir not found")?;
    Ok(dir.join(&app.config().tauri.bundle.identifier).join(name))
}

// 裁切模式（v1.2.7）：把全局裁切区域（全局物理像素）与当前显示器截图求交集裁剪。
// 未开启裁切或无交集（裁切区在另一块屏）时原样返回，保证管线行为可预期。
fn apply_crop_region(img: image::RgbaImage, origin_x: i32, origin_y: i32) -> image::RgbaImage {
    let (rx, ry, rw, rh) = match crate::window::crop_region() {
        Some(v) => v,
        None => return img,
    };
    let (dw, dh) = (img.width() as i32, img.height() as i32);
    let x0 = (rx).max(origin_x);
    let y0 = (ry).max(origin_y);
    let x1 = (rx + rw as i32).min(origin_x + dw);
    let y1 = (ry + rh as i32).min(origin_y + dh);
    if x1 <= x0 || y1 <= y0 {
        info!(
            "[solver] 裁切区域 ({},{},{},{}) 与当前显示器无交集，本帧不裁剪",
            rx, ry, rw, rh
        );
        return img;
    }
    image::imageops::crop_imm(
        &img,
        (x0 - origin_x) as u32,
        (y0 - origin_y) as u32,
        (x1 - x0) as u32,
        (y1 - y0) as u32,
    )
    .to_image()
}

// 鼠标坐标 (mx, my) 命中的显示器：返回显示器对象与原点+尺寸（物理像素），
// 两处截图管线共用同一套命中循环；坐标在所有显示器之外时返回 None。
fn monitor_at(mx: i32, my: i32) -> Option<(Screen, i32, i32, i32, i32)> {
    let screens = match Screen::all() {
        Ok(v) => v,
        Err(e) => {
            warn!("[solver] 枚举显示器失败: {}", e);
            return None;
        }
    };
    for screen in screens {
        let d = screen.display_info;
        let (dx, dy) = (d.x, d.y);
        let (dw, dh) = (d.width as i32, d.height as i32);
        if mx >= dx && my >= dy && mx < dx + dw && my < dy + dh {
            return Some((screen, dx, dy, dw, dh));
        }
    }
    None
}

// 抓鼠标所在显示器的全屏图，写入 pot_screenshot_full.png。
// v1.2.7 起裁切模式生效时只写入框选区域（交集裁剪）
pub fn capture_fullscreen() -> Result<(), String> {
    use mouse_position::mouse_position::Mouse;

    let (mx, my) = match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => (x, y),
        // 静默回退 (0,0) 会把截图打到主屏并吞掉真实故障，改为向上传播
        Mouse::Error => return Err("获取鼠标位置失败".to_string()),
    };
    let (screen, dx, dy, _dw, _dh) =
        monitor_at(mx, my).ok_or_else(|| format!("mouse monitor not found: {},{}", mx, my))?;
    let img = screen.capture().map_err(|e| e.to_string())?;
    let raw = image::RgbaImage::from_raw(img.width(), img.height(), img.rgba().clone())
        .ok_or_else(|| "截图缓冲尺寸不符".to_string())?;
    let cropped = apply_crop_region(raw, dx, dy);
    let buf = to_png(&DynamicImage::ImageRgba8(cropped))?;
    let path = cache_file("pot_screenshot_full.png")?;
    let _io = SHOT_IO.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::write(&path, buf).map_err(|e| e.to_string())?;
    info!("[solver] 全屏截图完成: {:?}", path);
    Ok(())
}

// 抓鼠标所在显示器的全屏图，直接返回内存 RGBA（不落盘）。
// 供高频轮询管线（西蒙符印录制）复用。screenshots 0.7.2 的 Image 内部已是
// 转换好的 RGBA（win32 路径经 Image::from_bgra 转换，alpha=255），直接按长度
// 校验取用——不再经 PNG 编码+解码兜圈子（旧写法一帧 2560x1440 要多花
// ~300-500ms，把西蒙录制实际采样周期从 200ms 拖到 600ms+，游戏符号 ~0.5s
// 的亮起闪现在两帧滞回确认之间结束，一步都录不上：v1.2.4 修复的根因）。
pub fn capture_monitor_to_mem() -> Result<image::RgbaImage, String> {
    use mouse_position::mouse_position::Mouse;

    let (mx, my) = match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => (x, y),
        // 静默回退 (0,0) 会把截图打到主屏并吞掉真实故障，改为向上传播
        Mouse::Error => return Err("获取鼠标位置失败".to_string()),
    };
    let (screen, dx, dy, _dw, _dh) =
        monitor_at(mx, my).ok_or_else(|| format!("mouse monitor not found: {},{}", mx, my))?;
    let img = screen.capture().map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    let raw = img.rgba().clone();
    let img = image::RgbaImage::from_raw(w, h, raw).ok_or_else(|| "截图缓冲尺寸不符".into());
    // 裁切模式生效时只保留框选区域（高频轮询管线同样受益：图更小、检测更快）
    return img.map(|v| apply_crop_region(v, dx, dy));
}

// 读取缓存截图。按内容嗅探格式（兼容测试时直接拷入的 JPG），不依赖扩展名
pub fn load_screenshot(name: &str) -> Result<image::RgbaImage, String> {
    let path = cache_file(name)?;
    if !path.exists() {
        return Err("截图缓存不存在".into());
    }
    let _io = SHOT_IO.lock().unwrap_or_else(|e| e.into_inner());
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let img = reader.decode().map_err(|e| format!("截图解码失败: {}", e))?;
    Ok(img.to_rgba8())
}

pub fn to_png(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {}", e))?;
    Ok(buf)
}

// 在图像四周加 margin 边距：文字贴边的 ROI 送 OCR 识别率极差（旧管线实测主因）
pub fn pad_canvas(img: &DynamicImage, margin: u32, bg: image::Rgba<u8>) -> DynamicImage {
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
pub mod ocr {
    use windows::core::{Interface, HSTRING};
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    // 引擎类型对外可见（转轮模块测试需要命名它）
    pub use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{
        DataWriter, InMemoryRandomAccessStream, IOutputStream, IRandomAccessStream,
    };

    fn try_engine(tag: &str) -> Option<OcrEngine> {
        let lang = Language::CreateLanguage(&HSTRING::from(tag)).ok()?;
        OcrEngine::TryCreateFromLanguage(&lang).ok()
    }

    // 中文引擎：优先 zh-Hans-CN，失败则扫描系统语言表中的 zh-Hans 变体。
    // 全部失败 = 系统无中文 OCR 语言包（调用方降级到字形模板匹配）
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
        let bmp = decoder.GetSoftwareBitmapAsync().map_err(err)?.get().map_err(err)?;
        let result = engine.RecognizeAsync(&bmp).map_err(err)?.get().map_err(err)?;
        let text = result.Text().map_err(err)?;
        Ok(String::from_utf16_lossy(text.as_wide()))
    }
}

#[cfg(not(target_os = "windows"))]
pub mod ocr {
    // 非 Windows 平台占位：管线不可用，调用方降级为字形模板匹配
    pub struct Engine;
    pub type OcrEngine = Engine;
    pub fn zh_engine() -> Option<Engine> {
        None
    }
    pub fn recognize_png(_engine: &Engine, _png: &[u8]) -> Result<String, String> {
        Err("仅支持 Windows".into())
    }
}
