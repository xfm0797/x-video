//! 应用编排：一次播放 = 一个 Session（队列 + 进度 + 连播策略）。
//!
//! 这里是 HTTP 层与库/内核之间的唯一粘合点：负责在第一次播放时创建原生
//! 承载面与 mpv 实例、按剧集顺序推进队列、把播放位置定期落库。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use xv_library::{ItemRow, Library};
use xv_player::Player;
use xv_source::Hub;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub item_id: Option<i64>,
    pub series_id: Option<i64>,
    pub queue: Vec<i64>,
    pub index: usize,
    /// next | loop | single | stop
    pub mode: String,
    #[serde(skip)]
    pub rect: Rect,
    #[serde(skip)]
    pub eof_handled: bool,
    #[serde(skip)]
    pub last_saved: f64,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            item_id: None,
            series_id: None,
            queue: Vec::new(),
            index: 0,
            mode: "next".into(),
            rect: Rect::default(),
            eof_handled: false,
            last_saved: 0.0,
        }
    }
}

/// 把闭包送回 UI 线程执行的调度器（壳层用 `run_on_main_thread` 实现）。
pub type UiDispatch = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

pub struct App {
    pub data_dir: PathBuf,
    pub dist: PathBuf,
    pub port: u16,
    pub library: Library,
    pub sources: Arc<Hub>,
    pub session: Mutex<Session>,
    pub player: Mutex<Option<Arc<Player>>>,
    surface: Arc<Mutex<isize>>,
    /// 由 Tauri 壳注入：把闭包送回 UI 线程执行
    ui: Mutex<Option<UiDispatch>>,
    prefs: Mutex<PlayerPrefs>,
    pub started: Instant,
    pub scanning: AtomicBool,
    pub last_scan: Mutex<Option<xv_library::ScanReport>>,
    ticker_stop: AtomicBool,
}

pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("XVIDEO_DATA_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("XVideo2")
}

impl App {
    pub fn new(data_dir: PathBuf, dist: PathBuf, port: u16) -> Result<Arc<Self>> {
        let library = Library::open(&data_dir)?;
        let sources = Arc::new(Hub::open(&data_dir.join("sources.json"))?);
        let mut prefs = PlayerPrefs::default();
        if let Ok(Some(v)) = library.get_setting("player") {
            if let Ok(p) = serde_json::from_str::<PlayerPrefs>(&v) {
                prefs = p;
            }
        }
        let app = Arc::new(Self {
            data_dir,
            dist,
            port,
            library,
            sources,
            session: Mutex::new(Session::default()),
            player: Mutex::new(None),
            surface: Arc::new(Mutex::new(0)),
            ui: Mutex::new(None),
            prefs: Mutex::new(prefs),
            started: Instant::now(),
            scanning: AtomicBool::new(false),
            last_scan: Mutex::new(None),
            ticker_stop: AtomicBool::new(false),
        });

        Ok(app)
    }

    /// 注入 UI 线程调度能力（壳层在 setup 里调用）。
    pub fn set_ui_dispatch(&self, f: UiDispatch) {
        if let Ok(mut slot) = self.ui.lock() {
            *slot = Some(f);
        }
    }

    /// 在 UI 线程执行并等待完成。没有调度器（如脱离壳跑服务）时返回 false。
    pub fn ui_call<F: FnOnce() + Send + 'static>(&self, f: F) -> bool {
        let dispatch = self.ui.lock().ok().and_then(|g| g.clone());
        let Some(dispatch) = dispatch else {
            return false;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        dispatch(Box::new(move || {
            f();
            let _ = tx.send(());
        }));
        rx.recv_timeout(Duration::from_secs(5)).is_ok()
    }

    /// 全屏切换：改窗口样式必须回 UI 线程做。
    pub fn set_fullscreen(&self, on: bool) -> bool {
        let hwnd = xv_win::main_hwnd();
        if hwnd == 0 {
            return false;
        }
        if self.ui.lock().map(|g| g.is_some()).unwrap_or(false) {
            let done = std::sync::Arc::new(AtomicBool::new(false));
            let flag = std::sync::Arc::clone(&done);
            let ok = self.ui_call(move || {
                let r = xv_win::set_fullscreen(hwnd, on);
                flag.store(r, Ordering::Relaxed);
            });
            return ok && done.load(Ordering::Relaxed);
        }
        xv_win::set_fullscreen(hwnd, on)
    }

