// 启动内嵌 API/代理服务，再打开主窗口加载前端界面。
// 播放、源解析等能力全部由 xv-core/xv-server 通过 HTTP API 提供。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 独立线程运行 tokio runtime + axum 服务
    std::thread::spawn(|| {
        let port: u16 = std::env::var("XVIDEO_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(21600);
        let result: anyhow::Result<()> = (|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(xv_server::serve(port))
        })();
        if let Err(e) = result {
            eprintln!("内嵌服务启动失败: {e:#}");
        }
    });

    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
