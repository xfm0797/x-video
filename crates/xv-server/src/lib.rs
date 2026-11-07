//! 本地 HTTP 接口 + 静态页面。
//!
//! 为什么不是 Tauri IPC：窗口加载的是 `http://127.0.0.1:<port>`（这是当初修白屏
//! 的稳定路径），前端同源 fetch 即可，开发时还能直接用浏览器截图调试界面。
//! 只监听回环地址，并校验 Origin，防止其他网页跨站调用。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxPath, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};

use xv_app::{App, PlayerPrefs, Rect};

pub mod sources_api;

pub struct ApiError(String);

impl ApiError {
    pub(crate) fn msg(m: impl Into<String>) -> Self {
        ApiError(m.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(format!("{e:#}"))
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError(format!("{e:#}"))
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError(format!("{e:#}"))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError(format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": self.0 }))).into_response()
    }
}

pub(crate) type ApiResult = Result<Json<Value>, ApiError>;

pub fn router(app: Arc<App>) -> Router {
    let index = app.dist.join("index.html");
    let spa = ServeDir::new(&app.dist).not_found_service(ServeFile::new(index));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route("/api/library/series", get(list_series))
        .route("/api/library/series/{id}", get(series_detail))
        .route("/api/library/continue", get(continue_watching))
        .route("/api/library/recent", get(recent))
        .route("/api/library/roots", get(list_roots).post(add_root))
        .route("/api/library/roots/remove", post(remove_root))
        .route("/api/library/scan", post(scan))
        .route("/api/library/item/{id}/queue", get(item_queue))
        .route("/api/artwork/{kind}/{id}", get(artwork))
        .route("/api/player/state", get(player_state))
        .route("/api/player/open", post(player_open))
        .route("/api/player/cmd", post(player_cmd))
        .route("/api/player/rect", post(player_rect))
        .route("/api/settings", get(get_settings).post(set_settings))
        .route("/api/fs/drives", get(fs_drives))
        .route("/api/fs/list", get(fs_list))
        .route("/api/window/fullscreen", post(window_fullscreen))
        .route(
            "/api/sources",
            get(sources_api::subs_list).post(sources_api::add),
        )
        .route("/api/sources/remove", post(sources_api::remove))
        .route("/api/sources/refresh", post(sources_api::refresh))
        .route("/api/sources/toggle", post(sources_api::toggle))
        .route("/api/live", get(sources_api::live))
        .route("/api/live/open", post(sources_api::open_live))
        .route("/api/vod/sites", get(sources_api::sites))
        .route("/api/vod/search", get(sources_api::search))
        .route("/api/vod/categories", get(sources_api::categories))
        .route("/api/vod/list", get(sources_api::vod_list))
        .route("/api/vod/detail", get(sources_api::detail))
        .route("/api/vod/open", post(sources_api::open_vod))
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(app.clone(), origin_guard))
        .with_state(app)
}

async fn origin_guard(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or("");
        let allowed = [
            format!("http://127.0.0.1:{}", app.port),
            format!("http://localhost:{}", app.port),
            "http://localhost:5173".to_string(),
            "http://127.0.0.1:5173".to_string(),
            "tauri://localhost".to_string(),
            "http://tauri.localhost".to_string(),
        ];
        if !allowed.iter().any(|a| a == origin) {
            return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
        }
    }
    next.run(req).await
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "app": "x-video", "version": env!("CARGO_PKG_VERSION") }))
}

async fn status(State(app): State<Arc<App>>) -> ApiResult {
    let player = app.player();
    let st = player.state();
    let (scanning, last) = app.scan_state();
    let kinds = json!({
        "movie": app.library.series_count(Some("movie")).unwrap_or(0),
        "tv": app.library.series_count(Some("tv")).unwrap_or(0),
        "short": app.library.series_count(Some("short")).unwrap_or(0),
        "music": app.library.series_count(Some("music")).unwrap_or(0),
    });
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "engine": {
            "available": st.available,
            "lib": st.lib_path,
            "error": st.lib_error,
        },
        "counts": kinds,
        "scanning": scanning,
        "last_scan": last,
        "data_dir": app.data_dir.display().to_string(),
        "uptime": app.started.elapsed().as_secs(),
    })))
}

