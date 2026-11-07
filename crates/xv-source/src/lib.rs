//! 在线源订阅中心。
//!
//! 一条订阅 = 一个 IPTV 直播清单（m3u/txt）或一份 TVBox 配置（json）。
//! 订阅列表与解析结果都落盘到 `<data>/sources.json`，冷启动可以立刻出内容，
//! 刷新失败也不会把上一次可用的频道列表冲掉。

pub mod extract;
pub mod http;
pub mod live;
pub mod pdf;
pub mod spider;
pub mod tvbox;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

pub use live::{LiveChannel, LiveGroup};
pub use tvbox::{Hit, VodDetail};

const AUTOREFRESH_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sub {
    pub id: String,
    pub name: String,
    pub url: String,
    /// iptv | tvbox
    pub kind: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub added_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub summary: String,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    subs: Vec<Sub>,
    #[serde(default)]
    live: HashMap<String, Vec<LiveGroup>>,
    #[serde(default)]
    configs: HashMap<String, tvbox::TvConfig>,
}

#[derive(Default)]
struct State {
    subs: Vec<Sub>,
    live: HashMap<String, Vec<LiveGroup>>,
    configs: HashMap<String, tvbox::TvConfig>,
    /// 已经加载好的 JS spider（key = `sub_id|site_key`），不落盘。
    spiders: HashMap<String, spider::Spy>,
}

impl State {
    fn to_persisted(&self) -> Persisted {
        Persisted {
            subs: self.subs.clone(),
            live: self.live.clone(),
            configs: self.configs.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubView {
    #[serde(flatten)]
    pub sub: Sub,
    pub groups: usize,
    pub channels: usize,
    pub sites: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveView {
    pub sub_id: String,
    pub sub_name: String,
    pub group: String,
    pub channels: Vec<LiveChannel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteView {
    pub sub_id: String,
    pub sub_name: String,
    pub key: String,
    pub name: String,
    pub kind: i32,
    pub supported: bool,
    pub searchable: bool,
    /// 需要 Java spider 内核（`csp_xxx`），本程序跑不了
    pub needs_java: bool,
}

pub struct Hub {
    path: PathBuf,
    state: Mutex<State>,
    client: reqwest::Client,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 把站点里的 api / ext 相对路径补全（幂等，可重复调用）。
fn normalize_cfg(cfg: &mut tvbox::TvConfig, base: &str) {
    for site in &mut cfg.sites {
        site.api = abs_maybe(base, &site.api);
        if let Some(s) = site.ext.as_str() {
            let abs = abs_maybe(base, s);
            if abs != s {
                site.ext = serde_json::Value::String(abs);
            }
        }
    }
}

impl Hub {
    pub fn open(path: &Path) -> Result<Self> {
        let mut state = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Persisted>(&text) {
                Ok(p) => State {
                    subs: p.subs,
                    live: p.live,
                    configs: p.configs,
                    spiders: HashMap::new(),
                },
                Err(e) => {
                    eprintln!("[sources] 订阅文件损坏，已忽略：{e}");
                    State::default()
                }
            },
            Err(_) => State::default(),
        };
        // 老版本落盘的是相对路径，读出来时按各自的订阅地址补全
        for sub in &state.subs {
            if let Some(cfg) = state.configs.get_mut(&sub.id) {
                normalize_cfg(cfg, &sub.url);
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            state: Mutex::new(state),
            client: http::client(),
        })
    }

    fn save(&self) {
        let snapshot = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.to_persisted()
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string(&snapshot) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&self.path, text) {
                    eprintln!("[sources] 写入订阅文件失败：{e}");
                }
            }
            Err(e) => eprintln!("[sources] 序列化订阅失败：{e}"),
        }
    }

    pub fn subs(&self) -> Vec<SubView> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.subs
            .iter()
            .map(|s| {
                let groups = st.live.get(&s.id).map(|g| g.len()).unwrap_or(0);
                let channels: usize = st
                    .live
                    .get(&s.id)
                    .map(|g| g.iter().map(|x| x.channels.len()).sum())
                    .unwrap_or(0);
                let sites = st.configs.get(&s.id).map(|c| c.sites.len()).unwrap_or(0);
                SubView {
                    sub: s.clone(),
                    groups,
                    channels,
                    sites,
                }
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Sub> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.subs.iter().find(|s| s.id == id).cloned()
    }

    /// 新增订阅；`kind` 传空或 `auto` 时按内容自动判断。
    pub async fn add(&self, name: &str, url: &str, kind: &str) -> Result<Sub> {
        let url = url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(anyhow!("订阅地址要以 http(s):// 开头"));
        }
        // 同一个地址只保留一条，重复添加视为「刷新已有订阅」。
        let dup = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.subs.iter().find(|s| s.url == url).cloned()
        };
        if let Some(existing) = dup {
            let _ = self.refresh(&existing.id).await;
            self.save();
            return Ok(self.get(&existing.id).unwrap_or(existing));
        }
        let kind = match kind {
            "iptv" | "tvbox" => kind.to_string(),
            _ => detect_kind(url),
        };
        let id = format!("s{}-{}", now(), self.state.lock().map(|s| s.subs.len()).unwrap_or(0));
        let sub = Sub {
            id: id.clone(),
            name: if name.trim().is_empty() {
                short_name(url)
            } else {
                name.trim().to_string()
            },
            url: url.to_string(),
            kind,
            enabled: true,
            added_at: now(),
            updated_at: 0,
            error: None,
            summary: String::new(),
        };
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.subs.push(sub.clone());
        }
        let _ = self.refresh(&id).await;
        self.save();
        Ok(self.get(&id).unwrap_or(sub))
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let before = st.subs.len();
        st.subs.retain(|s| s.id != id);
        st.live.remove(id);
        st.configs.remove(id);
        st.spiders.retain(|k, _| !k.starts_with(&format!("{id}|")));
        let removed = st.subs.len() != before;
        drop(st);
        if removed {
            self.save();
        }
        removed
    }

