use crate::config::StoreWrapper;
use crate::error::Error;
use crate::APP;
use log::{error, info, warn};
use tauri::Manager;

#[tauri::command]
pub fn reload_store() {
    let state = APP.get().unwrap().state::<StoreWrapper>();
    let mut store = state.0.lock().unwrap_or_else(|e| e.into_inner());
    // config.json 可能损坏/被手改成非法内容：重载失败仅记日志并保留内存态
    // （与启动路径的容忍行为一致），不得 panic
    if let Err(e) = store.load() {
        error!("重载配置失败: {}", e);
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

// 手绘点拟合（noocr 支线）：前端画布采集归一化点集（x∈[0,1]，y∈[-1,1] 向上为正）
#[tauri::command]
pub fn wave_fit_points(points: Vec<(f64, f64)>) -> serde_json::Value {
    crate::wave::fit_points(points.into_iter().map(|(x, y)| (x as f32, y as f32)).collect())
}

// 读取待切页签（读即清，防 emit 早于前端监听注册的竞态，仿 get_wave_scan 模式）
#[tauri::command]
pub fn get_open_tab() -> Option<String> {
    crate::window::take_pending_tab()
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
