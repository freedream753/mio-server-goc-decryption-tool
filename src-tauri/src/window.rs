use crate::config::{get, set};
use crate::APP;
use log::{info, warn};
use tauri::Manager;
use tauri::Monitor;
use tauri::Window;
use tauri::WindowBuilder;
use window_shadows::set_shadow;

// Get daemon window instance
fn get_daemon_window() -> Window {
    let app_handle = APP.get().unwrap();
    match app_handle.get_window("daemon") {
        Some(v) => v,
        None => {
            warn!("Daemon window not found, create new daemon window!");
            WindowBuilder::new(
                app_handle,
                "daemon",
                tauri::WindowUrl::App("daemon.html".into()),
            )
            .title("Daemon")
            .visible(false)
            .build()
            .unwrap()
        }
    }
}

// Get monitor where the mouse is currently located
fn get_current_monitor(x: i32, y: i32) -> Monitor {
    info!("Mouse position: {}, {}", x, y);
    let daemon_window = get_daemon_window();
    let monitors = daemon_window.available_monitors().unwrap_or_default();

    for m in monitors {
        let size = m.size();
        let position = m.position();

        if x >= position.x
            && x <= (position.x + size.width as i32)
            && y >= position.y
            && y <= (position.y + size.height as i32)
        {
            info!("Current Monitor: {:?}", m);
            return m;
        }
    }
    warn!("Current Monitor not found, using primary monitor");
    // 双 unwrap 改为单点兜底：仅在完全无显示器时 panic（此时应用本无法使用）
    daemon_window
        .primary_monitor()
        .ok()
        .flatten()
        .expect("No monitor available")
}

// 鼠标所在显示器。隐藏/刚创建的窗口 current_monitor() 可能返空，
// 而 daemon 窗口的 available_monitors() 实测稳定，故统一走这条路径
fn mouse_monitor() -> Monitor {
    use mouse_position::mouse_position::{Mouse, Position};

    let mouse_position = match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => Position { x, y },
        Mouse::Error => {
            warn!("Mouse position not found, using (0, 0) as default");
            Position { x: 0, y: 0 }
        }
    };
    get_current_monitor(mouse_position.x, mouse_position.y)
}

// Creating a window on the mouse monitor
fn build_window(label: &str, title: &str) -> (Window, bool) {
    let monitor = mouse_monitor();
    let position = monitor.position();

    let app_handle = APP.get().unwrap();
    match app_handle.get_window(label) {
        Some(v) => {
            info!("Window existence: {}", label);
            let _ = v.set_focus();
            (v, true)
        }
        None => {
            info!("Window not existence, Creating new window: {}", label);
            let builder = tauri::WindowBuilder::new(
                app_handle,
                label,
                tauri::WindowUrl::App("index.html".into()),
            )
            .position(position.x.into(), position.y.into())
            .focused(true)
            .title(title)
            .visible(false)
            .transparent(true)
            .decorations(false);
            // 不要给单个窗口设置 additional_browser_args：wry/WebView2 的浏览器参数是
            // 进程级（绑定 user data folder）的，只要有一个窗口用了不同参数，
            // 后续窗口的 CreateCoreWebView2EnvironmentWithOptions 就会返回
            // ERROR_INVALID_STATE，导致新建窗口直接失败（此前 recognize 窗口创建即 panic）。

            // 窗口创建失败属致命错误，panic 但携带上下文便于定位
            let window = builder.build().unwrap_or_else(|e| {
                panic!("Failed to create window {label}: {e}");
            });

            if label != "screenshot" {
                set_shadow(&window, true).unwrap_or_default();
            }
            let _ = window.current_monitor();
            (window, false)
        }
    }
}

// tao v1 隐藏创建的窗口 scale_factor 恒为 1.0（即使显示在 150% 显示器上），
// 因此一律改用显示器自身的缩放系数
fn monitor_scale(window: &tauri::Window) -> f64 {
    match window.current_monitor() {
        Ok(Some(m)) if m.scale_factor() >= 0.1 => m.scale_factor(),
        _ => {
            // 隐藏窗口 current_monitor() 可能返空，回退到鼠标所在显示器，
            // 避免用 1.0 保存物理尺寸污染 "config_window_size"
            let s = mouse_monitor().scale_factor();
            if s >= 0.1 {
                s
            } else {
                1.0
            }
        }
    }
}

// 保存偏好设置窗口状态（最大化标记 + 逻辑尺寸）；隐藏阶段不保存，避免污染
fn save_config_window_state(window: &tauri::Window) {
    if !matches!(window.is_visible(), Ok(true)) {
        return;
    }
    match window.is_maximized() {
        Ok(true) => set("config_window_maximized", true),
        _ => {
            set("config_window_maximized", false);
            if let Ok(size) = window.outer_size() {
                let scale = monitor_scale(window);
                let w = (size.width as f64 / scale * 10.0).round() / 10.0;
                let h = (size.height as f64 / scale * 10.0).round() / 10.0;
                set("config_window_size", vec![w, h]);
            }
        }
    }
}