    /// 由 UI 线程调用：把原生承载面挂到主窗口上，返回承载面句柄（0 表示没有）。
    pub fn set_surface(&self, parent: isize) -> isize {
        let mut surface = self.surface.lock().unwrap_or_else(|e| e.into_inner());
        if *surface == 0 && parent != 0 {
            let rect = self.session.lock().map(|s| s.rect).unwrap_or_default();
            *surface = xv_win::create_surface(parent, rect.x, rect.y, rect.w, rect.h);
        }
        *surface
    }

    /// 播放内核（首次调用时创建原生承载面）。
    ///
    /// 承载面必须由 **UI 线程** 创建（见 `set_surface`）：Windows 上父窗口
    /// 与子窗口归属不同线程时，`WM_PARENTNOTIFY`、焦点、绘制这些消息会退化成
    /// 跨线程同步调用，父窗口忙一点就互等，表现出来就是「点一下程序未响应」。
    /// 所以这里只读取已经建好的句柄，绝不在请求线程里建窗。
    pub fn player(&self) -> Arc<Player> {
        let mut guard = self.player.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_ref() {
            return Arc::clone(p);
        }
        // 承载面还没建就请 UI 线程来建：跨线程建子窗口会让父子消息跨线程同步。
        if *self.surface.lock().unwrap_or_else(|e| e.into_inner()) == 0 {
            let slot = Arc::clone(&self.surface);
            let created = self.ui_call(move || {
                let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
                if *s == 0 {
                    let parent = xv_win::main_hwnd();
                    if parent != 0 {
                        *s = xv_win::create_surface(parent, 0, 0, 0, 0);
                    }
                }
            });
            if !created {
                // 没有壳（单跑服务）时保持 0，播放器退化为无画面
                eprintln!("[player] 没有 UI 线程调度器，跳过承载面创建");
            }
        }
        let surface = *self.surface.lock().unwrap_or_else(|e| e.into_inner());
        let player = Arc::new(Player::new(surface));
        if let Ok(prefs) = self.prefs.lock() {
            let _ = player.set_volume(prefs.volume);
            let _ = player.set_speed(prefs.speed);
            if player.available() {
                let _ = player.set_pause(false);
            }
        }
        *guard = Some(Arc::clone(&player));
        player
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerPrefs {
    pub volume: f64,
    pub speed: f64,
    /// 播完行为：next / loop / single / stop
    pub mode: String,
    /// auto | software
    pub hwdec: String,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self {
            volume: 100.0,
            speed: 1.0,
            mode: "next".into(),
            hwdec: "auto".into(),
        }
    }
}

impl App {
    pub fn prefs(&self) -> PlayerPrefs {
        self.prefs.lock().map(|p| p.clone()).unwrap_or_default()
    }

    pub fn update_prefs(&self, f: impl FnOnce(&mut PlayerPrefs)) {
        let mut guard = self.prefs.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
        let snapshot = guard.clone();
        drop(guard);
        if let Ok(text) = serde_json::to_string(&snapshot) {
            let _ = self.library.set_setting("player", &text);
        }
    }

    pub fn save_prefs(&self, prefs: &PlayerPrefs) -> Result<()> {
        *self.prefs.lock().unwrap() = prefs.clone();
        self.library
            .set_setting("player", &serde_json::to_string(prefs)?)?;
        Ok(())
    }

    /// 更新视频区矩形（物理像素，相对主窗口客户区）。
    pub fn set_rect(&self, rect: Rect, visible: bool) {
        if let Ok(mut s) = self.session.lock() {
            s.rect = rect;
        }
        let surface = *self.surface.lock().unwrap_or_else(|e| e.into_inner());
        if surface == 0 {
            return;
        }
        let moved = self.ui_call(move || {
            xv_win::set_surface(surface, rect.x, rect.y, rect.w, rect.h, visible);
        });
        if !moved {
            xv_win::set_surface(surface, rect.x, rect.y, rect.w, rect.h, visible);
        }
    }

