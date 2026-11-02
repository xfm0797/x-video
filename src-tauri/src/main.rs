#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 桌面壳：起本地服务 → 等端口就绪 → 建窗加载本地页面。
//!
//! 窗口加载的是 `http://127.0.0.1:<port>` 而不是 `tauri://localhost`，
//! 这是历史上唯一稳定不白屏的路径；静态文件作为 bundle resource 随包分发。

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

fn resource_dist() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for rel in ["resources/dist", "dist"] {
                let p = dir.join(rel);
                if p.join("index.html").is_file() {
                    return p;
                }
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist")
}

fn wait_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    false
}

fn main() {
    let port: u16 = std::env::var("XVIDEO_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21600);
    let dist = resource_dist();
    let data_dir = xv_app::data_dir();

    let app_state = xv_app::App::new(data_dir.clone(), dist.clone(), port)
        .unwrap_or_else(|e| panic!("初始化失败：{e:#}"));
    app_state.spawn_ticker();

    let server_state = Arc::clone(&app_state);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("运行时创建失败：{e}");
                return;
            }
        };
        if let Err(e) = rt.block_on(xv_server::serve(server_state, port)) {
            eprintln!("本地服务启动失败：{e:#}");
        }
    });

    if !wait_port(port, Duration::from_secs(8)) {
        eprintln!("本地服务未在预期时间内就绪，仍尝试打开窗口");
    }

    let exit_state = Arc::clone(&app_state);
    tauri::Builder::default()
        .setup(move |app| {
            let url = format!("http://127.0.0.1:{port}");
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("本地地址应当合法")),
            )
            .title("X-Video")
            .inner_size(1440.0, 920.0)
            .min_inner_size(1024.0, 660.0)
            .center()
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tauri 启动失败")
        .run(move |_app, event| {
            if matches!(event, RunEvent::Exit) {
                exit_state.shutdown();
            }
        });
}
