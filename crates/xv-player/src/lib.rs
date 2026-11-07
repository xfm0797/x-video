//! 播放内核：单个 libmpv 实例 + 事件线程 + 状态快照。
//!
//! 设计要点：
//! - 状态由事件线程写、HTTP 侧读，用一把短锁保护；前端按 4Hz 轮询快照即可，
//!   不需要 SSE，也不用把 mpv 的指针暴露出去。
//! - 所有命令走 `mpv_command` 的数组形式，避免 Windows 路径里的反斜杠被
//!   mpv 的命令行解析器吃掉。
//! - 库缺失不是致命错误：库、浏览、设置照常可用，只有播放会返回明确提示。

mod probe;
mod sys;

pub use probe::{probe_file, thumbnail, Probe};
pub use sys::{Api, Mpv};

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub lang: String,
    pub title: String,
    pub codec: String,
    pub selected: bool,
    pub channels: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct State {
    pub available: bool,
    pub lib_path: String,
    pub lib_error: Option<String>,
    pub loaded: bool,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub title: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub vcodec: String,
    pub acodec: String,
    pub container: String,
    pub hwid: String,
    pub audio_tracks: Vec<Track>,
    pub sub_tracks: Vec<Track>,
    pub audio_id: Option<i64>,
    pub sub_id: Option<i64>,
    pub eof: bool,
    pub error: Option<String>,
}

struct Inner {
    mpv: Arc<Mpv>,
}

impl Inner {
    fn tracks(&self) -> (Vec<Track>, Vec<Track>) {
        let mut audio = Vec::new();
        let mut subs = Vec::new();
        let count = self.mpv.get_i64("track-list/count").unwrap_or(0);
        for i in 0..count {
            let kind = self
                .mpv
                .get_string(&format!("track-list/{i}/type"))
                .unwrap_or_default();
            if kind != "audio" && kind != "sub" {
                continue;
            }
            let t = Track {
                id: self.mpv.get_i64(&format!("track-list/{i}/id")).unwrap_or(0),
                lang: self
                    .mpv
                    .get_string(&format!("track-list/{i}/lang"))
                    .unwrap_or_default(),
                title: self
                    .mpv
                    .get_string(&format!("track-list/{i}/title"))
                    .unwrap_or_default(),
                codec: self
                    .mpv
                    .get_string(&format!("track-list/{i}/codec"))
                    .unwrap_or_default(),
                selected: self
                    .mpv
                    .get_flag(&format!("track-list/{i}/selected"))
                    .unwrap_or(false),
                channels: self
                    .mpv
                    .get_string(&format!("track-list/{i}/demux-channel-count"))
                    .unwrap_or_default(),
            };
            if kind == "audio" {
                audio.push(t);
            } else {
                subs.push(t);
            }
        }
        (audio, subs)
    }
}

pub struct Player {
    state: Arc<Mutex<State>>,
    inner: Option<Arc<Inner>>,
    surface: AtomicIsize,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    lib_error: Option<String>,
}

