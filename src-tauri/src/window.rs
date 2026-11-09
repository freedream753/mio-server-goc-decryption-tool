use crate::config::{get, set};
use crate::APP;
use log::{info, warn};
use std::sync::Mutex;
use tauri::Manager;
use tauri::Monitor;
use tauri::Window;
use tauri::WindowBuilder;
use window_shadows::set_shadow;

// 结果窗口自动隐藏截止时刻：WebView2 对非激活窗口节流 timer，前端倒计时可能停摆，
// 由 Rust 侧兜底到期隐藏
static RESULT_DEADLINE: Mutex<Option<std::time::Instant>> = Mutex::new(None);

// 结果窗口几何（逻辑尺寸）：符文结果界面需要放下 4 个符文格 + 颜色 + 按钮
const RESULT_WINDOW_W: f64 = 420.0;
const RESULT_WINDOW_H: f64 = 360.0;

#[tauri::command]
pub fn rune_window() {
    let (window, exists) = build_window("runes", "GOC · 四位符文解密");
    if !exists {
        let _ = window.set_size(tauri::LogicalSize::new(660.0, 720.0));
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(580.0, 480.0)));
        let _ = window.center();
        let _ = window.set_always_on_top(true);
        let keep = window.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 隐藏而不销毁，保留本局状态。
                api.prevent_close();
                let _ = keep.hide();
            }
        });
    }
    let _ = window.show();
    let _ = window.set_focus();
}

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
            // 最小化状态下 set_focus 不生效（Windows 不还原最小化窗口），
            // 先还原再聚焦，否则托盘/HTTP 再次打开时表现为「点了没反应」
            if matches!(v.is_minimized(), Ok(true)) {
                let _ = v.unminimize();
            }
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

// 全屏截图框选覆盖窗（手动框选降级路径）：展示全屏截图，用户拖拽选出识别区域
fn screenshot_window() -> Window {
    let (window, exists) = build_window("screenshot", "Screenshot");

    // 框选完成后由截图窗口前端 emit("success") 触发识别。
    // tauri v1 的 Window::listen 只能收到 window 标签与自身 label 相同的 trigger
    // （JS 全局 emit 会以「发起窗口的 label」触发 Rust 监听器），因此监听必须挂在
    // 截图窗口自己身上；窗口每次新建时重新注册，复用时沿用已有监听，避免监听丢失。
    // 选区图交给符文识别管线的框选降级入口
    if !exists {
        let _ = window.listen("success", |_| crate::runes::rune_scan_manual());
    }

    let _ = window.set_skip_taskbar(true);
    let _ = window.set_fullscreen(true);

    let _ = window.set_always_on_top(true);
    window
}

// 手动框选（框选交互沿用 v1.0.1）：拉起全屏框选窗 -> 用户选区 -> 选区图进符文识别
// 管线。供自动定位失败（高 DPI 缩放、面板被遮挡等）时由用户自由选择截图区域
pub fn ocr_recognize() {
    let window = screenshot_window();
    // 截图窗口会被复用：其前端 useEffect 只在挂载时执行一次，复用旧窗口不会重新截屏，
    // 导致第二次 OCR 截屏一直显示旧图/无图。这里每次触发都显式通知前端重新截屏并显示；
    // 新建窗口时前端会在挂载阶段自行截屏（事件可能早于前端监听注册，丢失也无妨）。
    if let Err(e) = window.emit("screenshot_request", "") {
        warn!("Emit screenshot_request failed: {}", e);
    }
}

// ---------------- 符文识别结果窗口 ----------------
// 始终置顶 + WS_EX_NOACTIVATE（不抢焦点，不打断游戏按键交互），
// 数据由 Rust 推送（emit rune_scan），展示/纠正/倒计时由前端负责。

// 结果窗口 Win32 扩展样式：NOACTIVATE + TOPMOST
#[cfg(target_os = "windows")]
fn apply_result_ex_style(window: &Window) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
    };
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let h = HWND(hwnd.0 as *mut _);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            // LONG_PTR 宽度随架构变化（x64=isize、x86=i32），按指针宽度显式转换
            #[cfg(target_pointer_width = "64")]
            let flags = (WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0) as isize;
            #[cfg(target_pointer_width = "32")]
            let flags = (WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0) as i32;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | flags);
        }
    }
}

// 在鼠标附近定位结果窗口（物理像素，绕开 tao v1 隐藏窗口缩放 bug），返回 (x, y, w, h)
#[cfg(target_os = "windows")]
fn result_window_geometry() -> (i32, i32, i32, i32) {
    let monitor = mouse_monitor();
    let scale = monitor.scale_factor().max(0.1);
    let pw = (RESULT_WINDOW_W * scale).round() as i32;
    let ph = (RESULT_WINDOW_H * scale).round() as i32;
    let (mx, my) = {
        use mouse_position::mouse_position::Mouse;
        match Mouse::get_mouse_position() {
            Mouse::Position { x, y } => (x, y),
            Mouse::Error => (0, 0),
        }
    };
    let mp = monitor.position();
    let ms = monitor.size();
    let x = (mx + 24).min(mp.x + ms.width as i32 - pw - 8).max(mp.x + 8);
    let y = (my + 24).min(mp.y + ms.height as i32 - ph - 8).max(mp.y + 8);
    (x, y, pw, ph)
}

