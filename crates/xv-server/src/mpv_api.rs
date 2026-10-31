//! mpv 内核控制接口。
//!
//! 内核实现是阻塞式的（子进程 + 管道读写），这里统一丢到 `spawn_blocking`，
//! 避免阻塞 axum 的异步执行器。

use crate::error_response;
use axum::{
    extract::Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};
use xv_mpv::{engine, OpenRequest, Rect, SurfaceMode};

async fn run<T, F>(f: F) -> Response
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => error_response(StatusCode::BAD_REQUEST, &format!("{e:#}")),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("内核调用异常: {e}"),
        ),
    }
}

pub async fn mpv_status() -> Response {
    run(|| Ok(engine().status())).await
}

pub async fn mpv_probe() -> Response {
    run(|| Ok(engine().probe())).await
}

pub async fn mpv_install() -> Response {
    run(|| engine().install()).await
}

#[derive(Deserialize)]
pub struct PathReq {
    #[serde(default)]
    pub path: String,
}

pub async fn mpv_set_path(Json(req): Json<PathReq>) -> Response {
    run(move || {
        xv_mpv::set_custom_path(&req.path)?;
        Ok(engine().probe())
    })
    .await
}

#[derive(Deserialize)]
pub struct OpenReq {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub rect: Rect,
}

pub async fn mpv_open(Json(req): Json<OpenReq>) -> Response {
    let mode = match req.mode.as_str() {
        "floating" => SurfaceMode::Floating,
        _ => SurfaceMode::Embed,
    };
    let open = OpenRequest {
        url: req.url,
        title: req.title,
        mode,
        rect: req.rect,
    };
    match tokio::task::spawn_blocking(move || engine().open(open)).await {
        Ok(Ok(())) => Json(json!({ "ok": true })).into_response(),
        Ok(Err(e)) => {
            let tail = engine().last_log();
            let msg = if tail.is_empty() {
                format!("{e:#}")
            } else {
                format!("{e:#}（mpv: {tail}）")
            };
            error_response(StatusCode::BAD_REQUEST, &msg)
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("内核调用异常: {e}"),
        ),
    }
}

#[derive(Deserialize)]
pub struct RectReq {
    #[serde(default)]
    pub rect: Rect,
}

pub async fn mpv_rect(Json(req): Json<RectReq>) -> Response {
    run(move || {
        engine().set_rect(req.rect)?;
        Ok(json!({ "ok": true }))
    })
    .await
}

#[derive(Deserialize)]
pub struct CmdReq {
    #[serde(default)]
    pub args: Vec<Value>,
}

pub async fn mpv_cmd(Json(req): Json<CmdReq>) -> Response {
    run(move || {
        engine().command(req.args)?;
        Ok(json!({ "ok": true }))
    })
    .await
}

pub async fn mpv_close() -> Response {
    run(|| {
        engine().close()?;
        Ok(json!({ "ok": true }))
    })
    .await
}
