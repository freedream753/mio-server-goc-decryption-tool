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

// 截图窗口框选落图（异步命令：4K 全屏 PNG 解码+裁剪+编码要 1s+，同步命令会
// 阻塞主线程，表现为框选完成后界面卡住一下）。落图写 LAST_CUT 后前端 emit
// "success"，由 window.rs 按 SHOT_PURPOSE 派发（符文/波形/全局裁切）。
#[tauri::command]
pub async fn cut_image(left: u32, top: u32, width: u32, height: u32, app_handle: tauri::AppHandle) {
    use dirs::cache_dir;
    use image::GenericImage;
    info!("Cut image: {}x{}+{}+{}", width, height, left, top);
    // 缓存截图读写统一走 SHOT_IO 串行（与 solver.rs 的读写锁约定一致）：
    // 波形框选分析正 load_screenshot 读 cut.png 时，再次框选落图不得并发覆盖写
    let _io = crate::solver::SHOT_IO.lock().unwrap();
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
    // 记录最近一次框选区域（物理像素），供波形画布框选（wave）派发使用
    *crate::window::LAST_CUT.lock().unwrap() = Some((left, top, width, height));
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

// ---------------- 全局裁切模式（v1.2.7） ----------------
// 结果窗口标题旁按钮的两个状态：框选（进入裁切模式）/ 取消裁切模式。
// 进入：置派发用途 → 隐藏结果窗口 → 拉全屏框选窗，框选完成后由 window.rs 的
// crop_selected 落定 CROP_REGION 并浮现置顶点击穿透指示框。
// 窗口创建必须挪到独立线程：在 tauri 命令（IPC 回调）上下文里同步创建
// WebView2 窗口会发生重入，协议请求管线被卡死（应用假死）。
#[tauri::command]
pub fn crop_select() {
    crate::window::set_shot_purpose("crop");
    // 临时隐藏（不重置定位标志）：框选完成后窗口原位显示，不重定向到鼠标
    crate::window::hide_result_window_temp();
    std::thread::spawn(crate::window::ocr_recognize);
}

#[tauri::command]
pub fn crop_clear() {
    crate::window::crop_clear();
}

// 结果窗口挂载后拉取当前裁切模式状态（防 emit 早于监听注册的竞态）
#[tauri::command]
pub fn get_crop_state() -> bool {
    crate::window::crop_region().is_some()
}

// 结果窗口前端挂载后拉取最新符文识别结果（避免 emit 早于监听注册的竞态）
#[tauri::command]
pub fn get_rune_scan() -> Option<serde_json::Value> {
    crate::runes::RUNE_SCAN.lock().unwrap().clone()
}

// 解密页「识别」按钮：与热键 Alt+V 同一入口（内部起线程，快速返回）
#[tauri::command]
pub fn rune_recognize() {
    crate::runes::rune_scan();
}

// 结果窗口显示（SW_SHOWNA + 置顶，绝不抢焦点），由前端渲染完成后调用
#[tauri::command]
pub fn show_result_window() {
    crate::window::show_result_window();
}

// 前端关闭按钮调用：立即隐藏结果窗口
#[tauri::command]
pub fn hide_result_window() {
    crate::window::hide_result_window();
}

// 结果窗口按页签调整尺寸（逻辑像素，保持位置不变）：
// 密码/波形/西蒙各页高度不同，由前端在切换页签时调用
#[tauri::command]
pub fn set_result_size(width: f64, height: f64) {
    crate::window::set_result_window_size(width, height);
}

// ---------------- v1.2.0 四关卡助手 ----------------

// 第一关：助手窗口前端挂载后拉取最新波形求解结果（避免 emit 竞态）
#[tauri::command]
pub fn get_wave_scan() -> Option<serde_json::Value> {
    crate::wave::WAVE_SCAN.lock().unwrap().clone()
}

// 第一关：波形求解（热键 Alt+X 同一入口）
#[tauri::command]
pub fn wave_scan_cmd() {
    crate::wave::wave_scan();
}

// 第一关：手动框选波形画布区域求解（自动定位被遮挡/干扰时的降级入口）：
// 置派发用途 → 隐藏结果窗口（不出现在截图里）→ 拉全屏框选窗，
// 框选完成后 success 事件派发到 wave::on_region_selected
#[tauri::command]
pub fn wave_select_region() {
    crate::window::set_shot_purpose("wave");
    // 临时隐藏（不重置定位标志）：框选完成后窗口原位显示，不重定向到鼠标
    crate::window::hide_result_window_temp();
    std::thread::spawn(crate::window::ocr_recognize);
}

// 第二关：西蒙符印录制开始/停止 + 步骤拉取
// start 带 target：4/5/6 = 第一/二/三关（录满自动暂停），0 = 自由录制
#[tauri::command]
pub fn simon_start(target: usize) {
    crate::simon::simon_start_with_target(target);
}

#[tauri::command]
pub fn simon_stop() {
    crate::simon::simon_stop();
}

#[tauri::command]
pub fn get_simon_steps() -> Vec<serde_json::Value> {
    crate::simon::get_simon_steps()
}

// 西蒙录制运行状态（结果窗口挂载后拉取：录制已在跑则切到西蒙页签）
#[tauri::command]
pub fn get_simon_state() -> bool {
    crate::simon::simon_running()
}