// 创建（或复用）结果窗口并推送新结果。调用方须在主线程。
pub fn result_window_notify(payload: serde_json::Value) {
    let app = APP.get().unwrap();
    let window = match app.get_window("result") {
        Some(v) => v,
        None => {
            info!("Creating result window");
            let builder = tauri::WindowBuilder::new(
                app,
                "result",
                tauri::WindowUrl::App("index.html".into()),
            )
            .title("GOC Result")
            .visible(false)
            .transparent(true)
            .decorations(false)
            .focused(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false);
            let window = match builder.build() {
                Ok(v) => v,
                Err(e) => {
                    warn!("Failed to create result window: {}", e);
                    return;
                }
            };
            let _ = window.current_monitor();
            #[cfg(target_os = "windows")]
            apply_result_ex_style(&window);
            // 拖动判定不基于 Moved 事件（拖动中途停住会被误判结束），
            // 而是前端按下拖拽区调用 begin_result_drag，回收线程轮询左键状态
            ensure_result_reaper(window.clone());
            window
        }
    };
    if let Err(e) = window.emit("rune_scan", payload) {
        warn!("Emit rune_scan failed: {}", e);
    }
}

// 显示结果窗口：鼠标附近重新定位 + SW_SHOWNA 式置顶（绝不抢焦点），复用 Win32 单调用
pub fn show_result_window() {
    if let Some(app) = APP.get() {
        if let Some(window) = app.get_window("result") {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
                };
                if let Ok(hwnd) = window.hwnd() {
                    let (x, y, w, h) = result_window_geometry();
                    unsafe {
                        let _ = SetWindowPos(
                            HWND(hwnd.0 as *mut _),
                            HWND_TOPMOST,
                            x,
                            y,
                            w,
                            h,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = window.show();
            }
            touch_result_deadline();
        }
    }
}

// 手动位移输入框需要键盘焦点：NOACTIVATE 窗口鼠标点击不会激活本窗口，
// 由前端在输入框聚焦时调用（程序化激活不受 NOACTIVATE 限制，仅在用户主动点输入框时发生）
pub fn focus_result_window() {
    if let Some(app) = APP.get() {
        if let Some(window) = app.get_window("result") {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
                if let Ok(hwnd) = window.hwnd() {
                    unsafe {
                        let _ = SetForegroundWindow(HWND(hwnd.0 as *mut _));
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = window.set_focus();
            }
        }
    }
}

// Rust 侧兜底：到期后由常驻回收线程隐藏结果窗口（前端 timer 被节流时靠它保证消失）。
// 拖动窗口时 Win32 处于模态循环，Moved 事件高频到达，此处只刷新截止时刻，
// 不为每个事件单独起线程。
fn touch_result_deadline() {
    *RESULT_DEADLINE.lock().unwrap() =
        Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
}

// 拖动状态：前端按下拖拽区置 true（begin_result_drag），回收线程检测到左键松开后结束
static RESULT_DRAG_HELD: Mutex<bool> = Mutex::new(false);

// 常驻回收线程：每 500ms 检查倒计时是否到期，到期直接 ShowWindow(SW_HIDE)。
// 窗口经 Win32 SetWindowPos(SWP_SHOWWINDOW) 显示，tao 内部可见状态仍是 false，
// window.hide() 会被当作 no-op 跳过，必须直接 ShowWindow(SW_HIDE)。
// 拖动中（左键按住）：倒计时保持 10s 不动；松开后等 2s 再开始 10s 倒计时。
fn ensure_result_reaper(window: Window) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        // 拖动中：以左键物理状态为准（不看是否移动，停住也不误判）
        if *RESULT_DRAG_HELD.lock().unwrap() {
            let held = {
                #[cfg(target_os = "windows")]
                unsafe {
                    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
                    (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0
                }
                #[cfg(not(target_os = "windows"))]
                { false }
            };
            if held {
                continue;
            }
            // 刚松开：等 2s 后开始 10s 倒计时（deadline = now + 12s），通知前端恢复
            *RESULT_DRAG_HELD.lock().unwrap() = false;
            *RESULT_DEADLINE.lock().unwrap() =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(12));
            let _ = window.emit("result-drag-end", ());
            info!("[window] 拖动结束，2s 后重新计 10s");
            continue;
        }
        let expired = {
            let d = RESULT_DEADLINE.lock().unwrap();
            matches!(*d, Some(t) if t <= std::time::Instant::now())
        };
        if !expired {
            continue;
        }
        *RESULT_DEADLINE.lock().unwrap() = None;
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
            if let Ok(hwnd) = window.hwnd() {
                let _ = ShowWindow(HWND(hwnd.0 as *mut _), SW_HIDE);
            }
        }
        #[cfg(not(target_os = "windows"))]
        let _ = window.hide();
        info!("[window] 结果窗口到期自动隐藏");
    });
}

// 前端交互（复制/拖动窗口）重置倒计时时同步推迟兜底隐藏
pub fn reset_result_timer() {
    touch_result_deadline();
}

// 前端按下拖拽区调用：进入拖动状态，倒计时重置为 10s 并保持不动（回收线程以左键状态判定结束）
pub fn begin_result_drag() {
    *RESULT_DRAG_HELD.lock().unwrap() = true;
    touch_result_deadline();
}

// 前端关闭按钮调用：立即隐藏（tao 可见状态为 false 时 window.hide() 是 no-op，必须 SW_HIDE）
pub fn hide_result_window() {
    *RESULT_DRAG_HELD.lock().unwrap() = false;
    *RESULT_DEADLINE.lock().unwrap() = None;
    if let Some(app) = APP.get() {
        if let Some(window) = app.get_window("result") {
            #[cfg(target_os = "windows")]
            unsafe {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                if let Ok(hwnd) = window.hwnd() {
                    let _ = ShowWindow(HWND(hwnd.0 as *mut _), SW_HIDE);
                }
            }
            #[cfg(not(target_os = "windows"))]
            let _ = window.hide();
            info!("[window] 结果窗口手动关闭");
        }
    }
}