#[derive(Deserialize)]
struct ListQuery {
    kind: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_series(State(app): State<Arc<App>>, Query(q): Query<ListQuery>) -> ApiResult {
    let kind = q.kind.filter(|k| k != "all");
    let rows = app.library.list_series(
        kind.as_deref(),
        q.q.as_deref(),
        q.sort.as_deref().unwrap_or("recent"),
        q.limit.unwrap_or(200),
        q.offset.unwrap_or(0),
    )?;
    let total = app.library.series_count(kind.as_deref()).unwrap_or(0);
    Ok(Json(json!({ "items": rows, "total": total })))
}

async fn series_detail(State(app): State<Arc<App>>, AxPath(id): AxPath<i64>) -> ApiResult {
    let (series, items) = app
        .library
        .series(id)?
        .ok_or_else(|| ApiError("剧集不存在".into()))?;
    Ok(Json(json!({ "series": series, "items": items })))
}

async fn continue_watching(State(app): State<Arc<App>>) -> ApiResult {
    Ok(Json(json!({ "items": app.library.continue_watching(20)? })))
}

async fn recent(State(app): State<Arc<App>>) -> ApiResult {
    Ok(Json(json!({ "items": app.library.recent_items(24)? })))
}

async fn list_roots(State(app): State<Arc<App>>) -> ApiResult {
    Ok(Json(json!({ "items": app.library.roots()? })))
}

#[derive(Deserialize)]
struct RootReq {
    path: String,
}

async fn add_root(State(app): State<Arc<App>>, Json(req): Json<RootReq>) -> ApiResult {
    app.library.add_root(&req.path)?;
    Ok(Json(json!({ "ok": true })))
}

async fn remove_root(State(app): State<Arc<App>>, Json(req): Json<RootReq>) -> ApiResult {
    let n = app.library.remove_root(&req.path)?;
    Ok(Json(json!({ "removed": n })))
}

async fn scan(State(app): State<Arc<App>>, Json(req): Json<RootReq>) -> ApiResult {
    if !Path::new(&req.path).is_dir() {
        return Err(ApiError(format!("目录不存在：{}", req.path)));
    }
    app.start_scan(req.path);
    Ok(Json(json!({ "started": true })))
}

async fn item_queue(State(app): State<Arc<App>>, AxPath(id): AxPath<i64>) -> ApiResult {
    Ok(Json(json!({ "items": app.library.series_of_item(id)? })))
}

// ---------- 封面 ----------

fn mime_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "image/png",
    }
}

