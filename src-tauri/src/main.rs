// 启动内嵌 API/代理服务，再打开主窗口。
// 打包版窗口直接加载 http://127.0.0.1:21600（由内嵌服务托管前端），
// 这样前端与 API 同源，避免 tauri:// 资源协议下接口不可用导致的白屏。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

fn main() {
    let port: u16 = std::env::var("XVIDEO_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(21600);

    // 先同步绑定端口：绑定成功说明服务可用，窗口随后创建即可正常加载
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).ok();
    let server_ready = listener.is_some();

    tauri::Builder::default()
        .setup(move |app| {
            // 静态资源目录：打包后取 resources/dist，开发态回退仓库 dist
            let mut static_dir: Option<std::path::PathBuf> = None;
            if let Ok(dir) = app.path().resource_dir() {
                let candidate = dir.join("dist");
                if candidate.join("index.html").exists() {
                    static_dir = Some(candidate);
                }
            }
            if static_dir.is_none() {
                let dev_dist = std::path::PathBuf::from("../dist");
                if dev_dist.join("index.html").exists() {
                    static_dir = Some(dev_dist);
                }
            }
            if let Some(dir) = &static_dir {
                std::env::set_var("XVIDEO_STATIC_DIR", dir);
            } else {
                eprintln!("警告：未找到前端静态资源目录，界面可能无法加载");
            }

            // 在独立线程启动服务
            if let Some(std_listener) = listener {
                std::thread::spawn(move || {
                    let result: anyhow::Result<()> = (|| {
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .enable_all()
                            .build()?;
                        rt.block_on(xv_server::serve_on(std_listener, port))
                    })();
                    if let Err(e) = result {
                        eprintln!("内嵌服务启动失败: {e:#}");
                    }
                });
            } else {
                eprintln!("端口 {port} 被占用，改为加载内置前端资源");
            }

            let url = if !cfg!(debug_assertions) && server_ready {
                WebviewUrl::External(format!("http://127.0.0.1:{port}").parse()?)
            } else {
                WebviewUrl::App("index.html".into())
            };

            WebviewWindowBuilder::new(app, "main", url)
                .title("X-Video 播放器")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 640.0)
                .center()
                .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