impl Player {
    /// `surface` 是承载画面的子窗口句柄；0 表示只出声不出画（例如纯音频库）。
    pub fn new(surface: isize) -> Self {
        let mut state = State {
            available: false,
            volume: 100.0,
            speed: 1.0,
            ..Default::default()
        };

        let (inner, lib_error) = match sys::locate().and_then(|p| {
            let api = Api::load(&p)?;
            Ok::<_, anyhow::Error>((Box::leak(Box::new(api)) as &'static Api, p))
        }) {
            Ok((api, path)) => {
                state.lib_path = path.display().to_string();
                match Mpv::new(api, &playback_options(surface)) {
                    Ok(mpv) => (Some(Arc::new(Inner { mpv: Arc::new(mpv) })), None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                }
            }
            Err(e) => (None, Some(format!("{e:#}"))),
        };

        if inner.is_some() {
            state.available = true;
        } else {
            state.lib_error = lib_error.clone();
        }

        let state = Arc::new(Mutex::new(state));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = inner.as_ref().map(|i| {
            let s = Arc::clone(&state);
            let st = Arc::clone(&stop);
            let inner = Arc::clone(i);
            std::thread::Builder::new()
                .name("mpv-events".into())
                .spawn(move || event_loop(inner, s, st))
                .ok()
        });

        let player = Self {
            state,
            inner,
            surface: AtomicIsize::new(surface),
            stop,
            thread: Mutex::new(thread.flatten()),
            lib_error,
        };

        if player.inner.is_some() {
            player.apply_defaults();
        }
        player
    }

    fn apply_defaults(&self) {
        let Some(inner) = &self.inner else { return };
        let props: &[(&str, i32)] = &[
            ("time-pos", sys::FORMAT_DOUBLE),
            ("duration", sys::FORMAT_DOUBLE),
            ("pause", sys::FORMAT_FLAG),
            ("volume", sys::FORMAT_DOUBLE),
            ("mute", sys::FORMAT_FLAG),
            ("speed", sys::FORMAT_DOUBLE),
            ("media-title", sys::FORMAT_STRING),
        ];
        for (name, fmt) in props {
            let _ = inner.mpv.observe(name, *fmt);
        }
        let _ = inner.mpv.set_f64("volume", 100.0);
    }

    pub fn available(&self) -> bool {
        self.inner.is_some()
    }

    pub fn surface(&self) -> isize {
        self.surface.load(Ordering::Relaxed)
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn inner(&self) -> Result<&Arc<Inner>> {
        self.inner
            .as_ref()
            .ok_or_else(|| anyhow!(self.lib_error.clone().unwrap_or_else(|| "播放内核未就绪".into())))
    }

    /// 打开一个本地文件或网络地址。`start` 为续播秒数（0 表示从头）。
    pub fn open(&self, target: &str, start: f64) -> Result<()> {
        let inner = self.inner()?;
        {
            let mut s = self.state.lock().unwrap();
            s.loaded = true;
            s.eof = false;
            s.error = None;
            s.position = start.max(0.0);
            s.path = target.to_string();
            s.title = Path::new(target)
                .file_stem()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| target.to_string());
            s.width = 0;
            s.height = 0;
            s.audio_tracks.clear();
            s.sub_tracks.clear();
        }

        if start > 1.0 {
            let _ = inner.mpv.option("start", &format!("{start:.3}"));
        } else {
            let _ = inner.mpv.option("start", "none");
        }
        inner.mpv.command(&["loadfile", target, "replace"])?;
        Ok(())
    }

    /// 打开网络地址并附带请求头（防盗链 / Referer）。`headers` 形如
    /// `Referer: https://a/,User-Agent: xxx`，空串表示不带。
    pub fn open_with(&self, target: &str, start: f64, headers: &str) -> Result<()> {
        let inner = self.inner()?;
        let _ = inner.mpv.option("http-header-fields", headers.trim());
        self.open(target, start)
    }

    /// 只补一条到播放列表尾部，用于连续播放时预取。
    pub fn append(&self, target: &str) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.command(&["loadfile", target, "append"])
    }

    pub fn play_pause(&self) -> Result<()> {
        let inner = self.inner()?;
        let paused = inner.mpv.get_flag("pause").unwrap_or(false);
        inner.mpv.set_flag("pause", !paused)?;
        if let Ok(mut s) = self.state.lock() {
            s.playing = paused;
        }
        Ok(())
    }

    pub fn set_pause(&self, pause: bool) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_flag("pause", pause)?;
        if let Ok(mut s) = self.state.lock() {
            s.playing = !pause;
        }
        Ok(())
    }

    pub fn seek(&self, seconds: f64, absolute: bool) -> Result<()> {
        let inner = self.inner()?;
        let mode = if absolute { "absolute+exact" } else { "relative+exact" };
        inner.mpv.command(&["seek", &format!("{seconds:.3}"), mode])?;
        if absolute {
            if let Ok(mut s) = self.state.lock() {
                s.position = seconds;
                s.eof = false;
            }
        }
        Ok(())
    }

    pub fn set_volume(&self, v: f64) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_f64("volume", v.clamp(0.0, 150.0))?;
        if let Ok(mut s) = self.state.lock() {
            s.volume = v;
            s.muted = v <= 0.01;
        }
        let _ = inner.mpv.set_flag("mute", v <= 0.01);
        Ok(())
    }

