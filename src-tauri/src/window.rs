use crate::config::{get, set};
use crate::APP;
use log::{info, warn};
use std::sync::Mutex;
use tauri::Manager;
use tauri::Monitor;
use tauri::Window;
use tauri::WindowBuilder;
use window_shadows::set_shadow;

// 框选用途派发：截图窗口 success 事件按此分发给符文识别("rune")或波形框选("wave")
pub static SHOT_PURPOSE: Mutex<&'static str> = Mutex::new("rune");

// 框选流程设置互斥：crop/wave 两个入口共用 SHOT_PURPOSE 单值与同一截图窗口，
// 「设置用途 → 起线程拉框选窗」必须原子完成，并发入口会互相覆盖用途值导致
// success 派发到错误管线。入口用 try_lock 抢锁，抢不到说明已有流程在设置中
pub static PURPOSE_IO: Mutex<()> = Mutex::new(());

// 最近一次框选区域（物理像素），截图窗口 cut_image 命令写入、
// 波形框选（wave::on_region_selected）读取落定
pub static LAST_CUT: Mutex<Option<(u32, u32, u32, u32)>> = Mutex::new(None);

// ---------------- 全局裁切模式（v1.2.7） ----------------
// 框选后密码/波形/西蒙三条截图管线只在该区域内识别，不再采集全屏。
// CROP_REGION 为全局桌面物理像素 (x, y, w, h)；SHOT_ORIGIN 记录最近一次
// 框选截图所覆盖显示器的全局原点（cmd::screenshot 写入），用于把框选的
// 图内坐标换算成全局坐标。

pub static CROP_REGION: Mutex<Option<(i32, i32, u32, u32)>> = Mutex::new(None);
pub static SHOT_ORIGIN: Mutex<(i32, i32)> = Mutex::new((0, 0));

pub fn set_shot_origin(x: i32, y: i32) {
    *SHOT_ORIGIN.lock().unwrap() = (x, y);
}

pub fn crop_region() -> Option<(i32, i32, u32, u32)> {
    *CROP_REGION.lock().unwrap()
}

pub fn set_shot_purpose(p: &'static str) {
    *SHOT_PURPOSE.lock().unwrap() = p;
}

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

// 全屏截图框选覆盖窗（手动框选降级路径）：展示全屏截图，用户拖拽选出识别区域
fn screenshot_window() -> Window {
    let (window, exists) = build_window("screenshot", "Screenshot");

    // 框选完成后由截图窗口前端 emit("success") 触发识别。
    // tauri v1 的 Window::listen 只能收到 window 标签与自身 label 相同的 trigger
    // （JS 全局 emit 会以「发起窗口的 label」触发 Rust 监听器），因此监听必须挂在
    // 截图窗口自己身上；窗口每次新建时重新注册，复用时沿用已有监听，避免监听丢失。
    // 选区图按 SHOT_PURPOSE 派发：符文识别 / 波形画布框选 / 全局裁切
    if !exists {
        let _ = window.listen("success", |_| {
            match *crate::window::SHOT_PURPOSE.lock().unwrap() {
                "wave" => crate::wave::on_region_selected(),
                "crop" => crop_selected(),
                _ => crate::runes::rune_scan_manual(),
            }
        });
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

// ---------------- 全局裁切模式（v1.2.7） ----------------

// 裁切模式框选完成（SHOT_PURPOSE="crop" 派发）：图内选区 + 截图显示器原点
// → 全局物理像素区域 → 落定 CROP_REGION（含持久化）→ 浮现指示框 → 重显结果窗口
fn crop_selected() {
    let cut = match *LAST_CUT.lock().unwrap() {
        Some(v) => v,
        None => return,
    };
    let origin = *SHOT_ORIGIN.lock().unwrap();
    let region = (
        origin.0 + cut.0 as i32,
        origin.1 + cut.1 as i32,
        cut.2,
        cut.3,
    );
    set_crop_region(Some(region));
    info!(
        "[crop] 裁切模式生效: {:?}（全局物理像素），后续识别只采集框内画面",
        region
    );
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(move || {
            show_result_window();
            show_crop_overlay(region);
        });
    }
}

// 取消裁切模式：清空区域（含持久化）并隐藏指示框，恢复全屏采集
pub fn crop_clear() {
    set_crop_region(None);
    info!("[crop] 裁切模式已取消，恢复全屏识别");
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(hide_crop_overlay);
    }
}

// 启动时恢复上次的裁切模式（配置持久化）：指示框重新浮现，区域继续生效。
// 区域必须与现存显示器相交才恢复：拔掉副屏后旧区域可能悬在虚空，
// 直接恢复会让识别管线永远采集空区域
pub fn restore_crop_region() {
    let parsed = get("crop_region").and_then(|v| {
        v.as_array().map(|a| {
            a.iter()
                .filter_map(|x| x.as_f64())
                .collect::<Vec<f64>>()
        })
    });
    if let Some(a) = parsed {
        // 宽高须为正；x/y 允许负值——左侧/上方副屏的全局坐标为负，
        // crop_selected 落定的区域完全可能是负坐标（不能用「非负」判断）
        if a.len() == 4 && a[2] > 0.0 && a[3] > 0.0 {
            // 用 i64 求交防溢出（配置可被手改成超大数值）
            let (x, y, w, h) = (a[0] as i64, a[1] as i64, a[2] as i64, a[3] as i64);
            // APP 可用时枚举现存显示器（物理坐标原点+尺寸）校验相交：
            // 任一显示器满足 x < mx+mw && x+w > mx && y < my+mh && y+h > my 即可；
            // APP 不可用（早于 setup）时保持原行为不拦截
            let alive = match APP.get() {
                Some(app) => {
                    // tauri v1 的 available_monitors 在 Window 上（AppHandle 没有），
                    // 且 daemon 窗口的枚举实测最稳定（见 get_current_monitor 注释）
                    let daemon = app.get_window("daemon");
                    match daemon.map(|w| w.available_monitors()) {
                        Some(Ok(monitors)) => monitors.iter().any(|m| {
                            let (mx, my) = (m.position().x as i64, m.position().y as i64);
                            let (mw, mh) = (m.size().width as i64, m.size().height as i64);
                            x < mx + mw && x + w > mx && y < my + mh && y + h > my
                        }),
                        // 窗口不存在/枚举失败时不拦截，保持原恢复行为
                        _ => true,
                    }
                }
                None => true,
            };
            if alive {
                let region = (x as i32, y as i32, w as u32, h as u32);
                *CROP_REGION
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(region);
                info!("[crop] 已恢复裁切模式: {:?}", region);
                show_crop_overlay(region);
            } else {
                warn!(
                    "[crop] 已保存的裁切区域 {:?} 不在任何现存显示器内（对应副屏可能已拔出），丢弃",
                    (x, y, w, h)
                );
                // 清掉失效配置，避免每次启动重复告警
                set("crop_region", serde_json::Value::Null);
            }
        }
    }
}

