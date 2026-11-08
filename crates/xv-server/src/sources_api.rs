//! 在线源接口：订阅增删刷新、直播频道、TVBox 搜索/详情/播放地址解析。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::task::JoinSet;

use crate::{ApiError, ApiResult, App};

/// `subid|sitekey`
fn split_site(spec: &str) -> Result<(&str, &str), ApiError> {
    spec.split_once('|')
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .ok_or_else(|| ApiError::msg("站点参数格式应为 订阅ID|站点KEY"))
}

pub async fn subs_list(State(app): State<Arc<App>>) -> ApiResult {
    Ok(Json(json!({ "items": app.sources.subs() })))
}

#[derive(Deserialize)]
pub struct AddReq {
    #[serde(default)]
    name: String,
    url: String,
    #[serde(default)]
    kind: String,
}

pub async fn add(State(app): State<Arc<App>>, Json(req): Json<AddReq>) -> ApiResult {
    let sub = app.sources.add(&req.name, &req.url, &req.kind).await?;
    Ok(Json(json!({
        "sub": sub,
        "items": app.sources.subs(),
    })))
}

#[derive(Deserialize)]
pub struct IdReq {
    id: String,
}

pub async fn remove(State(app): State<Arc<App>>, Json(req): Json<IdReq>) -> ApiResult {
    let removed = app.sources.remove(&req.id);
    Ok(Json(json!({ "removed": removed, "items": app.sources.subs() })))
}

#[derive(Deserialize)]
pub struct ToggleReq {
    id: String,
    enabled: bool,
}

pub async fn toggle(State(app): State<Arc<App>>, Json(req): Json<ToggleReq>) -> ApiResult {
    app.sources.set_enabled(&req.id, req.enabled);
    Ok(Json(json!({ "items": app.sources.subs() })))
}

#[derive(Deserialize)]
pub struct RefreshReq {
    #[serde(default)]
    id: String,
}

pub async fn refresh(State(app): State<Arc<App>>, Json(req): Json<RefreshReq>) -> ApiResult {
    if req.id.is_empty() {
        // 逐条刷新，单条失败不影响其它
        let ids: Vec<String> = app.sources.subs().into_iter().map(|s| s.sub.id).collect();
        for id in ids {
            let _ = app.sources.refresh(&id).await;
        }
    } else {
        app.sources.refresh(&req.id).await?;
    }
    Ok(Json(json!({ "items": app.sources.subs() })))
}

// ---------- 直播 ----------

#[derive(Deserialize)]
pub struct LiveQuery {
    sub: Option<String>,
}

pub async fn live(State(app): State<Arc<App>>, Query(q): Query<LiveQuery>) -> ApiResult {
    let groups = app.sources.live(q.sub.as_deref());
    Ok(Json(json!({ "groups": groups })))
}

#[derive(Deserialize)]
pub struct OpenLiveReq {
    #[serde(default)]
    url: String,
    /// 同名频道的候选地址（IPTV 多源），按顺序尝试。
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    headers: String,
}

pub async fn open_live(State(app): State<Arc<App>>, Json(req): Json<OpenLiveReq>) -> ApiResult {
    let mut urls = req.urls.clone();
    if !req.url.is_empty() {
        urls.insert(0, req.url.clone());
    }
    let headers = req.headers.clone();
    let name = req.name.clone();
    let ok = tokio::task::spawn_blocking(move || app.play_candidates(&urls, &headers))
        .await
        .map_err(|e| ApiError::msg(e.to_string()))??;
    Ok(Json(json!({ "ok": true, "name": name, "source_index": ok })))
}

// ---------- 点播 ----------

pub async fn sites(State(app): State<Arc<App>>) -> ApiResult {
    Ok(Json(json!({ "items": app.sources.sites() })))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    site: Option<String>,
    #[serde(default)]
    limit: usize,
}