    pub fn set_mute(&self, muted: bool) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_flag("mute", muted)?;
        if let Ok(mut s) = self.state.lock() {
            s.muted = muted;
        }
        Ok(())
    }

    pub fn set_speed(&self, v: f64) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_f64("speed", v.clamp(0.25, 4.0))?;
        if let Ok(mut s) = self.state.lock() {
            s.speed = v;
        }
        Ok(())
    }

    /// 选择音轨；-1 表示关闭。
    pub fn set_audio(&self, id: i64) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_string("aid", &id.to_string())?;
        self.refresh_tracks();
        Ok(())
    }

    /// 选择字幕轨；-1 表示关闭。
    pub fn set_sub(&self, id: i64) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_string("sid", &id.to_string())?;
        self.refresh_tracks();
        Ok(())
    }

    pub fn add_sub(&self, path: &str) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.command(&["sub-add", path, "select"])?;
        self.refresh_tracks();
        Ok(())
    }

    pub fn set_sub_delay(&self, secs: f64) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_f64("sub-delay", secs)?;
        Ok(())
    }

    pub fn set_aspect(&self, value: &str) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.set_string("video-aspect-override", value)?;
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let inner = self.inner()?;
        inner.mpv.command(&["stop"])?;
        if let Ok(mut s) = self.state.lock() {
            s.loaded = false;
            s.playing = false;
            s.eof = false;
            s.position = 0.0;
        }
        Ok(())
    }

    pub fn refresh_tracks(&self) {
        let Some(inner) = &self.inner else { return };
        let (audio, subs) = inner.tracks();
        let audio_id = audio.iter().find(|t| t.selected).map(|t| t.id);
        let sub_id = subs.iter().find(|t| t.selected).map(|t| t.id);
        if let Ok(mut s) = self.state.lock() {
            s.audio_tracks = audio;
            s.sub_tracks = subs;
            s.audio_id = audio_id;
            s.sub_id = sub_id;
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(inner) = &self.inner {
            inner.mpv.command(&["quit"]).ok();
        }
        if let Some(h) = self.thread.lock().ok().and_then(|mut t| t.take()) {
            let _ = h.join();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn playback_options(surface: isize) -> Vec<(&'static str, String)> {
    let mut opts: Vec<(&'static str, String)> = vec![
        ("config", "no".into()),
        ("terminal", "no".into()),
        ("input-default-bindings", "no".into()),
        ("input-vo-keyboard", "no".into()),
        ("osc", "no".into()),
        ("osd-level", "0".into()),
        ("idle", "yes".into()),
        ("force-window", "no".into()),
        ("keep-open", "no".into()),
        ("vo", "gpu-next".into()),
        ("gpu-api", "d3d11".into()),
        ("hwdec", "auto-safe".into()),
        ("ao", "wasapi".into()),
        ("audio-exclusive", "no".into()),
        ("gapless-audio", "yes".into()),
        ("volume-max", "150".into()),
        ("sub-auto", "fuzzy".into()),
        ("audio-file-auto", "fuzzy".into()),
        ("demuxer-max-bytes", "256MiB".into()),
        ("demuxer-readahead-secs", "30".into()),
        ("screenshot-format", "png".into()),
        ("tone-mapping", "auto".into()),
        ("ytdl", "no".into()),
        // 网络流：默认缓存策略在直播上容易卡顿，这里显式打开并放宽超时
        ("cache", "yes".into()),
        ("network-timeout", "30".into()),
    ];
    if surface != 0 {
        opts.push(("wid", surface.to_string()));
    } else {
        opts.push(("vo", "null".into()));
        opts.push(("force-window", "no".into()));
    }
    opts
}

fn event_loop(inner: Arc<Inner>, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let Some(ev) = inner.mpv.wait_event(0.2) else {
            continue;
        };
        let id = ev.event_id;
        match id {
            sys::EVENT_SHUTDOWN => break,
            sys::EVENT_FILE_LOADED => {
                let audio = inner.tracks();
                let mut s = state.lock().unwrap();
                s.loaded = true;
                s.eof = false;
                s.error = None;
                s.playing = !inner.mpv.get_flag("pause").unwrap_or(false);
                s.duration = inner.mpv.get_f64("duration").unwrap_or(0.0);
                s.position = inner.mpv.get_f64("time-pos").unwrap_or(0.0);
                s.width = inner.mpv.get_i64("video-params/w").unwrap_or(0).max(0) as u32;
                s.height = inner.mpv.get_i64("video-params/h").unwrap_or(0).max(0) as u32;
                s.vcodec = inner.mpv.get_string("video-codec").unwrap_or_default();
                s.acodec = inner.mpv.get_string("audio-codec-name").unwrap_or_default();
                s.container = inner.mpv.get_string("file-format").unwrap_or_default();
                s.hwid = inner.mpv.get_string("hwdec-current").unwrap_or_default();
                s.audio_tracks = audio.0;
                s.sub_tracks = audio.1;
                s.audio_id = s.audio_tracks.iter().find(|t| t.selected).map(|t| t.id);
                s.sub_id = s.sub_tracks.iter().find(|t| t.selected).map(|t| t.id);
                if let Some(t) = inner.mpv.get_string("media-title") {
                    if !t.is_empty() {
                        s.title = t;
                    }
                }
            }
            sys::EVENT_END_FILE => {
                let end = unsafe { &*(ev.data as *const sys::MpvEventEndFile) };
                let mut s = state.lock().unwrap();
                s.playing = false;
                s.eof = end.reason == 0;
                if end.reason == 4 {
                    s.error = Some("文件无法播放（格式或编码不受支持）".into());
                }
            }
            sys::EVENT_PROPERTY_CHANGE => {
                let prop = unsafe { &*(ev.data as *const sys::MpvEventProperty) };
                if prop.name.is_null() || prop.data.is_null() {
                    continue;
                }
                let name = unsafe { std::ffi::CStr::from_ptr(prop.name) }
                    .to_string_lossy()
                    .into_owned();
                let mut s = state.lock().unwrap();
                unsafe {
                    match (name.as_str(), prop.format) {
                        ("time-pos", sys::FORMAT_DOUBLE) => {
                            s.position = *(prop.data as *const f64);
                        }
                        ("duration", sys::FORMAT_DOUBLE) => {
                            s.duration = *(prop.data as *const f64);
                        }
                        ("volume", sys::FORMAT_DOUBLE) => {
                            s.volume = *(prop.data as *const f64);
                        }
                        ("speed", sys::FORMAT_DOUBLE) => {
                            s.speed = *(prop.data as *const f64);
                        }
                        ("pause", sys::FORMAT_FLAG) => {
                            s.playing = *(prop.data as *const i32) == 0;
                        }
                        ("mute", sys::FORMAT_FLAG) => {
                            s.muted = *(prop.data as *const i32) != 0;
                        }
                        ("media-title", sys::FORMAT_STRING) => {
                            let p = *(prop.data as *const *const std::ffi::c_char);
                            if !p.is_null() {
                                s.title = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}
