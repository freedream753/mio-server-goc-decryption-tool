use crate::config::{get, set};
use crate::runes::rune_scan;
use crate::window::config_window;
use log::{info, warn};
use tauri::CustomMenuItem;
use tauri::GlobalShortcutManager;
use tauri::SystemTrayEvent;
use tauri::SystemTrayMenu;
use tauri::SystemTrayMenuItem;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn update_tray(app_handle: tauri::AppHandle, mut language: String, _copy_mode: String) {
    let tray_handle = app_handle.tray_handle();

    if language.is_empty() {
        language = match get("app_language") {
            // 配置可能被手改成非字符串：回退英文菜单，不 panic
            Some(v) => v
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| "en".to_string()),
            None => {
                set("app_language", "en");
                "en".to_string()
            }
        };
    }

    info!("Update tray with language: {}", language);
    tray_handle
        .set_menu(match language.as_str() {
            "zh_cn" => tray_menu_zh_cn(),
            "zh_tw" => tray_menu_zh_tw(),
            _ => tray_menu_en(),
        })
        .unwrap();
    #[cfg(not(target_os = "linux"))]
    tray_handle
        .set_tooltip(&format!("GOC解题妙妙小工具 {}", app_handle.package_info().version))
        .unwrap();
}

pub fn tray_event_handler<'a>(app: &'a AppHandle, event: SystemTrayEvent) {
    match event {
        #[cfg(target_os = "windows")]
        SystemTrayEvent::LeftClick { .. } => on_tray_click(),
        SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
            "ocr_recognize" => on_ocr_recognize_click(),
            "config" => on_config_click(),
            "view_log" => on_view_log_click(app),
            "restart" => on_restart_click(app),
            "quit" => on_quit_click(app),
            _ => {}
        },
        _ => {}
    }
}

#[cfg(target_os = "windows")]
fn on_tray_click() {
    let event = match get("tray_click_event") {
        // 配置可能被手改成非字符串：回退默认行为（打开偏好设置），不 panic
        Some(v) => v
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| "config".to_string()),
        None => {
            set("tray_click_event", "config");
            "config".to_string()
        }
    };
    match event.as_str() {
        "config" => config_window(),
        "ocr_recognize" => rune_scan(),
        "disable" => {}
        _ => config_window(),
    }
}

fn on_ocr_recognize_click() {
    rune_scan();
}

fn on_config_click() {
    config_window();
}

fn on_view_log_click(app: &AppHandle) {
    use tauri::api::notification;
    use tauri::api::path::app_log_dir;
    let log_path = app_log_dir(&app.config()).unwrap();
    // 打开失败只告警 + 系统通知用户，不在托盘事件回调里 panic
    if let Err(e) = tauri::api::shell::open(&app.shell_scope(), log_path.to_str().unwrap(), None) {
        warn!("打开日志目录失败: {}", e);
        let _ = notification::Notification::new(&app.config().tauri.bundle.identifier)
            .title("GOC解题妙妙小工具")
            .body("打开日志目录失败，请手动查看日志文件")
            .show();
    }
}
fn on_restart_click(app: &AppHandle) {
    info!("============== Restart App ==============");
    // 不能用 app.restart()：它先 spawn 新进程再退出旧进程，新进程会被
    // single_instance 插件判定为重复启动而自杀（旧进程尚未释放锁）。
    // 改为 spawn 自身并携带等待参数，新进程等旧实例完全退出后再初始化。
    // --goc-restart-pid 带上旧进程（当前）PID，供新进程精确轮询退出，
    // 替代新进程侧的固定 sleep（旧进程退出慢于固定时长时新进程会被
    // single_instance 静默退出）。
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg("--goc-restart-wait")
            .arg(format!("--goc-restart-pid={}", std::process::id()))
            .spawn();
    }
    app.exit(0);
}
fn on_quit_click(app: &AppHandle) {
    app.global_shortcut_manager().unregister_all().unwrap();
    info!("============== Quit App ==============");
    app.exit(0);
}

fn tray_menu_en() -> tauri::SystemTrayMenu {
    let ocr_recognize = CustomMenuItem::new("ocr_recognize", "Screenshot -> Rune Identify");
    let config = CustomMenuItem::new("config", "Config");
    let view_log = CustomMenuItem::new("view_log", "View Log");
    let restart = CustomMenuItem::new("restart", "Restart");
    let quit = CustomMenuItem::new("quit", "Quit");
    SystemTrayMenu::new()
        .add_item(ocr_recognize)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(config)

        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}

fn tray_menu_zh_cn() -> tauri::SystemTrayMenu {
    let ocr_recognize = CustomMenuItem::new("ocr_recognize", "截图识别符文");
    let config = CustomMenuItem::new("config", "偏好设置");
    let restart = CustomMenuItem::new("restart", "重启应用");
    let view_log = CustomMenuItem::new("view_log", "查看日志");
    let quit = CustomMenuItem::new("quit", "退出");
    SystemTrayMenu::new()
        .add_item(ocr_recognize)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(config)

        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}

fn tray_menu_zh_tw() -> tauri::SystemTrayMenu {
    let ocr_recognize = CustomMenuItem::new("ocr_recognize", "截圖識別符文");
    let config = CustomMenuItem::new("config", "偏好設定");
    let restart = CustomMenuItem::new("restart", "重啓程式");
    let view_log = CustomMenuItem::new("view_log", "查看日誌");
    let quit = CustomMenuItem::new("quit", "退出");
    SystemTrayMenu::new()
        .add_item(ocr_recognize)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(config)

        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}