fn set_crop_region(region: Option<(i32, i32, u32, u32)>) {
    *CROP_REGION.lock().unwrap() = region;
    match region {
        Some((x, y, w, h)) => set("crop_region", vec![x as f64, y as f64, w as f64, h as f64]),
        None => set("crop_region", serde_json::Value::Null),
    }
    // 通知结果窗口切换按钮态（框选 ⇄ 取消裁切模式）
    if let Some(app) = APP.get().cloned() {
        let _ = app.clone().run_on_main_thread(move || {
            if let Some(w) = app.get_window("result") {
                let _ = w.emit("crop_state", region.is_some());
            }
        });
    }
}

// 裁切模式指示框：罩在裁切区四周的置顶点击穿透窗（边框 + 「裁切模式」角标），
// 让用户随时知道识别只发生在框内。窗口比裁切区四周各外扩 OVERLAY_PAD，
// 留出边框与角标的位置；点击穿透由 set_ignore_cursor_events 保证。
const OVERLAY_PAD: i32 = 14;

fn show_crop_overlay(region: (i32, i32, u32, u32)) {
    let app = match APP.get() {
        Some(v) => v,
        None => return,
    };
    let (x, y, w, h) = region;
    let ow = w as i32 + OVERLAY_PAD * 2;
    let oh = h as i32 + OVERLAY_PAD * 2;
    let ox = x - OVERLAY_PAD;
    let oy = y - OVERLAY_PAD;
    let window = match app.get_window("crop") {
        Some(v) => v,
        None => {
            info!("Creating crop overlay window");
            let builder = tauri::WindowBuilder::new(
                app,
                "crop",
                tauri::WindowUrl::App("index.html".into()),
            )
            .title("GOC Crop")
            .visible(false)
            .transparent(true)
            .decorations(false)
            .focused(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false);
            match builder.build() {
                Ok(v) => v,
                Err(e) => {
                    warn!("Failed to create crop overlay: {}", e);
                    return;
                }
            }
        }
    };
    let _ = window.set_ignore_cursor_events(true);
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowDisplayAffinity, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE,
            SWP_SHOWWINDOW, WDA_EXCLUDEFROMCAPTURE,
        };
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                let h = HWND(hwnd.0 as *mut _);
                // 指示窗对屏幕捕获隐身（Win10 2004+）：橙色边框/角标是工具自绘 UI，
                // 若被截进画面，竖直橙线会让解密的彩色行带黏成巨带、西蒙的边缘
                // 掩膜多出假跳变沿——识别管线不该看到自己的指示器
                if let Err(e) = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE) {
                    warn!("SetWindowDisplayAffinity(crop) failed: {}", e);
                }
                let _ = SetWindowPos(
                    h,
                    HWND_TOPMOST,
                    ox,
                    oy,
                    ow,
                    oh,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window.set_size(tauri::PhysicalSize::new(ow.max(1) as u32, oh.max(1) as u32));
        let _ = window.set_position(tauri::PhysicalPosition::new(ox, oy));
        let _ = window.show();
    }
}

fn hide_crop_overlay() {
    if let Some(app) = APP.get() {
        if let Some(window) = app.get_window("crop") {
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
        }
    }
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
            apply_result_ex_style(&v);
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

// 创建（或复用）结果窗口并推送新结果。调用方须在主线程。
pub fn result_window_notify(payload: serde_json::Value) {
    if let Some(window) = get_or_create_result_window() {
        if let Err(e) = window.emit("rune_scan", payload) {
            warn!("Emit rune_scan failed: {}", e);
        }
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

// 框选截图前的临时隐藏：窗口不是被用户关闭，框选完成后要原位显示，
// 因此不重置 RESULT_PLACED、不清归属显示器（否则框选后窗口会重定向到鼠标）
pub fn hide_result_window_temp() {
    hide_result_window_inner(false);
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
                // 关闭即重置定位标志并清除归属显示器：下次按快捷键重新定向到鼠标附近。
                // 西蒙录制仅限西蒙窗口：窗口被关闭时自动暂停录制
                RESULT_PLACED.store(false, std::sync::atomic::Ordering::SeqCst);
                if let Ok(mut home) = RESULT_HOME_MONITOR.lock() {
                    *home = None;
                }
                if crate::simon::simon_running() {
                    crate::simon::simon_stop_impl();
                }
            }
            info!("[window] 结果窗口隐藏（reset_place={}）", reset_place);
        }
    }
}


