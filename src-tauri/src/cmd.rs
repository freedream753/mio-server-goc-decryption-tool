use crate::config::StoreWrapper;
use crate::error::Error;
use crate::APP;
use log::{error, info, warn};
use tauri::Manager;

#[tauri::command]
pub fn reload_store() {
    let state = APP.get().unwrap().state::<StoreWrapper>();
    let mut store = state.0.lock().unwrap();
    store.load().unwrap();
}

#[tauri::command]
pub fn cut_image(left: u32, top: u32, width: u32, height: u32, app_handle: tauri::AppHandle) {
    use dirs::cache_dir;
    use image::GenericImage;
    info!("Cut image: {}x{}+{}+{}", width, height, left, top);
    let mut app_cache_dir_path = cache_dir().expect("Get Cache Dir Failed");
    app_cache_dir_path.push(&app_handle.config().tauri.bundle.identifier);
    app_cache_dir_path.push("pot_screenshot.png");
    if !app_cache_dir_path.exists() {
        return;
    }
    let mut img = match image::open(&app_cache_dir_path) {
        Ok(v) => v,
        Err(e) => {
            error!("{:?}", e.to_string());
            return;
        }
    };
    let img2 = img.sub_image(left, top, width, height);
    app_cache_dir_path.pop();
    app_cache_dir_path.push("pot_screenshot_cut.png");
    match img2.to_image().save(&app_cache_dir_path) {
        Ok(_) => {}
        Err(e) => {
            error!("{:?}", e.to_string());
        }
    }
}

#[tauri::command]
pub fn log_event(level: String, message: String) {
    // 前端失败事件转发：经日志插件落盘到日志文件，供用户通过托盘「查看日志」打开并带文件反馈。
    // 隐私边界：只记失败原因与错误信息，不记录识别出的文本内容。
    match level.as_str() {
        "error" => error!("[frontend] {}", message),
        "warn" => warn!("[frontend] {}", message),
        _ => info!("[frontend] {}", message),
    }
}

#[tauri::command]
pub fn font_list() -> Result<Vec<String>, Error> {
    use font_kit::source::SystemSource;
    let source = SystemSource::new();

    Ok(source.all_families()?)
}

// 结果窗口「OCR 框选」按钮入口（框选降级路径）：
// 先隐藏结果窗口（保证不出现在截图里），再拉起全屏框选窗；框选完成后
// pot_screenshot_cut.png 由 runes::rune_scan_manual 接同一符文识别管线。
// 窗口创建必须挪到独立线程：在 tauri 命令（IPC 回调）上下文里同步创建
// WebView2 窗口会发生重入，协议请求管线被卡死，表现为新窗口永远停在
// about:blank 且后续所有窗口的 WebView2 操作被连带卡死（应用假死）。
#[tauri::command]
pub fn ocr_recognize() {
    crate::window::hide_result_window();
    std::thread::spawn(crate::window::ocr_recognize);
}

// 结果窗口前端挂载后拉取最新符文识别结果（避免 emit 早于监听注册的竞态）
#[tauri::command]
pub fn get_rune_scan() -> Option<serde_json::Value> {
    crate::runes::RUNE_SCAN.lock().unwrap().clone()
}

// 结果窗口显示（SW_SHOWNA + 置顶，绝不抢焦点），由前端渲染完成后调用
#[tauri::command]
pub fn show_result_window() {
    crate::window::show_result_window();
}

// 前端交互（点击复制/按下窗口）重置 10 秒倒计时时调用，同步推迟 Rust 侧兜底隐藏
#[tauri::command]
pub fn reset_result_timer() {
    crate::window::reset_result_timer();
}

// 前端按下拖拽区调用：倒计时重置为 10s 并保持不动，左键松开后等 2s 重新计
#[tauri::command]
pub fn begin_result_drag() {
    crate::window::begin_result_drag();
}

// 前端关闭按钮调用：立即隐藏结果窗口
#[tauri::command]
pub fn hide_result_window() {
    crate::window::hide_result_window();
}

// 需要键盘输入的场景由前端在输入框聚焦时调用（如手动纠正符文）：
// 程序化激活结果窗口让键盘输入到达（NOACTIVATE 窗口点击不会激活）
#[tauri::command]
pub fn focus_result_window() {
    crate::window::focus_result_window();
}
