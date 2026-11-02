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

pub struct App {
    pub data_dir: PathBuf,
    pub dist: PathBuf,
    pub port: u16,
    pub library: Library,
    pub session: Mutex<Session>,
    pub player: Mutex<Option<Arc<Player>>>,
    surface: Mutex<isize>,
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

fn wait_parent(timeout: Duration) -> isize {
    let deadline = Instant::now() + timeout;
    loop {
        let h = xv_win::main_hwnd();
        if h != 0 {
            return h;
        }
        if Instant::now() > deadline {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}

impl App {
    pub fn new(data_dir: PathBuf, dist: PathBuf, port: u16) -> Result<Arc<Self>> {
        let library = Library::open(&data_dir)?;
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
            session: Mutex::new(Session::default()),
            player: Mutex::new(None),
            surface: Mutex::new(0),
            prefs: Mutex::new(prefs),
            started: Instant::now(),
            scanning: AtomicBool::new(false),
            last_scan: Mutex::new(None),
            ticker_stop: AtomicBool::new(false),
        });

        Ok(app)
    }

    /// 播放内核（首次调用时创建原生承载面）。
    pub fn player(&self) -> Arc<Player> {
        let mut guard = self.player.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_ref() {
            return Arc::clone(p);
        }
        let mut surface = self.surface.lock().unwrap_or_else(|e| e.into_inner());
        if *surface == 0 {
            let parent = wait_parent(Duration::from_millis(2500));
            if parent != 0 {
                let rect = self.session.lock().map(|s| s.rect).unwrap_or_default();
                *surface = xv_win::create_surface(parent, rect.x, rect.y, rect.w, rect.h);
            }
        }
        let player = Arc::new(Player::new(*surface));
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
        if surface != 0 {
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

    pub fn next(&self, auto: bool) -> Result<()> {
        let (next_id, mode) = {
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
            xv_win::destroy_surface(surface);
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
