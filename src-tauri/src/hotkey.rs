use crate::config::{get, set};
use crate::solver::solver_recognize;
use crate::APP;
use log::{error, info, warn};
use tauri::{AppHandle, GlobalShortcutManager};

fn register<F>(app_handle: &AppHandle, name: &str, handler: F, key: &str) -> Result<(), String>
where
    F: Fn() + Send + 'static,
{
    let hotkey = {
        if key.is_empty() {
            match get(name) {
                Some(v) => v.as_str().unwrap().to_string(),
                None => {
                    // 未配置过热键时写入默认值（左 Alt+C，Windows 全局热键不区分左右 Alt）
                    set(name, "Alt+C");
                    "Alt+C".to_string()
                }
            }
        } else {
            key.to_string()
        }
    };

    if !hotkey.is_empty() {
        match app_handle
            .global_shortcut_manager()
            .register(hotkey.as_str(), handler)
        {
            Ok(()) => {
                info!("Registered global shortcut: {} for {}", hotkey, name);
            }
            Err(e) => {
                warn!("Failed to register global shortcut: {} {:?}", hotkey, e);
                return Err(e.to_string());
            }
        };
    }
    Ok(())
}

// Register global shortcuts
pub fn register_shortcut(shortcut: &str) -> Result<(), String> {
    let app_handle = APP.get().unwrap();
    match shortcut {
        "hotkey_ocr_recognize" => register(app_handle, "hotkey_ocr_recognize", solver_recognize, "")?,
        "all" => {
            register(app_handle, "hotkey_ocr_recognize", solver_recognize, "")?;
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub fn register_shortcut_by_frontend(name: &str, shortcut: &str) -> Result<(), String> {
    let app_handle = APP.get().unwrap();
    match name {
        "hotkey_ocr_recognize" => {
            register(app_handle, "hotkey_ocr_recognize", solver_recognize, shortcut)?
        }
        _ => {}
    }
    Ok(())
}

// 更新热键（带事务性回滚）：先注销旧键 -> 注册新键，注册失败则回滚重注旧键。
// store 仅在注册成功后写入，保证「配置值 = 实际生效的热键」，失败不污染配置。
#[tauri::command]
pub fn update_shortcut(name: &str, shortcut: String) -> Result<(), String> {
    let app_handle = APP.get().unwrap();
    let mut manager = app_handle.global_shortcut_manager();
    let new_key = shortcut.trim().to_string();
    if new_key.is_empty() {
        return Err("快捷键为空".into());
    }
    // 旧键取 store 当前值（= 上次注册成功并生效的键），不依赖前端状态
    let old = get(name)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if new_key == old {
        return Ok(());
    }
    if !old.is_empty() {
        let _ = manager.unregister(&old);
    }
    match manager.register(&new_key, solver_recognize) {
        Ok(()) => {
            set(name, new_key.clone());
            info!("Hotkey updated: {} -> {} for {}", old, new_key, name);
            Ok(())
        }
        Err(e) => {
            // 回滚：重新注册旧键，保证本会话热键不丢失
            if !old.is_empty() {
                if let Err(re) = manager.register(&old, solver_recognize) {
                    error!("Hotkey rollback failed for {}: {:?}", old, re);
                }
            }
            warn!(
                "Failed to register new hotkey {} for {}: {:?}",
                new_key, name, e
            );
            Err(format!(
                "注册失败，已保留原快捷键 {}：{}",
                if old.is_empty() { "无" } else { &old },
                e
            ))
        }
    }
}
