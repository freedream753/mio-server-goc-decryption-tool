use crate::config::{get, set};
use crate::runes::rune_scan;
use crate::wave::wave_scan;
use crate::APP;
use log::{error, info, warn};
use std::sync::Mutex;
use tauri::{AppHandle, GlobalShortcutManager};

// 全局热键注册/注销互斥锁：update_shortcut 的 unregister→register 序列非原子，
// 与启动注册（register_shortcut）、前端单键注册并发交错时会互相覆盖，导致热键
// 丢失或重复注册失败。所有触碰这批全局热键的入口统一在此锁内串行执行；锁被
// panic 毒化时降级继续用（into_inner），不让单次故障永久卡死热键功能
static HOTKEY_IO: Mutex<()> = Mutex::new(());

// 按热键名选 handler：第一关波形校准走 wave_scan，西蒙走开始/停止录制切换，其余走符文识别
fn handler_for(name: &str) -> fn() {
    match name {
        "hotkey_wave_scan" => wave_scan,
        "hotkey_simon" => crate::simon::simon_toggle,
        _ => rune_scan,
    }
}

// 各热键的默认键位（未配置过时写入）：Alt+X 波形校准 / Alt+C 西蒙录制 / Alt+V 解密识别
fn default_key(name: &str) -> &'static str {
    match name {
        "hotkey_wave_scan" => "Alt+X",
        "hotkey_simon" => "Alt+C",
        _ => "Alt+V",
    }
}

// v1.2.7 默认键位轮换（校准 V→X、西蒙 X→C、解密 C→V）：已持久化旧默认值的
// 配置一次性迁移到新默认，避免老用户按键不变以为改动没生效。用户自定义过的
// 键位（≠旧默认）不动。迁移是幂等的：写入新值后下次启动不再命中旧默认。
fn migrate_default_hotkeys() {
    const MIGRATIONS: [(&str, &str, &str); 3] = [
        // (配置名, 旧默认, 新默认)
        ("hotkey_wave_scan", "Alt+V", "Alt+X"),
        ("hotkey_simon", "Alt+X", "Alt+C"),
        ("hotkey_ocr_recognize", "Alt+C", "Alt+V"),
    ];
    for (name, old, new) in MIGRATIONS {
        let stored = get(name).and_then(|v| v.as_str().map(|s| s.to_string()));
        if stored.as_deref() == Some(old) {
            set(name, new);
            info!("Hotkey default migrated: {} {} -> {}", name, old, new);
        }
    }
}

fn register<F>(app_handle: &AppHandle, name: &str, handler: F, key: &str) -> Result<(), String>
where
    F: Fn() + Send + 'static,
{
    let hotkey = {
        if key.is_empty() {
            match get(name) {
                // 配置文件可被用户手改成非字符串（数字/布尔等）：as_str 失败时
                // 回退默认键位，不 panic
                Some(v) => v
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| default_key(name).to_string()),
                None => {
                    // 未配置过热键时写入默认值（Windows 全局热键不区分左右修饰键）
                    let def = default_key(name);
                    set(name, def);
                    def.to_string()
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
    // 与 update_shortcut / 前端注册操作同一批全局热键：注册前先取互斥锁
    let _io = HOTKEY_IO.lock().unwrap_or_else(|e| e.into_inner());
    if shortcut == "all" {
        // 默认键位轮换的一次性迁移，必须在读取配置注册之前执行
        migrate_default_hotkeys();
    }
    match shortcut {
        "hotkey_ocr_recognize" => register(app_handle, "hotkey_ocr_recognize", rune_scan, "")?,
        "hotkey_wave_scan" => register(app_handle, "hotkey_wave_scan", wave_scan, "")?,
        "hotkey_simon" => register(app_handle, "hotkey_simon", crate::simon::simon_toggle, "")?,
        "all" => {
            // 逐个注册并收集失败项：任一热键被占用（与其他程序冲突）不得用 `?`
            // 短路导致后续热键全部不注册。全部失败才返回 Err；部分失败时告警
            // 列出失败的热键名+原因，返回 Ok 保证其余热键可用
            let mut failures: Vec<String> = Vec::new();
            if let Err(e) = register(app_handle, "hotkey_ocr_recognize", rune_scan, "") {
                failures.push(format!("hotkey_ocr_recognize: {}", e));
            }
            if let Err(e) = register(app_handle, "hotkey_wave_scan", wave_scan, "") {
                failures.push(format!("hotkey_wave_scan: {}", e));
            }
            if let Err(e) = register(app_handle, "hotkey_simon", crate::simon::simon_toggle, "") {
                failures.push(format!("hotkey_simon: {}", e));
            }
            if failures.len() == 3 {
                return Err(failures.join("; "));
            }
            if !failures.is_empty() {
                warn!("部分热键注册失败（其余热键已生效）: {}", failures.join("; "));
            }
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub fn register_shortcut_by_frontend(name: &str, shortcut: &str) -> Result<(), String> {
    let app_handle = APP.get().unwrap();
    // 与启动注册 / update_shortcut 操作同一批全局热键，同样需互斥
    let _io = HOTKEY_IO.lock().unwrap_or_else(|e| e.into_inner());
    match name {
        "hotkey_ocr_recognize" => {
            register(app_handle, "hotkey_ocr_recognize", rune_scan, shortcut)?
        }
        "hotkey_wave_scan" => register(app_handle, "hotkey_wave_scan", wave_scan, shortcut)?,
        "hotkey_simon" => register(app_handle, "hotkey_simon", crate::simon::simon_toggle, shortcut)?,
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
    // unregister→register（含失败回滚重注）非原子：包进 HOTKEY_IO 与启动/
    // 前端注册路径互斥，防止并发交错导致热键丢失
    let _io = HOTKEY_IO.lock().unwrap_or_else(|e| e.into_inner());
    if !old.is_empty() {
        let _ = manager.unregister(&old);
    }
    let handler = handler_for(name);
    match manager.register(&new_key, handler) {
        Ok(()) => {
            set(name, new_key.clone());
            info!("Hotkey updated: {} -> {} for {}", old, new_key, name);
            Ok(())
        }
        Err(e) => {
            // 回滚：重新注册旧键，保证本会话热键不丢失
            if !old.is_empty() {
                if let Err(re) = manager.register(&old, handler) {
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
