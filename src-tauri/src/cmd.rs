use crate::config::StoreWrapper;
use crate::error::Error;
use crate::APP;
use image::GenericImageView;
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
    let _io = crate::solver::SHOT_IO.lock().unwrap_or_else(|e| e.into_inner());
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
    // IPC 四参数直通无钳制：越界的 sub_image 会 panic。起点钳到 [0, iw-1]/
    // [0, ih-1]，宽高钳到剩余可用空间，保证裁剪始终落在图像范围内
    let orig = (left, top, width, height);
    let (iw, ih) = img.dimensions();
    let left = left.min(iw.saturating_sub(1));
    let top = top.min(ih.saturating_sub(1));
    let width = width.min(iw - left);
    let height = height.min(ih - top);
    if (left, top, width, height) != orig {
        warn!(
            "框选参数越界已钳制: {}x{}+{}+{} -> {}x{}+{}+{}（图像 {}x{}）",
            orig.2, orig.3, orig.0, orig.1, width, height, left, top, iw, ih
        );
    }
    if width == 0 || height == 0 {
        error!(
            "框选区域钳制后为空，丢弃本次落图: {}x{}+{}+{}（图像 {}x{}）",
            width, height, left, top, iw, ih
        );
        return;
    }
    let img2 = img.sub_image(left, top, width, height);
    app_cache_dir_path.pop();
    app_cache_dir_path.push("pot_screenshot_cut.png");
    match img2.to_image().save(&app_cache_dir_path) {
        Ok(_) => {}
        Err(e) => {
            error!("{:?}", e.to_string());
        }
    }
    // 记录最近一次框选区域（物理像素，钳制后的实际值），供波形画布框选（wave）派发使用
    *crate::window::LAST_CUT.lock().unwrap_or_else(|e| e.into_inner()) = Some((left, top, width, height));
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

// ---------------- 全局裁切模式（v1.2.7；v1.4.0 起为唯一识别前提） ----------------
// 结果窗口标题旁按钮的两个状态：框选（进入裁切模式）/ 取消裁切模式。
// 进入：置派发用途 → 隐藏结果窗口 → 拉全屏框选窗，框选完成后由 window.rs 的
// crop_selected 落定 CROP_REGION 并浮现置顶点击穿透指示框。
#[tauri::command]
pub fn crop_select() {
    // 与热键「未开裁切先框选」共用同一入口；窗口按钮触发无后续动作
    crate::window::begin_crop_selection("");
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
    crate::runes::RUNE_SCAN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// 解密页「识别」按钮：与热键 Alt+V 同一入口（内部起线程，快速返回）。
// v1.4.0 窗口内操作门控：未开裁切不代拉框选，发 crop_required 由前端提示
#[tauri::command]
pub fn rune_recognize() {
    if !crate::window::require_crop_for_window() {
        return;
    }
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
    crate::wave::WAVE_SCAN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// 第一关：波形求解（热键 Alt+X 同一入口）。v1.4.0 窗口内操作门控：
// 未开裁切不代拉框选，发 crop_required 由前端提示
#[tauri::command]
pub fn wave_scan_cmd() {
    if !crate::window::require_crop_for_window() {
        return;
    }
    crate::wave::wave_scan();
}

// 第一关：手动框选波形画布区域求解（自动定位被遮挡/干扰时的降级入口）：
// v1.4.0 封存——手动框选属全屏实现，默认不编入构建（legacy-fullscreen 特性可恢复）。
// 所有识别统一先经全局裁切模式
#[tauri::command]
pub fn wave_select_region() {
    #[cfg(feature = "legacy-fullscreen")]
    {
        // PURPOSE_IO 串行化「设置用途→起线程」段（与 crop_select 同一把锁）：
        // try_lock 失败说明已有框选流程在途，放弃本次触发防止用途值被覆盖
        let _io = match crate::window::PURPOSE_IO.try_lock() {
            Ok(g) => g,
            Err(_) => {
                warn!("[wave] 已有框选流程在途，忽略本次触发");
                return;
            }
        };
        crate::window::set_shot_purpose("wave");
        // 临时隐藏（不重置定位标志）：框选完成后窗口原位显示，不重定向到鼠标
        crate::window::hide_result_window_temp();
        std::thread::spawn(crate::window::ocr_recognize);
    }
    #[cfg(not(feature = "legacy-fullscreen"))]
    {
        warn!("[wave] 手动框选已封存（全屏实现弃用），请使用全局裁切模式");
    }
}

// 第二关：西蒙符印录制开始/停止 + 步骤拉取
// start 带 target：仅 0 = 自由录制（原 4/5/6 三关式已移除，自动采集走 simon_start_auto）
// v1.4.0 窗口内操作门控：未开裁切不代拉框选，发 crop_required 由前端提示
#[tauri::command]
pub fn simon_start(target: usize) {
    if !crate::window::require_crop_for_window() {
        return;
    }
    crate::simon::simon_start_with_target(target);
}

// 自动模式：状态区文字（开始/正在播放/轮次X/3/错误）驱动相位机，见 simon::auto_loop
// v1.4.0 窗口内操作门控：热键入口（simon_toggle）未开裁切时先拉框选，窗口入口只提示
#[tauri::command]
pub fn simon_start_auto() {
    if !crate::window::require_crop_for_window() {
        return;
    }
    crate::simon::simon_start_auto();
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