    pub fn play_item(&self, item_id: i64, start: Option<f64>) -> Result<()> {
        let item = self
            .library
            .item(item_id)?
            .ok_or_else(|| anyhow!("条目不存在：{item_id}"))?;

        let siblings = self.library.series_of_item(item_id).unwrap_or_default();
        let queue: Vec<i64> = if siblings.is_empty() {
            vec![item_id]
        } else {
            siblings.iter().map(|i| i.id).collect()
        };
        let index = queue.iter().position(|id| *id == item_id).unwrap_or(0);

        let resume = start.unwrap_or(item.position);
        {
            let mut s = self.session.lock().unwrap();
            s.item_id = Some(item_id);
            s.series_id = item.series_id;
            s.queue = queue;
            s.index = index;
            s.eof_handled = false;
            s.last_saved = resume;
            let mode = self.prefs.lock().map(|p| p.mode.clone()).unwrap_or_default();
            if !mode.is_empty() {
                s.mode = mode;
            }
        }

        let player = self.player();
        if !player.available() {
            return Err(anyhow!(player.state().lib_error.unwrap_or_else(|| {
                "播放内核未就绪".to_string()
            })));
        }
        player.open(&item.path, resume)?;
        Ok(())
    }

    pub fn play_path(&self, path: &str) -> Result<()> {
        if let Some(item) = self.library.item_by_path(path)? {
            return self.play_item(item.id, None);
        }
        {
            let mut s = self.session.lock().unwrap();
            s.item_id = None;
            s.queue = Vec::new();
            s.index = 0;
            s.eof_handled = false;
        }
        let player = self.player();
        player.open(path, 0.0)?;
        Ok(())
    }

    /// 播放一个网络地址（直播频道 / 在线影视）。不进入本地队列。
    pub fn play_url(&self, url: &str, headers: &str) -> Result<()> {
        self.play_candidates(&[url.to_string()], headers)
            .map(|_| ())
    }

    /// 按顺序尝试多个直播地址：每个地址等它真正出画面（FILE_LOADED）或报错，
    /// 失败自动切下一个。IPTV 的「频道名,url1#url2#url3」多源就是为这个准备的。
    /// 返回成功命中的候选下标。
    pub fn play_candidates(&self, urls: &[String], headers: &str) -> Result<usize> {
        // 去重、去空，最多 4 个，别让一次点击卡太久
        let mut cands: Vec<String> = Vec::new();
        for u in urls {
            let u = u.trim();
            if !u.is_empty() && !cands.iter().any(|c| c == u) {
                cands.push(u.to_string());
            }
            if cands.len() >= 4 {
                break;
            }
        }
        anyhow::ensure!(!cands.is_empty(), "没有可播放的地址");

        {
            let mut s = self.session.lock().unwrap_or_else(|e| e.into_inner());
            s.item_id = None;
            s.series_id = None;
            s.queue.clear();
            s.index = 0;
            s.eof_handled = true; // 网络流不参与自动连播
        }
        let player = self.player();
        if !player.available() {
            return Err(anyhow!(player
                .state()
                .lib_error
                .unwrap_or_else(|| "播放内核未就绪".to_string())));
        }

        let mut fails: Vec<String> = Vec::new();
        let mut raws: Vec<String> = Vec::new();
        for (i, url) in cands.iter().enumerate() {
            player.open_with(url, 0.0, headers)?;
            if player.wait_ready(10.0) {
                return Ok(i);
            }
            let st = player.state();
            fails.push(st.error.unwrap_or_else(|| "连接超时".to_string()));
            if let Some(d) = st.error_detail {
                if !d.is_empty() {
                    raws.push(format!(">> {url}\n{d}"));
                }
            }
        }

        let detail = if raws.is_empty() {
            None
        } else {
            Some(raws.join("\n\n"))
        };
        let summary = format!(
            "试了 {} 个直播地址都无法播放。最后失败原因：{}",
            cands.len(),
            fails.last().cloned().unwrap_or_default()
        );
        player.set_play_error(summary.clone(), detail);
        Err(anyhow!(summary))
    }

