//! X-Video 的播放内核封装（libmpv）。
//!
//! 设计要点：
//! - 用 mpv 可执行文件 + `--input-ipc-server` 的 JSON IPC 控制，不链接 libmpv，
//!   这样 CI 三平台都不需要预装 mpv 开发库，内核可独立升级。
//! - 画面上屏有两条路径：Windows 自建原生窗口并交给 mpv `--wid`（真嵌入），
//!   其它平台退化为无边框置顶浮窗（`--geometry` 定位）。
//! - 所有对外方法都是阻塞的，HTTP 层用 `spawn_blocking` 调用。

mod install;
mod ipc;
mod surface;

pub use install::{bin_name, detect, install_dir, set_custom_path, MpvBinary};
pub use surface::{Rect, SurfaceMode};

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// mpv 播放状态（由 IPC 事件线程持续刷新）
#[derive(Debug, Clone, Serialize)]
pub struct MpvState {
    pub running: bool,
    pub idle: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub mute: bool,
    pub eof: bool,
    pub title: String,
    pub error: Option<String>,
}

impl Default for MpvState {
    fn default() -> Self {
        Self {
            running: false,
            idle: true,
            paused: true,
            position: 0.0,
            duration: 0.0,
            volume: 100.0,
            mute: false,
            eof: false,
            title: String::new(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub platform: &'static str,
    pub can_install: bool,
    pub install_dir: String,
    pub installing: bool,
    pub progress: u64,
    #[serde(flatten)]
    pub state: MpvState,
}

#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub url: String,
    pub title: String,
    pub mode: SurfaceMode,
    pub rect: Rect,
}

struct Inner {
    child: Option<Child>,
    ipc: Option<ipc::Ipc>,
    surface: Option<surface::Surface>,
    log: ipc::LogBuf,
}

pub struct Engine {
    inner: Mutex<Inner>,
    /// None = 尚未探测；Some(None) = 探测过但没有
    bin: Mutex<Option<Option<MpvBinary>>>,
    state: Arc<Mutex<MpvState>>,
    installing: AtomicBool,
    progress: AtomicU64,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

/// 全局单例：HTTP 服务与桌面壳共用同一份内核进程
pub fn engine() -> &'static Arc<Engine> {
    ENGINE.get_or_init(|| Arc::new(Engine::new()))
}

impl Engine {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                child: None,
                ipc: None,
                surface: None,
                log: ipc::new_log(),
            }),
            bin: Mutex::new(None),
            state: Arc::new(Mutex::new(MpvState::default())),
            installing: AtomicBool::new(false),
            progress: AtomicU64::new(0),
        }
    }

    fn cached_bin(&self) -> Option<MpvBinary> {
        let mut guard = self.bin.lock().unwrap();
        if guard.is_none() {
            *guard = Some(detect());
        }
        guard.clone().flatten()
    }

    /// 重新探测（用户装好 mpv 或填了路径后调用）
    pub fn probe(&self) -> Status {
        *self.bin.lock().unwrap() = None;
        self.status()
    }

    pub fn status(&self) -> Status {
        let bin = self.cached_bin();
        let state = self.state.lock().unwrap().clone();
        Status {
            available: bin.is_some(),
            path: bin.as_ref().map(|b| b.path.clone()),
            version: bin.as_ref().map(|b| b.version.clone()),
            source: bin.as_ref().map(|b| b.source.clone()),
            platform: std::env::consts::OS,
            can_install: cfg!(windows),
            install_dir: install_dir().display().to_string(),
            installing: self.installing.load(Ordering::SeqCst),
            progress: self.progress.load(Ordering::SeqCst),
            state,
        }
    }

    pub fn install(&self) -> Result<MpvBinary> {
        if self.installing.swap(true, Ordering::SeqCst) {
            return Err(anyhow!("正在下载安装中，请稍候"));
        }
        self.progress.store(0, Ordering::SeqCst);
        let result = install::download_and_install(&|p| self.progress.store(p, Ordering::SeqCst));
        self.installing.store(false, Ordering::SeqCst);
        match result {
            Ok(bin) => {
                self.progress.store(100, Ordering::SeqCst);
                *self.bin.lock().unwrap() = Some(Some(bin.clone()));
                Ok(bin)
            }
            Err(e) => {
                self.progress.store(0, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// 打开地址：必要时建承载窗口、拉起 mpv、下发 loadfile
    pub fn open(&self, req: OpenRequest) -> Result<()> {
        let bin = self
            .cached_bin()
            .ok_or_else(|| anyhow!("未检测到 mpv 内核，请在设置页安装或指定 mpv 路径"))?;

        let mut inner = self.inner.lock().unwrap();

        // 进程已退出则先清理
        if let Some(child) = inner.child.as_mut() {
            if let Ok(Some(_)) = child.try_wait() {
                inner.shutdown();
            }
        }

        let need_surface = match &inner.surface {
            Some(s) => s.mode != req.mode,
            None => true,
        };
        if need_surface {
            inner.shutdown();
            inner.surface = Some(surface::create(req.mode, req.rect)?);
        } else if let Some(s) = inner.surface.as_mut() {
            s.resize(req.rect)?;
        }

        if inner.child.is_none() {
            let endpoint = ipc::endpoint();
            let surface = inner
                .surface
                .as_ref()
                .ok_or_else(|| anyhow!("mpv 输出窗口未就绪"))?;
            let title: &str = if req.title.is_empty() {
                "X-Video"
            } else {
                req.title.as_str()
            };
            let (child, handle) = ipc::spawn(
                &bin,
                surface,
                title,
                &endpoint,
                self.state.clone(),
                inner.log.clone(),
            )?;
            inner.child = Some(child);
            inner.ipc = Some(handle);
        } else if let Some(s) = inner.surface.as_mut() {
            s.resize(req.rect)?;
        }

        {
            let mut st = self.state.lock().unwrap();
            st.running = true;
            st.idle = false;
            st.eof = false;
            st.error = None;
            st.position = 0.0;
            st.duration = 0.0;
            st.paused = false;
            st.title = req.title.clone();
        }

        let handle = inner
            .ipc
            .as_ref()
            .ok_or_else(|| anyhow!("mpv IPC 未就绪"))?;
        handle
            .command(vec![
                Value::from("loadfile"),
                Value::from(req.url),
                Value::from("replace"),
            ])
            .map_err(|e| anyhow!("下发播放地址失败：{e}"))
    }

    pub fn set_rect(&self, rect: Rect) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(s) = inner.surface.as_mut() {
            s.resize(rect)?;
        }
        Ok(())
    }

    /// 通用命令透传，如 `["set_property","pause",true]` / `["seek",10,"relative"]`
    pub fn command(&self, args: Vec<Value>) -> Result<()> {
        if args.is_empty() {
            return Err(anyhow!("命令为空"));
        }
        let inner = self.inner.lock().unwrap();
        let handle = inner
            .ipc
            .as_ref()
            .ok_or_else(|| anyhow!("mpv 内核未启动"))?;
        handle.command(args)
    }

    pub fn close(&self) -> Result<()> {
        {
            let mut st = self.state.lock().unwrap();
            *st = MpvState::default();
        }
        let mut inner = self.inner.lock().unwrap();
        inner.shutdown();
        Ok(())
    }

    /// 进程退出前的兜底清理
    pub fn shutdown(&self) {
        let _ = self.close();
    }

    pub fn last_log(&self) -> String {
        ipc::log_tail(&self.inner.lock().unwrap().log, 6)
    }
}

impl Inner {
    fn shutdown(&mut self) {
        if let Some(handle) = self.ipc.take() {
            let _ = handle.command(vec![Value::from("quit")]);
            std::thread::sleep(Duration::from_millis(120));
        }
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_millis(900);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {
                        if Instant::now() > deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(60));
                    }
                    Err(_) => break,
                }
            }
        }
        if let Some(s) = self.surface.take() {
            s.destroy();
        }
    }
}
