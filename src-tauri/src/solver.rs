// 截图 / OCR 引擎封装等复用工具。
// v1.1.0 起整条凯撒解密链路（卡片像素定位 + 标签解析 + 词表反查 + 自学习修正）
// 已移除，本模块只保留符文识别管线仍需要的基础能力。

use crate::APP;
use image::DynamicImage;
use log::info;
use std::io::Cursor;
use std::path::PathBuf;

// 缓存文件绝对路径：<cache>/<bundle identifier>/<name>
pub fn cache_file(name: &str) -> Result<PathBuf, String> {
    let app = APP.get().ok_or("app not initialized")?;
    let dir = dirs::cache_dir().ok_or("cache dir not found")?;
    Ok(dir.join(&app.config().tauri.bundle.identifier).join(name))
}

// 抓鼠标所在显示器的全屏图，写入 pot_screenshot_full.png
pub fn capture_fullscreen() -> Result<(), String> {
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

// 读取缓存截图。按内容嗅探格式（兼容测试时直接拷入的 JPG），不依赖扩展名
pub fn load_screenshot(name: &str) -> Result<image::RgbaImage, String> {
    let path = cache_file(name)?;
    if !path.exists() {
        return Err("截图缓存不存在".into());
    }
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
    use windows::Media::Ocr::OcrEngine;
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
    pub fn zh_engine() -> Option<Engine> {
        None
    }
    pub fn recognize_png(_engine: &Engine, _png: &[u8]) -> Result<String, String> {
        Err("仅支持 Windows".into())
    }
}
