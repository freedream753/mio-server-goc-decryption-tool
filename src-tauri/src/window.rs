use crate::config::{get, set};
use crate::APP;
use log::{info, warn};
use std::sync::Mutex;
use tauri::Manager;
use tauri::Monitor;
use tauri::Window;
use tauri::WindowBuilder;
use window_shadows::set_shadow;

// 结果窗口默认几何（逻辑尺寸）= 识别页签尺寸；关卡页签由前端按页调用
// set_result_window_size 调整（v1.2.1 四关卡全部集成进结果窗口）
pub const RESULT_WINDOW_W: f64 = 420.0;
pub const RESULT_WINDOW_H: f64 = 360.0;

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

// ---------------- 符文识别结果窗口 ----------------
// 始终置顶 + WS_EX_NOACTIVATE（不抢焦点，不打断游戏按键交互），
// 数据由 Rust 推送（emit rune_scan），展示/纠正由前端负责（v1.2.0 起无自动倒计时，
// 结果窗口由用户手动关闭）。

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

// 获取（不存在则创建）结果窗口。调用方须在主线程
// （tauri v1 主线程外建 WebView2 窗口会重入卡死）
pub fn get_or_create_result_window() -> Option<Window> {
    let app = APP.get()?;
    if let Some(v) = app.get_window("result") {
        return Some(v);
    }
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
    match builder.build() {
        Ok(v) => {
            let _ = v.current_monitor();
            #[cfg(target_os = "windows")]
            {
                apply_result_ex_style(&v);
                // 结果窗口对屏幕捕获隐身（Win10 2004+）：
                // 窗口常被玩家摆在游戏画面上方，工具自有常显窗口不应进入
                // 外部采集/录屏画面
                {
                    use windows::Win32::Foundation::HWND;
                    use windows::Win32::UI::WindowsAndMessaging::{
                        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
                    };
                    if let Ok(hwnd) = v.hwnd() {
                        unsafe {
                            if let Err(e) = SetWindowDisplayAffinity(
                                HWND(hwnd.0 as *mut _),
                                WDA_EXCLUDEFROMCAPTURE,
                            ) {
                                warn!("SetWindowDisplayAffinity(result) failed: {}", e);
                            }
                        }
                    }
                }
            }
            // 拖动限制：窗口移动事件里把位置钳回归属显示器内（禁止跨屏拖动）。
            // 钳回后的下一次 Moved 事件坐标已合规，不会再 set_position，无递归
            let v2 = v.clone();
            v.on_window_event(move |event| {
                if let tauri::WindowEvent::Moved(pos) = event {
                    let home = *RESULT_HOME_MONITOR.lock().unwrap();
                    if let Some((mx, my, mw, mh)) = home {
                        let w = v2.outer_size().map(|s| s.width as i32).unwrap_or(0);
                        let h = v2.outer_size().map(|s| s.height as i32).unwrap_or(0);
                        // .max(mx)/.max(my)：窗口比屏幕还宽/高时钳到屏幕左上角，避免负区间
                        let x = pos.x.max(mx).min((mx + mw - w).max(mx));
                        let y = pos.y.max(my).min((my + mh - h).max(my));
                        if x != pos.x || y != pos.y {
                            let _ = v2.set_position(tauri::PhysicalPosition::new(x, y));
                        }
                    }
                }
            });
            Some(v)
        }
        Err(e) => {
            warn!("Failed to create result window: {}", e);
            None
        }
    }
}

// 结果窗口是否已完成首次鼠标定位：首次显示（或上次显示后又被手动关闭）时
// 重定向到鼠标附近；窗口持续可见期间再触发热键一律原位显示（NOMOVE|NOSIZE
// 仅抬回置顶），用户拖动后的位置得以保留
static RESULT_PLACED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// 结果窗口的「归属显示器」（快捷键呼出时鼠标所在屏的物理矩形 x,y,w,h）：
// 窗口只允许在该屏内拖动，跨屏需求 = 关闭窗口后到另一块屏再按快捷键
static RESULT_HOME_MONITOR: std::sync::Mutex<Option<(i32, i32, i32, i32)>> =
    std::sync::Mutex::new(None);

// 记录归属显示器（取鼠标所在屏的物理矩形）
fn set_result_home_monitor() {
    let m = mouse_monitor();
    let mp = m.position();
    let ms = m.size();
    if let Ok(mut home) = RESULT_HOME_MONITOR.lock() {
        *home = Some((mp.x, mp.y, ms.width as i32, ms.height as i32));
    }
}

