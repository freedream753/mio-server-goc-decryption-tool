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
    info!("Handle {} request", request.url());
    match request.url() {
        "/" | "/ocr_recognize" | "/ocr_recognize?screenshot=true" => handle_ocr_recognize(request),
        "/ocr_recognize?screenshot=false" => handle_recognize_window(request),
        "/config" => handle_config(request),
        _ => warn!("Unknown request url: {}", request.url()),
    }
}

fn handle_config(request: Request) {
    config_window();
    response_ok(request);
}

fn handle_ocr_recognize(request: Request) {
    ocr_recognize();
    response_ok(request);
}

fn handle_recognize_window(request: Request) {
    recognize_window();
    response_ok(request);
}

fn response_ok(request: Request) {
    let response = Response::from_string("ok");
    // 客户端提前断开时 respond 返回 Err，unwrap 会杀死 server 线程导致 HTTP 触发静默失效
    if let Err(e) = request.respond(response) {
        warn!("HTTP respond failed: {}", e);
    }
}
