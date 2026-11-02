//! 开发用后端：只起 HTTP 服务，不创建窗口。
//!
//! 用途：用浏览器直接调试界面（`npx vite` 代理到这里），以及给 UI 截图。
//! 生产运行走 `src-tauri` 的桌面壳。

use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("XVIDEO_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21600);

    let data_dir = std::env::var("XVIDEO_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("xvideo-data"));

    let dist = std::env::var("XVIDEO_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist"));

    let app = xv_app::App::new(data_dir.clone(), dist.clone(), port)?;
    app.spawn_ticker();

    println!("[dev] dist = {}", dist.display());
    println!("[dev] data = {}", data_dir.display());
    xv_server::serve(app, port).await
}
