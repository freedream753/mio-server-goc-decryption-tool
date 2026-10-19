// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cmd;
mod config;
mod error;
mod hotkey;
mod screenshot;
mod server;
mod tray;
mod window;

use cmd::*;
use config::*;
use hotkey::*;
use log::{info, warn};
use once_cell::sync::OnceCell;
use screenshot::screenshot;
use server::*;
use tauri::api::notification::Notification;
use tauri::Manager;
use tauri_plugin_log::LogTarget;
use tray::*;
use window::config_window;

// Global AppHandle
pub static APP: OnceCell<tauri::AppHandle> = OnceCell::new();

// 注册系统通知 AUMID（便携版无安装器，不注册则 WinRT 通知无法显示）
#[cfg(target_os = "windows")]
fn register_notification_aumid(identifier: String) {
    std::thread::spawn(move || {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let base = format!(r"HKCU\Software\Classes\AppUserModelId\{}", identifier);
        // 解包内置图标到配置目录，作为通知图标
        let icon_str = match dirs::config_dir() {
            Some(dir) => {
                let icon_path = dir.join(&identifier).join("notify_icon.png");
                if let Some(parent) = icon_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::write(&icon_path, include_bytes!("../icons/icon.png")).is_ok() {
                    icon_path.to_string_lossy().to_string()
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };
        let ok_display = std::process::Command::new("reg")
            .args(["add", &base, "/v", "DisplayName", "/d", "GOC解题妙妙小工具", "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok();
        let ok_icon = if icon_str.is_empty() {
            true
        } else {
            std::process::Command::new("reg")
                .args(["add", &base, "/v", "IconUri", "/d", &icon_str, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .is_ok()
        };
        info!(
            "Register notification AUMID: display={}, icon={}",
            ok_display, ok_icon
        );
    });
}

fn main() {
    // 重启流程：旧进程 spawn 自身（带 --goc-restart-wait）后退出。
    // 此处先等旧实例完全退出、释放 single_instance 锁与端口，再正常初始化。
    if std::env::args().any(|arg| arg == "--goc-restart-wait") {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, cwd| {
            // 通知失败不应导致主实例 panic（如 AUMID 尚未注册完成）
            let _ = Notification::new(&app.config().tauri.bundle.identifier)
                .title("The program is already running. Please do not start it again!")
                .body(cwd)
                .show();
        }))
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([LogTarget::LogDir, LogTarget::Stdout])
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_fs_watch::init())
        .system_tray(tauri::SystemTray::new())
        .setup(|app| {
            info!("============== Start App ==============");
            // Global AppHandle
            APP.get_or_init(|| app.handle());
            // Init Config
            info!("Init Config Store");
            init_config(app);
            // Register notification AUMID (Windows)
            #[cfg(target_os = "windows")]
            register_notification_aumid(app.config().tauri.bundle.identifier.clone());
            // Check First Run
            if is_first_run() {
                // Open Config Window
                info!("First Run, opening config window");
                config_window();
            }
            // Update Tray Menu
            update_tray(app.app_handle(), "".to_string(), "".to_string());
            // Start http server
            start_server();
            // Register Global Shortcut
            match register_shortcut("all") {
                Ok(()) => {}
                Err(e) => {
                    // 热键冲突常见，通知失败再 panic 会导致启动崩溃，降级为日志
                    warn!("Failed to register global shortcut: {}", e);
                    let _ = Notification::new(app.config().tauri.bundle.identifier.clone())
                        .title("Failed to register global shortcut")
                        .body(&e)
                        .show();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            reload_store, cut_image, get_base64, register_shortcut_by_frontend, update_tray, screenshot, font_list
        ])
        .on_system_tray_event(tray_event_handler)
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        // 窗口关闭不退出
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