// 显示结果窗口：首次显示重定向到鼠标附近（SW_SHOWNA 式置顶，绝不抢焦点），
// 之后一律原位显示（NOMOVE|NOSIZE 仅抬回置顶）——热键反复触发不跳位
pub fn show_result_window() {
    if let Some(app) = APP.get() {
        if let Some(window) = app.get_window("result") {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                    SWP_SHOWWINDOW,
                };
                if let Ok(hwnd) = window.hwnd() {
                    // 首次：定位到鼠标附近并按默认尺寸落位；之后：原位原尺寸抬顶
                    let first = !RESULT_PLACED.load(std::sync::atomic::Ordering::SeqCst);
                    let (flags, x, y, w, h) = if first {
                        let (x, y, w, h) = result_window_geometry();
                        (SWP_NOACTIVATE | SWP_SHOWWINDOW, x, y, w, h)
                    } else {
                        (
                            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
                            0,
                            0,
                            0,
                            0,
                        )
                    };
                    unsafe {
                        let _ = SetWindowPos(
                            HWND(hwnd.0 as *mut _),
                            HWND_TOPMOST,
                            x,
                            y,
                            w,
                            h,
                            flags,
                        );
                    }
                    if first {
                        // 记录归属显示器：本次会话内拖动被限制在该屏
                        set_result_home_monitor();
                        RESULT_PLACED.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                set_result_home_monitor();
                let _ = window.show();
                RESULT_PLACED.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

// 按页签调整结果窗口尺寸（逻辑像素）：保持位置不变。
// 绕开 tao v1 对隐藏窗口 set_size 的缩放 bug，直接按显示器缩放系数换算物理像素走 Win32
pub fn set_result_window_size(w: f64, h: f64) {
    if let Some(app) = APP.get() {
        if let Some(window) = app.get_window("result") {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
                };
                let scale = monitor_scale(&window);
                let pw = (w * scale).round() as i32;
                let mut ph = (h * scale).round() as i32;
                // 高度按所在显示器钳制（留 16px 边距）：内容超长时窗口不得
                // 撑出屏幕，超出部分由前端页内滚动消化
                if let Ok(Some(m)) = window.current_monitor() {
                    let max_ph = (m.size().height as i32 - 16).max(64);
                    if ph > max_ph {
                        ph = max_ph;
                    }
                }
                if let Ok(hwnd) = window.hwnd() {
                    unsafe {
                        let _ = SetWindowPos(
                            HWND(hwnd.0 as *mut _),
                            HWND::default(),
                            0,
                            0,
                            pw,
                            ph,
                            SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
                        );
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            let _ = window.set_size(tauri::LogicalSize::new(w, h));
        }
    }
}

// 前端关闭按钮调用：立即隐藏结果窗口
// （tao 可见状态为 false 时 window.hide() 是 no-op，必须 SW_HIDE）
pub fn hide_result_window() {
    // 用户主动关闭：重置定位标志 + 清归属显示器（下次按快捷键重新定向到鼠标）
    hide_result_window_inner(true);
}

fn hide_result_window_inner(reset_place: bool) {
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
            if reset_place {
                // 关闭即重置定位标志并清除归属显示器：下次按快捷键重新定向到鼠标附近
                RESULT_PLACED.store(false, std::sync::atomic::Ordering::SeqCst);
                if let Ok(mut home) = RESULT_HOME_MONITOR.lock() {
                    *home = None;
                }
            }
            info!("[window] 结果窗口隐藏（reset_place={}）", reset_place);
        }
    }
}

// 待切页签（open_result_tab 写入，前端 get_open_tab 读即清）
static PENDING_TAB: Mutex<Option<&'static str>> = Mutex::new(None);

// 热键/托盘/HTTP 通用入口：呼出结果窗口并切到指定页签。
// 铁律：绝不能在热键回调/命令上下文同步创建 WebView2 窗口（会卡死协议管线），
// 必须 thread::spawn 后经 run_on_main_thread 在主线程建窗（同旧 wave.rs notify_result 模式）
pub fn open_result_tab(tab: &'static str) {
    *PENDING_TAB.lock().unwrap_or_else(|e| e.into_inner()) = Some(tab);
    std::thread::spawn(move || {
        if let Some(app) = crate::APP.get() {
            let _ = app.run_on_main_thread(move || {
                get_or_create_result_window();
                show_result_window();
                if let Some(app) = crate::APP.get() {
                    if let Some(w) = app.get_window("result") {
                        let _ = w.emit("open_tab", tab);
                    }
                }
            });
        }
    });
}

// 读取并清空待切页签（读即清防竞态）
pub fn take_pending_tab() -> Option<String> {
    PENDING_TAB
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .map(String::from)
}


