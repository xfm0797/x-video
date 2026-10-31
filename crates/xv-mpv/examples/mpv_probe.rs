//! 本地调试工具（不参与打包）：
//!   cargo run -p xv-mpv --example mpv_probe -- status
//!   cargo run -p xv-mpv --example mpv_probe -- install
//!   cargo run -p xv-mpv --example mpv_probe -- play "http://127.0.0.1:21600/p?u=..."

use std::env;
use std::time::Duration;
use xv_mpv::{engine, OpenRequest, Rect, SurfaceMode};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("status");
    let eng = engine();

    match cmd {
        "status" => match serde_json::to_string_pretty(&eng.status()) {
            Ok(text) => println!("{text}"),
            Err(e) => eprintln!("序列化失败: {e}"),
        },
        "install" => match eng.install() {
            Ok(bin) => println!("安装完成: {} ({})", bin.path, bin.version),
            Err(e) => eprintln!("安装失败: {e:#}"),
        },
        "play" => {
            let url = args.get(1).cloned().unwrap_or_default();
            if url.is_empty() {
                eprintln!("用法: play <url>");
                return;
            }
            let mode = match args.get(2).map(|s| s.as_str()) {
                Some("floating") => SurfaceMode::Floating,
                _ => SurfaceMode::Embed,
            };
            let req = OpenRequest {
                url: url.clone(),
                title: "mpv_probe".into(),
                mode,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 960.0,
                    h: 540.0,
                    scale: 1.0,
                    sx: 80.0,
                    sy: 80.0,
                },
            };
            if let Err(e) = eng.open(req) {
                eprintln!("打开失败: {e:#}");
                return;
            }
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(500));
                let s = eng.status();
                println!(
                    "t={:.1}/{:.1} paused={} idle={} err={:?}",
                    s.state.position, s.state.duration, s.state.paused, s.state.idle, s.state.error
                );
            }
            let _ = eng.close();
        }
        "log" => println!("{}", eng.last_log()),
        other => eprintln!("未知命令: {other}"),
    }
}
