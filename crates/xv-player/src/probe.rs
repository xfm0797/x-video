//! 轻量探测与抽帧：借用 libmpv 自己的解码器，不额外打包 ffprobe。
//!
//! 每次探测建一个临时实例（vo=null / ao=null），读完轨道信息立刻销毁；
//! 抽帧用 `vo=image` 输出单帧 PNG。两者都跑在调用方的后台线程里。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::sys::{self, Api, Mpv};

static API: OnceLock<Option<&'static Api>> = OnceLock::new();

fn api() -> Option<&'static Api> {
    *API.get_or_init(|| {
        match sys::locate().and_then(|p| Api::load(&p)) {
            Ok(a) => Some(Box::leak(Box::new(a)) as &'static Api),
            Err(e) => {
                eprintln!("[xv-player] 内核不可用：{e:#}");
                None
            }
        }
    })
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub vcodec: String,
    pub acodec: String,
    pub container: String,
}

fn base_options() -> Vec<(&'static str, String)> {
    vec![
        ("config", "no".into()),
        ("terminal", "no".into()),
        ("idle", "yes".into()),
        ("pause", "yes".into()),
        ("ao", "null".into()),
        ("vo", "null".into()),
        ("audio-display", "no".into()),
        ("hwdec", "no".into()),
        ("cache", "no".into()),
        ("demuxer-readahead-secs", "1".into()),
        ("ytdl", "no".into()),
        ("osd-level", "0".into()),
    ]
}

fn wait_loaded(mpv: &Mpv, deadline: Instant) -> Result<()> {
    while Instant::now() < deadline {
        let Some(ev) = mpv.wait_event(0.2) else { continue };
        match ev.event_id {
            sys::EVENT_FILE_LOADED => return Ok(()),
            sys::EVENT_END_FILE => {
                return Err(anyhow!("文件解析失败"));
            }
            sys::EVENT_SHUTDOWN => return Err(anyhow!("内核已关闭")),
            _ => {}
        }
    }
    Err(anyhow!("解析超时"))
}

/// 读取容器的时长、分辨率与编解码信息。
pub fn probe_file(path: &Path) -> Result<Probe> {
    let api = api().ok_or_else(|| anyhow!("播放内核未就绪"))?;
    let mpv = Mpv::new(api, &base_options())?;
    let p = path.to_string_lossy().to_string();
    mpv.command(&["loadfile", &p, "replace"])?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let result = wait_loaded(&mpv, deadline);

    let probe = if result.is_ok() {
        Probe {
            duration: mpv.get_f64("duration").unwrap_or(0.0),
            width: mpv.get_i64("video-params/w").unwrap_or(0).max(0) as u32,
            height: mpv.get_i64("video-params/h").unwrap_or(0).max(0) as u32,
            vcodec: mpv.get_string("video-codec").unwrap_or_default(),
            acodec: mpv.get_string("audio-codec-name").unwrap_or_default(),
            container: mpv.get_string("file-format").unwrap_or_default(),
        }
    } else {
        Probe::default()
    };

    mpv.shutdown();
    result.map(|_| probe)
}

/// 抽一帧存成 PNG，返回生成的文件路径。失败时返回 Err，由调用方决定是否忽略。
pub fn thumbnail(path: &Path, secs: f64, out: &Path, width: u32) -> Result<PathBuf> {
    let api = api().ok_or_else(|| anyhow!("播放内核未就绪"))?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let _ = std::fs::remove_file(out);

    let mut opts = base_options();
    opts.retain(|(k, _)| *k != "vo" && *k != "pause");
    opts.extend([
        ("vo", "image".to_string()),
        ("of", "image2".to_string()),
        ("ovc", "png".to_string()),
        ("o", out.to_string_lossy().to_string()),
        ("frames", "1".to_string()),
        ("aid", "no".to_string()),
        ("sid", "no".to_string()),
        ("start", format!("{secs:.2}")),
        ("vf", format!("scale={width}:-2")),
        ("video-sync", "display-resample".to_string()),
    ]);

    let mpv = Mpv::new(api, &opts)?;
    let p = path.to_string_lossy().to_string();
    mpv.command(&["loadfile", &p, "replace"])?;

    let deadline = Instant::now() + Duration::from_secs(25);
    let mut done = false;
    while Instant::now() < deadline {
        let Some(ev) = mpv.wait_event(0.2) else { continue };
        match ev.event_id {
            sys::EVENT_END_FILE => {
                done = true;
                break;
            }
            sys::EVENT_SHUTDOWN => break,
            _ => {}
        }
        if out.is_file() && std::fs::metadata(out).map(|m| m.len() > 0).unwrap_or(false) {
            done = true;
            break;
        }
    }
    mpv.shutdown();

    if done && out.is_file() {
        Ok(out.to_path_buf())
    } else {
        Err(anyhow!("抽帧失败：{}", path.display()))
    }
}