    pub fn next(&self, auto: bool) -> Result<()> {
        let (next_id, _mode) = {
            let s = self.session.lock().unwrap();
            let mode = s.mode.clone();
            if s.queue.is_empty() {
                (None, mode)
            } else if s.index + 1 < s.queue.len() {
                (Some(s.queue[s.index + 1]), mode)
            } else if mode == "loop" {
                (s.queue.first().copied(), mode)
            } else {
                (None, mode)
            }
        };
        match next_id {
            Some(id) => self.play_item(id, Some(0.0)),
            None => {
                if auto {
                    let _ = self.player().set_pause(true);
                }
                Ok(())
            }
        }
    }

    pub fn prev(&self) -> Result<()> {
        let id = {
            let s = self.session.lock().unwrap();
            if s.index > 0 {
                Some(s.queue[s.index - 1])
            } else {
                None
            }
        };
        match id {
            Some(id) => self.play_item(id, Some(0.0)),
            None => Ok(()),
        }
    }

    /// 后台心跳：落进度 + 自动连播。
    pub fn spawn_ticker(self: &Arc<Self>) {
        let app = Arc::clone(self);
        std::thread::Builder::new()
            .name("session-ticker".into())
            .spawn(move || loop {
                if app.ticker_stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(900));
                app.tick();
            })
            .ok();
    }

    fn tick(&self) {
        let player = {
            let guard = self.player.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(p) => Arc::clone(p),
                None => return,
            }
        };
        let st = player.state();
        let (item_id, mode) = {
            let s = self.session.lock().unwrap_or_else(|e| e.into_inner());
            (s.item_id, s.mode.clone())
        };

        if let Some(id) = item_id {
            if st.position > 1.0 && st.position - self.session.lock().unwrap().last_saved >= 4.0 {
                let _ = self.library.save_progress(id, st.position, st.duration);
                if let Ok(mut s) = self.session.lock() {
                    s.last_saved = st.position;
                }
            }
        }

        if st.eof {
            let should_advance = {
                let mut s = self.session.lock().unwrap_or_else(|e| e.into_inner());
                if s.eof_handled {
                    false
                } else {
                    s.eof_handled = true;
                    true
                }
            };
            if should_advance && mode != "stop" {
                if let Some(id) = item_id {
                    if let Ok(Some(item)) = self.library.item(id) {
                        let _ = self
                            .library
                            .save_progress(id, item.duration.max(st.duration), item.duration.max(st.duration));
                    }
                }
                if mode == "single" {
                    let _ = player.seek(0.0, true);
                    let _ = player.set_pause(false);
                    if let Ok(mut s) = self.session.lock() {
                        s.eof_handled = false;
                    }
                } else {
                    let _ = self.next(true);
                }
            }
        }
    }

    pub fn shutdown(&self) {
        self.ticker_stop.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.player.lock() {
            if let Some(p) = guard.take() {
                let st = p.state();
                let id = self.session.lock().ok().and_then(|s| s.item_id);
                if let Some(id) = id {
                    let _ = self.library.save_progress(id, st.position, st.duration);
                }
                p.shutdown();
            }
        }
        let surface = *self.surface.lock().unwrap_or_else(|e| e.into_inner());
        if surface != 0 {
            if !self.ui_call(move || xv_win::destroy_surface(surface)) {
                xv_win::destroy_surface(surface);
            }
        }
    }

    /// 后台扫描一个目录，进度与结果通过 `scan_state` 读取。
    pub fn start_scan(self: &Arc<Self>, path: String) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let _ = app.library.add_root(&path);
            let report = match xv_library::scan_path(&app.library, Path::new(&path)) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("[scan] {e:#}");
                    None
                }
            };
            if let Ok(mut slot) = app.last_scan.lock() {
                *slot = report;
            }
            app.scanning.store(false, Ordering::SeqCst);
        });
    }

    pub fn scan_state(&self) -> (bool, Option<xv_library::ScanReport>) {
        let scanning = self.scanning.load(Ordering::SeqCst);
        let last = self.last_scan.lock().ok().and_then(|s| s.clone());
        (scanning, last)
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn items_of_series(&self, series_id: i64) -> Result<Vec<ItemRow>> {
        self.library.items_of_series(series_id)
    }
}