async fn serve_image(path: PathBuf) -> Response {
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mime = mime_of(&path);
            (
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
                    (
                        header::CACHE_CONTROL,
                        HeaderValue::from_static("public, max-age=86400"),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn artwork(
    State(app): State<Arc<App>>,
    AxPath((kind, id)): AxPath<(String, i64)>,
) -> Response {
    // series 优先用同目录海报，没有就退到首集缩略图
    let item_id = if kind == "series" {
        match app.library.poster_of(id) {
            Some(p) => {
                let name = p.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                if name.ends_with(".png") && p.to_string_lossy().contains("thumbs") {
                    let raw = name.trim_end_matches(".png");
                    raw.parse::<i64>().unwrap_or(id)
                } else {
                    return serve_image(p).await;
                }
            }
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        id
    };

    let thumb = app.library.thumb_path(item_id);
    if thumb.is_file() {
        return serve_image(thumb).await;
    }

    let Some(item) = app.library.item(item_id).ok().flatten() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let secs = if item.duration > 120.0 {
        item.duration * 0.12
    } else if item.duration > 10.0 {
        item.duration * 0.3
    } else {
        1.0
    };
    let out = thumb.clone();
    let src = PathBuf::from(&item.path);
    let generated = tokio::task::spawn_blocking(move || {
        xv_player::thumbnail(&src, secs, &out, 480).is_ok()
    })
    .await
    .unwrap_or(false);

    if generated {
        serve_image(thumb).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

// ---------- 播放 ----------

async fn player_state(State(app): State<Arc<App>>) -> ApiResult {
    let player = app.player();
    let st = player.state();
    let session = app.session.lock().map(|s| s.clone()).unwrap_or_default();
    Ok(Json(json!({
        "engine": {
            "available": st.available,
            "loaded": st.loaded,
            "playing": st.playing,
            "position": st.position,
            "duration": st.duration,
            "volume": st.volume,
            "muted": st.muted,
            "speed": st.speed,
            "title": st.title,
            "path": st.path,
            "width": st.width,
            "height": st.height,
            "vcodec": st.vcodec,
            "acodec": st.acodec,
            "container": st.container,
            "hwdec": st.hwid,
            "audio_tracks": st.audio_tracks,
            "sub_tracks": st.sub_tracks,
            "audio_id": st.audio_id,
            "sub_id": st.sub_id,
            "eof": st.eof,
            "error": st.error,
        },
        "session": session,
    })))
}

#[derive(Deserialize)]
struct OpenReq {
    item_id: Option<i64>,
    path: Option<String>,
    start: Option<f64>,
}

async fn player_open(State(app): State<Arc<App>>, Json(req): Json<OpenReq>) -> ApiResult {
    let app2 = Arc::clone(&app);
    let result = tokio::task::spawn_blocking(move || match (req.item_id, req.path) {
        (Some(id), _) => app2.play_item(id, req.start),
        (None, Some(p)) => app2.play_path(&p),
        _ => Err(anyhow::anyhow!("缺少 item_id 或 path")),
    })
    .await
    .map_err(|e| ApiError(e.to_string()))?;
    result?;
    // 开播后立刻把音轨/字幕填充好
    app.player().refresh_tracks();
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CmdReq {
    cmd: String,
    value: Option<f64>,
    text: Option<String>,
    id: Option<i64>,
}

async fn player_cmd(State(app): State<Arc<App>>, Json(req): Json<CmdReq>) -> ApiResult {
    let player = app.player();
    match req.cmd.as_str() {
        "toggle" => player.play_pause()?,
        "play" => player.set_pause(false)?,
        "pause" => player.set_pause(true)?,
        "seek" => player.seek(req.value.unwrap_or(0.0), true)?,
        "seek_rel" => player.seek(req.value.unwrap_or(0.0), false)?,
        "volume" => {
            let v = req.value.unwrap_or(100.0);
            player.set_volume(v)?;
            app.update_prefs(|p| p.volume = v);
        }
        "mute" => player.set_mute(req.value.unwrap_or(1.0) > 0.5)?,
        "speed" => {
            let v = req.value.unwrap_or(1.0);
            player.set_speed(v)?;
            app.update_prefs(|p| p.speed = v);
        }
        "next" => app.next(false)?,
        "prev" => app.prev()?,
        "stop" => player.stop()?,
        "audio" => player.set_audio(req.id.unwrap_or(-1))?,
        "sub" => player.set_sub(req.id.unwrap_or(-1))?,
        "sub_add" => player.add_sub(req.text.as_deref().unwrap_or(""))?,
        "sub_delay" => player.set_sub_delay(req.value.unwrap_or(0.0))?,
        "aspect" => player.set_aspect(req.text.as_deref().unwrap_or("no"))?,
        "mode" => {
            let mode = req.text.clone().unwrap_or_else(|| "next".into());
            if let Ok(mut s) = app.session.lock() {
                s.mode = mode.clone();
            }
            app.update_prefs(|p| p.mode = mode);
        }
        other => return Err(ApiError(format!("未知命令：{other}"))),
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct RectReq {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    #[serde(default)]
    visible: bool,
}

async fn player_rect(State(app): State<Arc<App>>, Json(req): Json<RectReq>) -> ApiResult {
    app.set_rect(
        Rect {
            x: req.x.round() as i32,
            y: req.y.round() as i32,
            w: req.w.round() as i32,
            h: req.h.round() as i32,
        },
        req.visible,
    );
    Ok(Json(json!({ "ok": true })))
}

// ---------- 设置 ----------

async fn get_settings(State(app): State<Arc<App>>) -> ApiResult {
    Ok(Json(json!({
        "player": app.prefs(),
        "roots": app.library.roots()?,
    })))
}

#[derive(Deserialize)]
struct SettingsReq {
    player: Option<PlayerPrefs>,
}

async fn set_settings(State(app): State<Arc<App>>, Json(req): Json<SettingsReq>) -> ApiResult {
    if let Some(p) = req.player {
        app.save_prefs(&p)?;
        let player = app.player();
        player.set_volume(p.volume)?;
        player.set_speed(p.speed)?;
    }
    Ok(Json(json!({ "ok": true })))
}

// ---------- 窗口 ----------

#[derive(Deserialize)]
struct FullscreenReq {
    on: bool,
}

async fn window_fullscreen(Json(req): Json<FullscreenReq>) -> ApiResult {
    let hwnd = xv_win::main_hwnd();
    let ok = xv_win::set_fullscreen(hwnd, req.on);
    Ok(Json(json!({ "ok": ok })))
}

// ---------- 文件系统 ----------

async fn fs_drives() -> ApiResult {
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if Path::new(&root).exists() {
            drives.push(json!({ "name": format!("{}:", letter as char), "path": root }));
        }
    }
    Ok(Json(json!({ "items": drives })))
}

#[derive(Deserialize)]
struct FsQuery {
    path: Option<String>,
}

async fn fs_list(Query(q): Query<FsQuery>) -> ApiResult {
    let path = q.path.unwrap_or_else(|| {
        std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string())
    });
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(ApiError(format!("不是目录：{path}")));
    }
    let mut dirs: Vec<Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('$') || name.starts_with('.') {
                continue;
            }
            dirs.push(json!({ "name": name, "path": p.to_string_lossy() }));
        }
    }
    dirs.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });
    let parent = dir
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Json(json!({ "path": path, "parent": parent, "items": dirs })))
}

pub async fn serve(app: Arc<App>, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("[xv] http://127.0.0.1:{port}");

    // 订阅超过 6 小时没刷新就后台更新一次，不影响首屏
    let warm = Arc::clone(&app);
    tokio::spawn(async move {
        warm.sources.refresh_stale(6 * 3600).await;
    });

    axum::serve(listener, router(app)).await?;
    Ok(())
}
