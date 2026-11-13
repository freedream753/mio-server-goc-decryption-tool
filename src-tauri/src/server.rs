use crate::config::{get, set};
use crate::window::*;
use crate::APP;
use log::{info, warn};
use std::thread;
use tauri::api::notification;
use tiny_http::{Request, Response, Server};

pub fn start_server() {
    // 防御性解析：类型异常时回退默认端口，避免启动 unwrap panic
    let port = match get("server_port").and_then(|v| v.as_i64()) {
        Some(v) => v,
        None => {
            set("server_port", 60828);
            60828
        }
    };
    // 与 main.rs 一致动态取 bundle identifier，避免硬编码漂移
    let identifier = APP
        .get()
        .map(|a| a.config().tauri.bundle.identifier.clone())
        .unwrap_or_default();
    thread::spawn(move || {
        let server = match Server::http(format!("127.0.0.1:{port}")) {
            Ok(v) => v,
            Err(e) => {
                let _ = notification::Notification::new(identifier)
                    .title("Server start failed")
                    .body("Please Change Server Port and restart the application")
                    .show();
                warn!("Server start failed: {}", e);
                return;
            }
        };
        for request in server.incoming_requests() {
            http_handle(request);
        }
    });
}

fn http_handle(request: Request) {
    // 入口来源校验：服务只服务于本机脚本/托盘触发，任意网页可用
    // <img src="http://127.0.0.1:60828/"> 或 fetch no-cors 跨站触发截屏+弹窗+
    // 写剪贴板，且 Host 不校验存在 DNS rebinding 面。命中拒绝条件直接回 403，
    // 不执行任何管线；curl/脚本等非浏览器工具不带浏览器指纹头 → 正常放行。
    if let Some(reason) = reject_reason(&request) {
        warn!("HTTP request rejected: {}", reason);
        let response = Response::from_string("forbidden").with_status_code(403);
        if let Err(e) = request.respond(response) {
            warn!("HTTP respond failed: {}", e);
        }
        return;
    }
    info!("Handle {} request", request.url());
    match request.url() {
        // 主入口：截图 → 识别符文 → 结果窗口（失败在结果窗口报错）
        "/" | "/ocr_recognize" | "/ocr_recognize?screenshot=true" => handle_rune_scan(request),
        // 手动框选入口：对已有框选选区图（pot_screenshot_cut.png）跑同一符文管线
        "/solver_manual" => handle_rune_scan_manual(request),
        "/config" => handle_config(request),
        _ => warn!("Unknown request url: {}", request.url()),
    }
}

// 提取请求头中的 Host / Sec-Fetch-Site / Origin（字段名大小写不敏感），
// 按下列规则判定拒绝原因（None = 放行）：
// ① Host 存在但不以 127.0.0.1 / localhost 开头 → 防 DNS rebinding；
// ② Sec-Fetch-Site 存在且不是 none / same-origin → Chromium/WebView2 发起
//    跨站请求必带此头，非浏览器工具（curl/脚本）不带 → 放行；
// ③ Origin 存在且不以 http://127.0.0.1 / http://localhost / tauri:// 开头。
fn reject_reason(request: &Request) -> Option<String> {
    let mut host: Option<&str> = None;
    let mut site: Option<&str> = None;
    let mut origin: Option<&str> = None;
    for h in request.headers().iter() {
        let value = h.value.as_str();
        if h.field.equiv("Host") {
            host = Some(value);
        } else if h.field.equiv("Sec-Fetch-Site") {
            site = Some(value);
        } else if h.field.equiv("Origin") {
            origin = Some(value);
        }
    }
    if let Some(host) = host {
        if !host.starts_with("127.0.0.1") && !host.starts_with("localhost") {
            return Some(format!("非法 Host: {}", host));
        }
    }
    if let Some(site) = site {
        if site != "none" && site != "same-origin" {
            return Some(format!("跨站请求 Sec-Fetch-Site: {}", site));
        }
    }
    if let Some(origin) = origin {
        if !origin.starts_with("http://127.0.0.1")
            && !origin.starts_with("http://localhost")
            && !origin.starts_with("tauri://")
        {
            return Some(format!("非法 Origin: {}", origin));
        }
    }
    None
}

fn handle_config(request: Request) {
    config_window();
    response_ok(request);
}

fn handle_rune_scan(request: Request) {
    crate::runes::rune_scan();
    response_ok(request);
}

fn handle_rune_scan_manual(request: Request) {
    crate::runes::rune_scan_manual();
    response_ok(request);
}

fn response_ok(request: Request) {
    let response = Response::from_string("ok");
    // 客户端提前断开时 respond 返回 Err，unwrap 会杀死 server 线程导致 HTTP 触发静默失效
    if let Err(e) = request.respond(response) {
        warn!("HTTP respond failed: {}", e);
    }
}
