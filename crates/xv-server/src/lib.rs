use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use xv_core::{proxy, Core};

#[derive(Clone)]
struct AppState {
    core: Arc<Core>,
}


/// 启动 HTTP 服务（API + 代理 + 静态托管），由 bin 与 Tauri 壳共用
pub async fn serve(port: u16) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    serve_with(listener, port).await
}

/// 复用外部已绑定的 std 监听器（Tauri 壳先同步 bind，保证端口就绪后再建窗口）
pub async fn serve_on(listener: std::net::TcpListener, port: u16) -> Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    serve_with(listener, port).await
}

pub async fn serve_with(listener: tokio::net::TcpListener, port: u16) -> Result<()> {
    let proxy_base = format!("http://127.0.0.1:{port}");
    let core = Arc::new(Core::new(proxy_base));
    let state = AppState { core };

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/sources", get(list_sources).post(add_source))
        .route("/api/sources/{id}", delete(delete_source))
        .route("/api/sources/{id}/refresh", post(refresh_source))
        .route("/api/live/{id}", get(get_live))
        .route("/api/play/live", get(play_live))
        .route("/api/tvbox/{id}/sites", get(tvbox_sites))
        .route("/api/cms/{id}/{site}/home", get(cms_home))
        .route("/api/cms/{id}/{site}/list", get(cms_list))
        .route("/api/cms/{id}/{site}/detail", get(cms_detail))
        .route("/api/cms/{id}/{site}/search", get(cms_search))
        .route("/api/cms/{id}/{site}/play", get(cms_play))
        .route("/p", get(proxy_get))
        .fallback(get(static_files))
        .layer(CorsLayer::very_permissive())
        .with_state(state);

    println!("X-Video 服务已启动: http://127.0.0.1:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------- 基础 ----------

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true, "name": "x-video", "version": env!("CARGO_PKG_VERSION") }))
}

// ---------- 源管理 ----------

async fn list_sources(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.core.list_sources())
}

#[derive(Deserialize)]
struct AddSourceReq {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    url: String,
}

async fn add_source(
    State(state): State<AppState>,
    Json(req): Json<AddSourceReq>,
) -> Response {
    let kind = match req.kind.as_str() {
        "m3u" => xv_core::model::SourceKind::M3u,
        "txt" => xv_core::model::SourceKind::Txt,
        "tvbox" => xv_core::model::SourceKind::Tvbox,
        other => {
            return error_response(StatusCode::BAD_REQUEST, &format!("未知源类型: {other}"));
        }
    };
    if !req.url.contains("://") {
        return error_response(StatusCode::BAD_REQUEST, "URL 格式无效");
    }
    match state.core.add_source(&req.name, kind, &req.url).await {
        Ok(source) => (StatusCode::OK, Json(source)).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

async fn delete_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.core.delete_source(&id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn refresh_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.core.refresh_source(&id).await {
        Ok(source) => Json(source).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---------- 直播 ----------

async fn get_live(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.core.get_live(&id).await {
        Ok(groups) => Json(groups).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct PlayLiveReq {
    u: String,
}

async fn play_live(State(state): State<AppState>, Query(q): Query<PlayLiveReq>) -> Response {
    match state.core.resolve_live_url(&q.u) {
        Ok(play) => Json(play).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---------- TVBox ----------

async fn tvbox_sites(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.core.get_tvbox(&id) {
        Ok(cfg) => Json(json!({
            "sites": cfg.sites,
            "lives": cfg.lives,
            "spider": cfg.spider,
        }))
        .into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---------- CMS 点播 ----------

async fn cms_home(
    State(state): State<AppState>,
    Path((id, site)): Path<(String, String)>,
) -> Response {
    match state.core.cms_home(&id, &site).await {
        Ok(cats) => Json(cats).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct ListReq {
    tid: String,
    #[serde(default)]
    pg: Option<u32>,
}

async fn cms_list(
    State(state): State<AppState>,
    Path((id, site)): Path<(String, String)>,
    Query(q): Query<ListReq>,
) -> Response {
    let pg = q.pg.unwrap_or(1).max(1);
    match state.core.cms_list(&id, &site, &q.tid, pg).await {
        Ok((items, page_count, total)) => {
            Json(json!({"list": items, "pagecount": page_count, "total": total, "pg": pg}))
                .into_response()
        }
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct DetailReq {
    vod_id: String,
}

async fn cms_detail(
    State(state): State<AppState>,
    Path((id, site)): Path<(String, String)>,
    Query(q): Query<DetailReq>,
) -> Response {
    match state.core.cms_detail(&id, &site, &q.vod_id).await {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct SearchReq {
    wd: String,
}

async fn cms_search(
    State(state): State<AppState>,
    Path((id, site)): Path<(String, String)>,
    Query(q): Query<SearchReq>,
) -> Response {
    match state.core.cms_search(&id, &site, &q.wd).await {
        Ok(items) => Json(items).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct PlayReq {
    url: String,
}

async fn cms_play(
    State(state): State<AppState>,
    Path((id, site)): Path<(String, String)>,
    Query(q): Query<PlayReq>,
) -> Response {
    match state.core.resolve_play_url(&id, &site, &q.url) {
        Ok(play) => Json(play).into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---------- 本地代理 ----------

#[derive(Deserialize)]
struct ProxyReq {
    u: String,
    #[serde(default)]
    h: String,
}

async fn proxy_get(State(state): State<AppState>, Query(q): Query<ProxyReq>) -> Response {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build();
    let client = match client {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut rb = client.get(&q.u).header("User-Agent", proxy::DEFAULT_UA);
    let extra = proxy::decode_headers(&q.h);
    for (k, v) in extra.iter() {
        rb = rb.header(k, v);
    }
    let resp = match rb.send().await {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &format!("代理请求失败: {e}")),
    };

    let final_url = resp.url().to_string();
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // m3u8：缓冲文本并重写内部地址
    if proxy::is_m3u8(&final_url, &content_type) || proxy::is_m3u8(&q.u, &content_type) {
        match resp.bytes().await {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let rewritten =
                    proxy::rewrite_m3u8(&text, &final_url, &state.core.proxy_base, None);
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
                    rewritten,
                )
                    .into_response();
            }
            Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e.to_string()),
        }
    }

    // 其他流：透传
    let mut headers = HeaderMap::new();
    if !content_type.is_empty() {
        headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    }
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    if let Some(len) = resp.content_length() {
        headers.insert(header::CONTENT_LENGTH, len.to_string().parse().unwrap());
    }
    let stream = resp.bytes_stream();
    Response::builder()
        .status(status)
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "流转发失败"))
}

// ---------- 静态托管（前端 dist） ----------

async fn static_files(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let root = std::env::var("XVIDEO_STATIC_DIR").unwrap_or_else(|_| "../dist".into());
    let full = std::path::Path::new(&root).join(path);
    let full = if full.exists() {
        full
    } else {
        // SPA 回退
        std::path::Path::new(&root).join("index.html")
    };
    match tokio::fs::read(&full).await {
        Ok(bytes) => {
            let mime = mime_of(&full);
            ([(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(_) => error_response(StatusCode::NOT_FOUND, "资源不存在（请先 npm run build）"),
    }
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn error_response(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}
