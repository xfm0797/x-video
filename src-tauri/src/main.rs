#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 桌面壳：起本地服务 → 等端口就绪 → 建窗加载本地页面。
//!
//! 窗口加载的是 `http://127.0.0.1:<port>` 而不是 `tauri://localhost`，
//! 这是历史上唯一稳定不白屏的路径；静态文件作为 bundle resource 随包分发。

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// 界面线程看门狗：每秒给主窗口发一个空消息，记录响应耗时。
///
/// 用户反馈过「点一下就没响应」，而事后抓的 dump 里主线程都停在消息循环里空转，
/// 复现又不稳定。所以把探测放在程序里跑：真出现长时间无响应时，
/// 日志里有准确时间点，并且自动留一份 dump 供事后分析。
#[cfg(windows)]
mod watchdog {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const WM_NULL: u32 = 0;
    const SMTO_BLOCK: u32 = 0x0001;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    /// 超过这个耗时就算一次「卡顿」
    const SLOW_MS: u128 = 800;
    /// 超过这个耗时判定为无响应，写 dump
    const HANG_MS: u128 = 8_000;

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    #[link(name = "user32")]
    extern "system" {
        fn IsWindow(hwnd: isize) -> i32;
    }

    fn window_alive(hwnd: isize) -> bool {
        unsafe { IsWindow(hwnd) != 0 }
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetCurrentProcessId() -> u32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sa: *mut core::ffi::c_void,
            disposition: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        fn CloseHandle(h: isize) -> i32;
    }

    #[link(name = "Dbghelp")]
    extern "system" {
        fn MiniDumpWriteDump(
            process: isize,
            pid: u32,
            file: isize,
            dump_type: u32,
            exception: *mut core::ffi::c_void,
            user: *mut core::ffi::c_void,
            callback: *mut core::ffi::c_void,
        ) -> i32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// 写一个精简 dump（只要线程栈，不含全内存，体积可控）。
    fn write_dump(dir: &PathBuf) -> Option<PathBuf> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("ui-hang-{ts}.dmp"));
        let w = wide(&path);
        unsafe {
            // GENERIC_WRITE | CREATE_ALWAYS
            let file = CreateFileW(w.as_ptr(), 0x4000_0000, 0, std::ptr::null_mut(), 2, 0, 0);
            if file == -1 {
                return None;
            }
            // MiniDumpWithDataSegs | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules
            let ok = MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                file,
                (0x1 | 0x1000 | 0x10000) as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            CloseHandle(file);
            if ok == 0 {
                None
            } else {
                Some(path)
            }
        }
    }

    fn log_line(dir: &PathBuf, msg: &str) {
        let path = dir.join("ui-watchdog.log");
        let big = std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false);
        let mut opt = OpenOptions::new();
        opt.create(true).append(true).write(true);
        if big {
            opt.truncate(false);
        }
        if let Ok(mut f) = opt.open(&path) {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{ts}] {msg}");
        }
    }

    pub fn spawn(dir: PathBuf, stop: Arc<AtomicBool>) {
        std::thread::Builder::new()
            .name("ui-watchdog".into())
            .spawn(move || {
                // 窗口由 Tauri 在主线程建好后才能探测，这里等它出现
                let mut hwnd = 0isize;
                for _ in 0..150 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    hwnd = ::xv_win::main_hwnd();
                    if hwnd != 0 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                log_line(
                    &dir,
                    &format!("watchdog start hwnd={hwnd:#x} pid={}", unsafe { GetCurrentProcessId() }),
                );
                let mut warned = false;
                let mut dumps = 0u32;
                let mut worst = 0u128;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        log_line(&dir, &format!("watchdog stop, worst={worst}ms"));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(1000));
                    if hwnd != 0 && !window_alive(hwnd) {
                        log_line(&dir, "main window gone");
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        hwnd = ::xv_win::main_hwnd();
                        continue;
                    }
                    let t = Instant::now();
                    let mut out: usize = 0;
                    let ok = unsafe {
                        SendMessageTimeoutW(
                            hwnd,
                            WM_NULL,
                            0,
                            0,
                            SMTO_BLOCK | SMTO_ABORTIFHUNG,
                            HANG_MS as u32,
                            &mut out,
                        )
                    };
                    let ms = t.elapsed().as_millis();
                    if ms > worst {
                        worst = ms;
                    }
                    if ms >= SLOW_MS {
                        log_line(&dir, &format!("slow-response {ms}ms ok={ok}"));
                    }
                    if ok == 0 {
                        if !warned {
                            warned = true;
                            log_line(&dir, &format!("UNRESPONSIVE after {ms}ms"));
                        }
                        if dumps < 2 {
                            match write_dump(&dir) {
                                Some(p) => {
                                    dumps += 1;
                                    log_line(&dir, &format!("dump written: {}", p.display()));
                                }
                                None => log_line(&dir, "dump failed"),
                            }
                        }
                    } else {
                        warned = false;
                    }
                }
            })
            .ok();
    }
}

fn main() {
    let port: u16 = std::env::var("XVIDEO_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21600);
    let dist = resource_dist();
    let data_dir = xv_app::data_dir();

    // 安装版没有控制台：panic 与 backtrace 落盘，便于事后定位
    let crash_dir = data_dir.clone();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        std::fs::create_dir_all(&crash_dir).ok();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(crash_dir.join("crash.log"))
        {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{ts}] {info}");
            let _ = writeln!(f, "{:?}\n", std::backtrace::Backtrace::force_capture());
        }
    }));

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
    let wd_dir = data_dir.clone();
    let boot_dir = data_dir.clone();
    let wd_stop = Arc::new(AtomicBool::new(false));
    let wd_setup = Arc::clone(&wd_stop);
    let wd_exit = Arc::clone(&wd_stop);
    tauri::Builder::default()
        .setup(move |app| {
            std::fs::create_dir_all(&boot_dir).ok();
            let _ = std::fs::write(boot_dir.join("boot.log"), "setup enter\n");
            let handle = app.handle().clone();
            app_state.set_ui_dispatch(Arc::new(move |f: Box<dyn FnOnce() + Send>| {
                let _ = handle.run_on_main_thread(move || f());
            }));
            let url = format!("http://127.0.0.1:{port}");
            let win = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("本地地址应当合法")),
            )
            .title("X-Video")
            .inner_size(1440.0, 920.0)
            .min_inner_size(1024.0, 660.0)
            .center()
            .build()?;
            let _ = &win;
            #[cfg(windows)]
            watchdog::spawn(wd_dir, Arc::clone(&wd_setup));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tauri 启动失败")
        .run(move |_app, event| {
            if matches!(event, RunEvent::Exit) {
                wd_exit.store(true, Ordering::Relaxed);
                exit_state.shutdown();
            }
        });
}