pub fn config_window() {
    let (window, exists) = build_window("config", "GOC解题妙妙小工具");
    if exists {
        return;
    }
    // tao v1 隐藏窗口的尺寸换算存在缩放 bug（内部 scale 恒为 1.0，150% 屏上
    // 窗口会直接缩水 1/3），因此统一换算成物理像素，Windows 上直接走 Win32 设置
    let monitor = mouse_monitor();
    let mon_scale = monitor.scale_factor().max(0.1);
    let mon_w = monitor.size().width as f64;
    let mon_h = monitor.size().height as f64;
    let avail_w = if mon_w > 0.0 { mon_w / mon_scale - 40.0 } else { f64::MAX };
    let avail_h = if mon_h > 0.0 { mon_h / mon_scale - 80.0 } else { f64::MAX };
    // 默认 1200x900 以容纳完整的通用设置页，超小屏自动收缩，之后记住用户调整
    let def_w = 1200.0f64.min(avail_w).max(800.0);
    let def_h = 900.0f64.min(avail_h).max(500.0);
    let maximized = get("config_window_maximized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let (w, h) = get("config_window_size")
        .and_then(|v| serde_json::from_value::<Vec<f64>>(v).ok())
        .filter(|s| s.len() == 2 && s[0] >= 400.0 && s[1] >= 300.0)
        .map(|s| (s[0].min(avail_w), s[1].min(avail_h)))
        .unwrap_or((def_w, def_h));
    let phys_w = (w * mon_scale).round() as i32;
    let phys_h = (h * mon_scale).round() as i32;
    info!(
        "Config window geometry: logical {}x{}, physical {}x{}, scale {}, maximized {}",
        w, h, phys_w, phys_h, mon_scale, maximized
    );
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(800.0, 400.0)));
    if maximized {
        let _ = window.maximize();
    } else {
        #[cfg(target_os = "windows")]
        {
            // tao v1 对隐藏窗口的 set_size 换算存在缩放 bug，直接用
            // Win32 SetWindowPos 按原始物理像素设置尺寸并居中，绕开换算
            let mut sized = false;
            match window.hwnd() {
                Ok(hwnd) => {
                    let mp = monitor.position();
                    let ms = monitor.size();
                    let px = mp.x + ((ms.width as i32 - phys_w) / 2).max(0);
                    let py = mp.y + ((ms.height as i32 - phys_h) / 2).max(0);
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                            windows::Win32::Foundation::HWND(hwnd.0 as *mut _),
                            windows::Win32::Foundation::HWND::default(),
                            px,
                            py,
                            phys_w,
                            phys_h,
                            windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
                        );
                    }
                    sized = true;
                }
                Err(e) => warn!("Get hwnd failed, fallback to tao set_size: {}", e),
            }
            if !sized {
                let _ = window.set_size(tauri::LogicalSize::new(w, h));
                let _ = window.center();
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window.set_size(tauri::LogicalSize::new(w, h));
            let _ = window.center();
        }
    }
    // 三重保存：拖拽调整防抖 600ms 落盘，失焦/关闭立即保存；隐藏与最大化阶段不保存
    let window_ = window.clone();
    let (tx, rx) = std::sync::mpsc::channel::<(f64, f64)>();
    std::thread::spawn(move || {
        while let Ok(mut last) = rx.recv() {
            while let Ok(v) = rx.recv_timeout(std::time::Duration::from_millis(600)) {
                last = v;
            }
            set("config_window_size", vec![last.0, last.1]);
        }
    });
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(size) => {
            if matches!(window_.is_visible(), Ok(true)) && !matches!(window_.is_maximized(), Ok(true)) {
                let s = monitor_scale(&window_);
                let _ = tx.send((size.width as f64 / s, size.height as f64 / s));
            }
        }
        tauri::WindowEvent::CloseRequested { .. } => save_config_window_state(&window_),
        tauri::WindowEvent::Focused(false) => save_config_window_state(&window_),
        _ => {}
    });
}

// 隐藏的 OCR 解密 worker 窗口：不显示、不抢焦点，仅在前端执行 OCR->凯撒解密->剪贴板+通知
pub fn recognize_window() {
    let (window, exists) = build_window("recognize", "Recognize");
    if exists {
        let _ = window.emit("new_image", "");
        return;
    }
    let _ = window.emit("new_image", "");
}

fn screenshot_window() -> Window {
    let (window, exists) = build_window("screenshot", "Screenshot");

    // 框选完成后由截图窗口前端 emit("success") 触发 OCR。
    // tauri v1 的 Window::listen 只能收到 window 标签与自身 label 相同的 trigger
    // （JS 全局 emit 会以「发起窗口的 label」触发 Rust 监听器），因此监听必须挂在
    // 截图窗口自己身上；窗口每次新建时重新注册，复用时沿用已有监听，避免监听丢失。
    if !exists {
        let _ = window.listen("success", |_| recognize_window());
    }

    let _ = window.set_skip_taskbar(true);
    let _ = window.set_fullscreen(true);

    let _ = window.set_always_on_top(true);
    window
}

pub fn ocr_recognize() {
    let window = screenshot_window();
    // 截图窗口会被复用：其前端 useEffect 只在挂载时执行一次，复用旧窗口不会重新截屏，
    // 导致第二次 OCR 截屏一直显示旧图/无图。这里每次触发都显式通知前端重新截屏并显示；
    // 新建窗口时前端会在挂载阶段自行截屏（事件可能早于前端监听注册，丢失也无妨）。
    if let Err(e) = window.emit("screenshot_request", "") {
        warn!("Emit screenshot_request failed: {}", e);
    }
}