    pub fn set_enabled(&self, id: &str, on: bool) {
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = st.subs.iter_mut().find(|s| s.id == id) {
                s.enabled = on;
            }
        }
        self.save();
    }

    pub async fn refresh_all(&self) {
        let ids: Vec<String> = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.subs
                .iter()
                .filter(|s| s.enabled)
                .take(AUTOREFRESH_LIMIT)
                .map(|s| s.id.clone())
                .collect()
        };
        for id in ids {
            let _ = self.refresh(&id).await;
        }
        self.save();
    }

    /// 只刷新「从未成功过」或「超过 max_age 秒」的订阅。
    pub async fn refresh_stale(&self, max_age: i64) {
        let ids: Vec<String> = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let t = now();
            st.subs
                .iter()
                .filter(|s| s.enabled && (s.updated_at == 0 || t - s.updated_at > max_age))
                .map(|s| s.id.clone())
                .collect()
        };
        for id in ids {
            let _ = self.refresh(&id).await;
        }
    }

    /// 拉取一条订阅。失败只记录 error，不清空上次的缓存。
    pub async fn refresh(&self, id: &str) -> Result<()> {
        let Some(sub) = self.get(id) else {
            return Err(anyhow!("订阅不存在"));
        };
        let result = self.fetch_sub(&sub).await;
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // 配置变了，之前加载的 spider 规则也要失效
            st.spiders.retain(|k, _| !k.starts_with(&format!("{id}|")));
            if let Some(s) = st.subs.iter_mut().find(|s| s.id == id) {
                s.updated_at = now();
                match &result {
                    Ok(Fetched::Live(groups)) => {
                        let n: usize = groups.iter().map(|g| g.channels.len()).sum();
                        s.kind = "iptv".into();
                        s.summary = format!("{} 组 · {} 频道", groups.len(), n);
                        s.error = None;
                        st.live.insert(id.to_string(), groups.clone());
                    }
                    Ok(Fetched::Tv(cfg)) => {
                        s.kind = "tvbox".into();
                        s.summary = format!("{} 站点 · {} 直播分组", cfg.sites.len(), cfg.lives.len());
                        s.error = None;
                        st.configs.insert(id.to_string(), cfg.clone());
                    }
                    Err(e) => s.error = Some(format!("{e:#}")),
                }
            }
        }
        self.save();
        result.map(|_| ())
    }

    async fn fetch_sub(&self, sub: &Sub) -> Result<Fetched> {
        let fetched = http::fetch_text(&self.client, &sub.url, None).await?;
        let text = fetched.text;
        let looks_json = text.trim_start().starts_with('{');
        match sub.kind.as_str() {
            "tvbox" => self.load_tvbox(&text, sub).await.map(Fetched::Tv),
            "iptv" => Ok(Fetched::Live(live::parse_auto(&text))),
            _ => {
                if looks_json {
                    self.load_tvbox(&text, sub).await.map(Fetched::Tv)
                } else {
                    Ok(Fetched::Live(live::parse_auto(&text)))
                }
            }
        }
    }

    async fn load_tvbox(&self, text: &str, sub: &Sub) -> Result<tvbox::TvConfig> {
        let mut cfg = tvbox::parse_config(text)?;
        normalize_cfg(&mut cfg, &sub.url);
        // 顺带把配置里的 lives 拉下来，直播页可以直接用
        let mut sources: Vec<(String, String)> = Vec::new();
        for entry in &cfg.lives {
            for piece in entry.url.split('#') {
                // lives 里的地址可能是 proxy:// 包装或相对路径
                let real = live::unwrap_proxy(piece.trim());
                let abs = http::join(&sub.url, &real);
                if abs.contains("://") {
                    sources.push((entry.name.clone(), abs));
                }
            }
        }
        if !sources.is_empty() {
            let groups = live::load(&self.client, &sources, None).await;
            if !groups.is_empty() {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                st.live.insert(sub.id.clone(), groups);
            }
        }
        Ok(cfg)
    }

    pub fn live(&self, sub: Option<&str>) -> Vec<LiveView> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for s in st.subs.iter().filter(|s| s.enabled) {
            if let Some(want) = sub {
                if s.id != want {
                    continue;
                }
            }
            let groups = st.live.get(&s.id).cloned().unwrap_or_default();
            for g in groups {
                if g.channels.is_empty() {
                    continue;
                }
                out.push(LiveView {
                    sub_id: s.id.clone(),
                    sub_name: s.name.clone(),
                    group: g.name,
                    channels: g.channels,
                });
            }
        }
        out
    }

    pub fn sites(&self) -> Vec<SiteView> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for s in st.subs.iter().filter(|s| s.enabled) {
            let Some(cfg) = st.configs.get(&s.id) else { continue };
            for site in &cfg.sites {
                out.push(SiteView {
                    sub_id: s.id.clone(),
                    sub_name: s.name.clone(),
                    key: site.key.clone(),
                    name: site.name.clone(),
                    kind: site.kind,
                    supported: site.supported(),
                    searchable: site.searchable(),
                    needs_java: site.needs_java_spider(),
                });
            }
        }
        out
    }

    fn site_of(&self, sub_id: &str, key: &str) -> Result<(tvbox::Site, Vec<tvbox::Parse>)> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = st
            .configs
            .get(sub_id)
            .ok_or_else(|| anyhow!("订阅没有 TVBox 配置"))?;
        let site = cfg
            .sites
            .iter()
            .find(|s| s.key == key)
            .cloned()
            .ok_or_else(|| anyhow!("站点不存在：{key}"))?;
        Ok((site, cfg.parses.clone()))
    }

    /// 取（或懒加载）某个站点的 JS spider。
    async fn spy(&self, sub_id: &str, site: &tvbox::Site) -> Result<spider::Spy> {
        let cache_key = format!("{sub_id}|{}", site.key);
        if let Some(found) = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.spiders.get(&cache_key).cloned()
        } {
            return Ok(found);
        }
        let script_url = site
            .script_url()
            .ok_or_else(|| anyhow!("该站点需要 Java spider 内核，暂不支持"))?;
        let fetched = http::fetch_text(&self.client, &script_url, Some(&script_url)).await?;
        let spy = spider::Spy::load(&fetched.text, &fetched.final_url, &site.headers())?;
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.spiders.insert(cache_key, spy.clone());
        }
        Ok(spy)
    }

    pub async fn search_site(&self, sub_id: &str, key: &str, wd: &str) -> Result<Vec<Hit>> {
        let (site, _) = self.site_of(sub_id, key)?;
        if !site.supported() {
            return Err(anyhow!("该站点需要 Java spider 内核，暂不支持"));
        }
        if let Some(_) = site.script_url() {
            let spy = self.spy(sub_id, &site).await?;
            let wd = wd.to_string();
            let hits = tokio::task::spawn_blocking(move || spy.search(&wd, 1))
                .await
                .map_err(|e| anyhow!("spider 执行被打断：{e}"))??;
            return Ok(hits);
        }
        tvbox::search(&self.client, &site, wd).await
    }

    pub async fn detail(&self, sub_id: &str, key: &str, vod_id: &str) -> Result<VodDetail> {
        let (site, _) = self.site_of(sub_id, key)?;
        if site.script_url().is_some() {
            let spy = self.spy(sub_id, &site).await?;
            let id = vod_id.to_string();
            return tokio::task::spawn_blocking(move || spy.detail(&id))
                .await
                .map_err(|e| anyhow!("spider 执行被打断：{e}"))?;
        }
        tvbox::detail(&self.client, &site, vod_id).await
    }

    pub async fn resolve(
        &self,
        sub_id: &str,
        key: &str,
        flag: &str,
        url: &str,
    ) -> Result<tvbox::Resolved> {
        let (site, parses) = self.site_of(sub_id, key)?;
        // spider 站先按规则的 lazy 转一遍，多数能直接拿到 m3u8
        if site.script_url().is_some() {
            if let Ok(spy) = self.spy(sub_id, &site).await {
                let target = url.to_string();
                let played = tokio::task::spawn_blocking(move || spy.play(&target)).await;
                if let Ok(Ok((real, headers))) = played {
                    if tvbox::is_media_url(&real) {
                        let mut pairs = site.headers();
                        if let Some(text) = headers {
                            if let Some(v) = http::parse_json(&text) {
                                pairs.extend(http::header_pairs(&v));
                            }
                        }
                        return Ok(tvbox::Resolved { url: real, headers: pairs });
                    }
                }
            }
        }
        tvbox::resolve(&self.client, &site, &parses, flag, url).await
    }

    pub async fn categories(&self, sub_id: &str, key: &str) -> Result<Vec<tvbox::Category>> {
        let (site, _) = self.site_of(sub_id, key)?;
        if site.script_url().is_some() {
            // JS spider 的分类来自规则里的 class_name / class_url
            let spy = self.spy(sub_id, &site).await?;
            let names: Vec<&str> = spy.rule.class_name.split(',').filter(|x| !x.trim().is_empty()).collect();
            let ids: Vec<&str> = spy.rule.class_url.split(',').collect();
            return Ok(names
                .iter()
                .enumerate()
                .map(|(i, n)| tvbox::Category {
                    id: ids.get(i).map(|x| x.trim().to_string()).unwrap_or_default(),
                    name: n.trim().to_string(),
                })
                .collect());
        }
        tvbox::categories(&self.client, &site).await
    }

    pub async fn list(
        &self,
        sub_id: &str,
        key: &str,
        tid: &str,
        page: u32,
    ) -> Result<(Vec<Hit>, u32)> {
        let (site, _) = self.site_of(sub_id, key)?;
        if site.script_url().is_some() {
            let spy = self.spy(sub_id, &site).await?;
            let tid = tid.to_string();
            let hits = tokio::task::spawn_blocking(move || spy.list(&tid, page))
                .await
                .map_err(|e| anyhow!("spider 执行被打断：{e}"))??;
            return Ok((hits, page + 1));
        }
        tvbox::list(&self.client, &site, tid, page).await
    }
}

