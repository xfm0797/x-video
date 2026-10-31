//! mpv 子进程与 JSON IPC 通道。
//!
//! mpv 通过 `--input-ipc-server` 暴露一个命名管道（Windows）或 Unix socket，
//! 每行一个 JSON：命令走 `{"command":[...]}`，事件由 mpv 主动推送。

use crate::install::{bin_name, MpvBinary};
use crate::surface::Surface;
use crate::MpvState;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type LogBuf = Arc<Mutex<VecDeque<String>>>;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// IPC 端点：进程内唯一即可
pub fn endpoint() -> String {
    let pid = std::process::id();
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\xvideo-mpv-{pid}")
    }
    #[cfg(not(windows))]
    {
        let dir = xv_core::store::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("mpv-{pid}.sock")).display().to_string()
    }
}

pub fn new_log() -> LogBuf {
    Arc::new(Mutex::new(VecDeque::with_capacity(64)))
}

pub fn log_tail(log: &LogBuf, n: usize) -> String {
    let buf = log.lock().unwrap();
    buf.iter()
        .rev()
        .take(n)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ")
}

pub struct Ipc {
    writer: Mutex<Box<dyn Write + Send>>,
}

impl Ipc {
    pub fn send(&self, value: &Value) -> Result<()> {
        let mut line = serde_json::to_string(value)?;
        line.push('\n');
        let mut w = self.writer.lock().unwrap();
        w.write_all(line.as_bytes())?;
        w.flush()?;
        Ok(())
    }

    pub fn command(&self, args: Vec<Value>) -> Result<()> {
        self.send(&json!({ "command": args }))
    }
}

fn build_args(surface: &Surface, title: &str, endpoint: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--idle=yes".into(),
        "--no-terminal".into(),
        "--no-config".into(),
        "--force-window=no".into(),
        "--osc=no".into(),
        "--osd-level=0".into(),
        "--input-default-bindings=no".into(),
        "--input-vo-keyboard=no".into(),
        "--keep-open=no".into(),
        "--cache=yes".into(),
        "--demuxer-max-bytes=128MiB".into(),
        "--demuxer-max-back-bytes=64MiB".into(),
        "--hwdec=auto-safe".into(),
        "--vo=gpu".into(),
        "--volume-max=200".into(),
        "--audio-client-name=X-Video".into(),
        format!("--user-agent={UA}"),
        format!("--input-ipc-server={endpoint}"),
        format!("--title={title}"),
    ];
    if let Some(dir) = log_file() {
        args.push(format!("--log-file={}", dir.display()));
    }
    if let Some(wid) = surface.wid_arg() {
        args.push(wid);
    }
    args.extend(surface.extra_args());
    args
}

fn log_file() -> Option<std::path::PathBuf> {
    let dir = xv_core::store::data_dir().join("mpv");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("mpv.log"))
}

pub fn spawn(
    bin: &MpvBinary,
    surface: &Surface,
    title: &str,
    endpoint: &str,
    state: Arc<Mutex<MpvState>>,
    log: LogBuf,
) -> Result<(Child, Ipc)> {
    #[cfg(not(windows))]
    let _ = std::fs::remove_file(endpoint);

    let mut cmd = Command::new(&bin.path);
    cmd.args(build_args(surface, title, endpoint))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：不要给 mpv 弹出控制台
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("启动 {} 失败", bin_name()))?;

    if let Some(stderr) = child.stderr.take() {
        let log = log.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(|l| l.ok()) {
                let text = line.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let mut buf = log.lock().unwrap();
                if buf.len() >= 64 {
                    buf.pop_front();
                }
                buf.push_back(text);
            }
        });
    }

    // 等管道/socket 就绪
    let deadline = Instant::now() + Duration::from_secs(8);
    let (reader, writer) = loop {
        match open_conn(endpoint) {
            Ok(pair) => break pair,
            Err(e) => {
                if let Ok(Some(status)) = child.try_wait() {
                    let tail = log_tail(&log, 3);
                    return Err(anyhow!(
                        "mpv 启动后立即退出（{status}）{}{}",
                        if tail.is_empty() { "" } else { "：" },
                        tail
                    ));
                }
                if Instant::now() > deadline {
                    let tail = log_tail(&log, 3);
                    return Err(anyhow!(
                        "连接 mpv IPC 超时：{e}{}{}",
                        if tail.is_empty() { "" } else { "：" },
                        tail
                    ));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
        }
    };

    let ipc = Ipc {
        writer: Mutex::new(writer),
    };

    {
        let st = state.clone();
        std::thread::spawn(move || read_loop(reader, st));
    }

    for (id, name) in [
        (1, "time-pos"),
        (2, "duration"),
        (3, "pause"),
        (4, "volume"),
        (5, "mute"),
        (6, "media-title"),
        (7, "idle-active"),
    ] {
        ipc.send(&json!({ "command": ["observe_property", id, name] }))?;
    }

    Ok((child, ipc))
}

type Halves = (Box<dyn BufRead + Send>, Box<dyn Write + Send>);

fn open_conn(endpoint: &str) -> std::io::Result<Halves> {
    #[cfg(windows)]
    {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint)?;
        let writer = file.try_clone()?;
        Ok((
            Box::new(BufReader::new(file)) as Box<dyn BufRead + Send>,
            Box::new(writer) as Box<dyn Write + Send>,
        ))
    }
    #[cfg(not(windows))]
    {
        let stream = std::os::unix::net::UnixStream::connect(endpoint)?;
        let writer = stream.try_clone()?;
        Ok((
            Box::new(BufReader::new(stream)) as Box<dyn BufRead + Send>,
            Box::new(writer) as Box<dyn Write + Send>,
        ))
    }
}

fn read_loop(reader: Box<dyn BufRead + Send>, state: Arc<Mutex<MpvState>>) {
    let mut reader = reader;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let text = line.trim();
                if text.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(text) {
                    apply_event(&state, &v);
                }
            }
            Err(_) => break,
        }
    }
    let mut st = state.lock().unwrap();
    let was_running = st.running;
    st.running = false;
    st.idle = true;
    // 主动 close 时状态已被重置，不必报错
    if was_running && st.error.is_none() {
        st.error = Some("mpv 内核连接已断开".to_string());
    }
}

fn apply_event(state: &Arc<Mutex<MpvState>>, v: &Value) {
    match v["event"].as_str() {
        Some("property-change") => {
            let name = v["name"].as_str().unwrap_or("");
            let data = &v["data"];
            let mut st = state.lock().unwrap();
            match name {
                "time-pos" => st.position = data.as_f64().unwrap_or(0.0),
                "duration" => st.duration = data.as_f64().unwrap_or(st.duration),
                "pause" => st.paused = data.as_bool().unwrap_or(false),
                "volume" => st.volume = data.as_f64().unwrap_or(100.0),
                "mute" => st.mute = data.as_bool().unwrap_or(false),
                "media-title" => {
                    if let Some(t) = data.as_str() {
                        st.title = t.to_string();
                    }
                }
                "idle-active" => st.idle = data.as_bool().unwrap_or(false),
                _ => {}
            }
        }
        Some("start-file") => {
            let mut st = state.lock().unwrap();
            st.eof = false;
            st.position = 0.0;
            st.error = None;
        }
        Some("end-file") => {
            let reason = v["reason"].as_str().unwrap_or("");
            let mut st = state.lock().unwrap();
            st.eof = reason == "eof";
            st.paused = true;
            if reason == "error" {
                st.error = Some("该地址无法播放（网络或格式不支持）".to_string());
            }
        }
        _ => {}
    }
}