/// 跨站搜索：并发打各站 CMS 接口，单站 12 秒超时，先把有结果的站点返回。
pub async fn search(State(app): State<Arc<App>>, Query(q): Query<SearchQuery>) -> ApiResult {
    let wd = q.q.trim().to_string();
    if wd.is_empty() {
        return Ok(Json(json!({ "hits": [], "failed": [] })));
    }

    let targets: Vec<(String, String, String, String)> = match q.site.as_deref() {
        Some(spec) => {
            let (sub, key) = split_site(spec)?;
            let meta = app
                .sources
                .sites()
                .into_iter()
                .find(|s| s.sub_id == sub && s.key == key)
                .map(|s| (s.sub_name, s.name))
                .unwrap_or_default();
            vec![(sub.to_string(), key.to_string(), meta.0, meta.1)]
        }
        None => app
            .sources
            .sites()
            .into_iter()
            .filter(|s| s.supported && s.searchable)
            .take(if q.limit == 0 { 12 } else { q.limit.min(20) })
            .map(|s| (s.sub_id, s.key, s.sub_name, s.name))
            .collect(),
    };

    let mut set: JoinSet<(String, String, String, String, Result<Vec<xv_source::Hit>, String>)> =
        JoinSet::new();
    for (sub_id, key, sub_name, site_name) in targets {
        let hub = Arc::clone(&app.sources);
        let wd = wd.clone();
        set.spawn(async move {
            let res = tokio::time::timeout(
                Duration::from_secs(12),
                hub.search_site(&sub_id, &key, &wd),
            )
            .await
            .map_err(|_| "超时".to_string())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            (sub_id, sub_name, site_name, key, res)
        });
    }

    let mut hits: Vec<Value> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    // 总时限：站点再多也不能让接口长时间挂着——到点收摊，把已完成的结果先回去。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut timed_out = 0usize;
    loop {
        let joined = match tokio::time::timeout_at(deadline, set.join_next()).await {
            Ok(Some(j)) => j,
            Ok(None) => break,
            Err(_) => {
                timed_out = set.len();
                set.abort_all();
                break;
            }
        };
        let Ok((sub_id, sub_name, site_name, key, res)) = joined else {
            continue;
        };
        match res {
            Ok(list) => {
                for h in list.into_iter().take(20) {
                    hits.push(json!({
                        "site": format!("{sub_id}|{key}"),
                        "sub_name": sub_name,
                        "site_name": site_name,
                        "id": h.id,
                        "name": h.name,
                        "pic": h.pic,
                        "remarks": h.remarks,
                        "year": h.year,
                    }));
                }
            }
            Err(e) => failed.push(json!({ "site": site_name, "error": e })),
        }
    }
    if timed_out > 0 {
        failed.push(json!({ "site": "—", "error": format!("{timed_out} 个站点未在 30 秒内返回") }));
    }
    Ok(Json(json!({ "hits": hits, "failed": failed })))
}

#[derive(Deserialize)]
pub struct SiteQuery {
    site: String,
    #[serde(default)]
    tid: String,
    #[serde(default)]
    pg: u32,
}

pub async fn categories(State(app): State<Arc<App>>, Query(q): Query<SiteQuery>) -> ApiResult {
    let (sub, key) = split_site(&q.site)?;
    let items = app.sources.categories(sub, key).await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn vod_list(State(app): State<Arc<App>>, Query(q): Query<SiteQuery>) -> ApiResult {
    let (sub, key) = split_site(&q.site)?;
    let (items, pages) = app
        .sources
        .list(sub, key, &q.tid, q.pg.max(1))
        .await?;
    Ok(Json(json!({ "hits": items, "pages": pages, "pg": q.pg.max(1) })))
}

#[derive(Deserialize)]
pub struct DetailQuery {
    site: String,
    id: String,
}

pub async fn detail(State(app): State<Arc<App>>, Query(q): Query<DetailQuery>) -> ApiResult {
    let (sub, key) = split_site(&q.site)?;
    let detail = app.sources.detail(sub, key, &q.id).await?;
    Ok(Json(json!({ "detail": detail })))
}

#[derive(Deserialize)]
pub struct OpenVodReq {
    site: String,
    url: String,
    #[serde(default)]
    flag: String,
}

pub async fn open_vod(State(app): State<Arc<App>>, Json(req): Json<OpenVodReq>) -> ApiResult {
    let (sub, key) = split_site(&req.site)?;
    let resolved = app.sources.resolve(sub, key, &req.flag, &req.url).await?;
    let headers = xv_source::http::mpv_header_fields(&resolved.headers);
    let url = resolved.url.clone();
    tokio::task::spawn_blocking(move || app.play_url(&url, &headers))
        .await
        .map_err(|e| ApiError::msg(e.to_string()))??;
    Ok(Json(json!({ "ok": true, "url": resolved.url })))
}