enum Fetched {
    Live(Vec<LiveGroup>),
    Tv(tvbox::TvConfig),
}

/// 把 `./lib/x.js`、`/json/a.json` 这类相对路径补成绝对地址；
/// 像 `csp_Douban` 这种名字原样返回。
fn abs_maybe(base: &str, value: &str) -> String {
    let v = value.trim();
    if v.is_empty() || v.contains("://") {
        return v.to_string();
    }
    if v.starts_with("csp_") || v.starts_with("js:") || v.starts_with("py:") {
        return v.to_string();
    }
    if v.starts_with('.') || v.starts_with('/') || v.contains(".js") || v.contains(".json") {
        return http::join(base, v);
    }
    v.to_string()
}

fn detect_kind(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".json") || lower.contains("tvbox") || lower.contains("api.json") {
        "tvbox".into()
    } else {
        "iptv".into()
    }
}

fn short_name(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or(url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("xvsrc-{}", now()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sources.json");
        let hub = Hub::open(&path).unwrap();
        {
            let mut st = hub.state.lock().unwrap();
            st.subs.push(Sub {
                id: "a".into(),
                name: "测试".into(),
                url: "http://a/x.m3u".into(),
                kind: "iptv".into(),
                enabled: true,
                added_at: 1,
                updated_at: 2,
                error: None,
                summary: "1 组 · 2 频道".into(),
            });
            st.live.insert(
                "a".into(),
                vec![LiveGroup {
                    name: "央视".into(),
                    channels: vec![LiveChannel {
                        name: "CCTV-1".into(),
                        url: "http://a/1.m3u8".into(),
                        ..Default::default()
                    }],
                }],
            );
        }
        hub.save();
        let again = Hub::open(&path).unwrap();
        let subs = again.subs();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].channels, 1);
        let live = again.live(None);
        assert_eq!(live[0].group, "央视");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_kind() {
        assert_eq!(detect_kind("https://a/tvbox.json"), "tvbox");
        assert_eq!(detect_kind("https://a/iptv.m3u"), "iptv");
    }
}
