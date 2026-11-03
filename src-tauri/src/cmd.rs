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
pub fn get_base64(app_handle: tauri::AppHandle) -> String {
    use base64::{engine::general_purpose, Engine as _};
    use dirs::cache_dir;
    use std::fs::File;
    use std::io::Read;
    // 与 not exists 分支一致容错：目录缺失或文件被占用时返回空串，避免主流程 panic
    let mut app_cache_dir_path = match cache_dir() {
        Some(v) => v,
        None => return "".to_string(),
    };
    app_cache_dir_path.push(&app_handle.config().tauri.bundle.identifier);
    app_cache_dir_path.push("pot_screenshot_cut.png");
    if !app_cache_dir_path.exists() {
        return "".to_string();
    }
    let mut file = match File::open(&app_cache_dir_path) {
        Ok(v) => v,
        Err(e) => {
            error!("Open screenshot file failed: {}", e);
            return "".to_string();
        }
    };
    let mut vec = Vec::new();
    match file.read_to_end(&mut vec) {
        Ok(_) => {}
        Err(e) => {
            error!("{:?}", e.to_string());
            return "".to_string();
        }
    }
    let base64 = general_purpose::STANDARD.encode(&vec);
    base64.replace("\r\n", "")
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

// ---------------- v1.0.3 自动解题管线 ----------------

// 结果窗口前端挂载后拉取最新解题结果（避免 emit 早于监听注册的竞态）
#[tauri::command]
pub fn get_solve_result() -> Option<serde_json::Value> {
    crate::window::SOLVE_RESULT.lock().unwrap().clone()
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

// 手动位移输入框聚焦时调用：程序化激活结果窗口让键盘输入到达（NOACTIVATE 窗口点击不激活）
#[tauri::command]
pub fn focus_result_window() {
    crate::window::focus_result_window();
}

// 设置页/结果窗口用：当前生效词表（自定义 words.json 覆盖优先，否则内置默认）
#[tauri::command]
pub fn get_answer_words() -> Vec<String> {
    crate::solver::answer_words()
}

// 设置页用：当前是否为自定义词表（words.json 是否存在），控制「恢复默认」按钮状态
#[tauri::command]
pub fn is_custom_words() -> bool {
    crate::solver::is_custom_words()
}

// 设置页：保存自定义答案词表（立即生效并持久化）
#[tauri::command]
pub fn save_answer_words(words: Vec<String>) -> Result<usize, String> {
    crate::solver::save_answer_words(words)
}

// 设置页：恢复内置默认词表
#[tauri::command]
pub fn reset_answer_words() -> Result<(), String> {
    crate::solver::reset_answer_words()
}

// 自学习：结果窗口答案修正入口，写入本地 JSON，后续识别优先参考
// 错误上抛给前端提示（写盘失败时不能静默成功，误导用户以为修正已生效）
#[tauri::command]
pub fn save_correction(cipher: String, answer: String) -> Result<(), String> {
    crate::solver::save_correction(&cipher, &answer)
}

// 自学习：设置页读取词库（密文 -> 答案映射）
#[tauri::command]
pub fn get_corrections() -> std::collections::HashMap<String, String> {
    crate::solver::get_corrections()
}

// 自学习：设置页删除一条记录
#[tauri::command]
pub fn del_correction(cipher: String) -> Result<(), String> {
    crate::solver::del_correction(&cipher)
}
