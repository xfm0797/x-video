use crate::model::*;
use crate::sources::{jx, live_parse, tvbox as tvbox_mod};
use crate::store;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::Mutex;

/// jx 解析结果缓存有效期（秒）
const JX_CACHE_TTL: u64 = 600;
/// 单个解析接口的超时（秒）
const JX_STEP_TIMEOUT: u64 = 15;

pub struct Core {
    pub http: reqwest::Client,
    /// 全部源配置（持久化于 sources.json）
    pub sources: Mutex<Vec<Source>>,
    /// source_id -> 已解析直播分组
    pub live_cache: Mutex<HashMap<String, Vec<LiveGroup>>>,
    /// source_id -> 已解析 TVBox 配置
    pub tvbox_cache: Mutex<HashMap<String, TvBoxConfig>>,
    /// 原始播放页地址 -> (解析后的直链, 时间戳)
    pub jx_cache: Mutex<HashMap<String, (String, u64)>>,
    /// 本地代理基地址，如 http://127.0.0.1:21600
    pub proxy_base: String,
}

impl Core {
    pub fn new(proxy_base: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(tvbox_mod::DEFAULT_UA)
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .expect("构建 HTTP 客户端失败");
        let sources = store::load_sources();
        Self {
            http,
            sources: Mutex::new(sources),
            live_cache: Mutex::new(HashMap::new()),
            tvbox_cache: Mutex::new(HashMap::new()),
            jx_cache: Mutex::new(HashMap::new()),
            proxy_base,
        }
    }

    async fn fetch_text(&self, url: &str, headers: Option<&serde_json::Value>) -> Result<String> {
        let mut rb = self.http.get(url);
        if let Some(h) = headers {
            if let Some(obj) = h.as_object() {
                for (k, v) in obj {
                    if let Some(vs) = v.as_str() {
                        rb = rb.header(k.as_str(), vs);
                    }
                }
            }
        }
        let resp = rb.send().await.with_context(|| format!("请求失败: {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {}: {url}", resp.status()));
        }
        // 自行解码：优先 UTF-8，失败回退 GBK（常见于国内 IPTV 列表）
        let bytes = resp.bytes().await?;
        match String::from_utf8(bytes.to_vec()) {
            Ok(s) => Ok(s),
            Err(e) => {
                let raw = e.into_bytes();
                let (decoded, _, _) = encoding_rs::GBK.decode(&raw);
                Ok(decoded.into_owned())
            }
        }
    }

    pub fn list_sources(&self) -> Vec<Source> {
        self.sources.lock().unwrap().clone()
    }

    fn save(&self) -> Result<()> {
        let sources = self.sources.lock().unwrap().clone();
        store::save_sources(&sources)
    }

    fn update_source<F: FnOnce(&mut Source)>(&self, id: &str, f: F) -> Result<()> {
        let mut sources = self.sources.lock().unwrap();
        let src = sources
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow!("源不存在: {id}"))?;
        f(src);
        drop(sources);
        self.save()
    }

    /// 添加源：拉取并解析校验，失败则报错
    pub async fn add_source(&self, name: &str, kind: SourceKind, url: &str) -> Result<Source> {
        let mut source = Source {
            id: store::new_id(),
            name: name.to_string(),
            kind,
            url: url.to_string(),
            enabled: true,
            added_at: now_secs(),
            channel_count: 0,
            site_count: 0,
            last_error: None,
        };
        match self.refresh_parse(&mut source).await {
            Ok(()) => {}
            Err(e) => return Err(anyhow!("源校验失败: {e}")),
        }
        self.sources.lock().unwrap().push(source.clone());
        self.save()?;
        Ok(source)
    }

    pub fn delete_source(&self, id: &str) -> Result<()> {
        self.sources.lock().unwrap().retain(|s| s.id != id);
        self.live_cache.lock().unwrap().remove(id);
        self.tvbox_cache.lock().unwrap().remove(id);
        self.save()
    }

    /// 重新拉取并解析源内容，更新计数与缓存
    pub async fn refresh_source(&self, id: &str) -> Result<Source> {
        let mut snapshot = {
            let sources = self.sources.lock().unwrap();
            sources
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or_else(|| anyhow!("源不存在: {id}"))?
        };
        self.refresh_parse(&mut snapshot).await?;
        self.update_source(id, |s| {
            s.channel_count = snapshot.channel_count;
            s.site_count = snapshot.site_count;
            s.last_error = None;
        })?;
        Ok(snapshot)
    }

    async fn refresh_parse(&self, source: &mut Source) -> Result<()> {
        let result: Result<()> = async {
            match source.kind {
                SourceKind::M3u | SourceKind::Txt => {
                    let text = self.fetch_text(&source.url, None).await?;
                    let groups = if source.kind == SourceKind::M3u {
                        live_parse::parse_m3u(&text)?
                    } else {
                        live_parse::parse_txt_live(&text)?
                    };
                    let count = groups.iter().map(|g| g.channels.len() as u32).sum();
                    source.channel_count = count;
                    self.live_cache
                        .lock()
                        .unwrap()
                        .insert(source.id.clone(), groups);
                }
                SourceKind::Tvbox => {
                    let text = self.fetch_text(&source.url, None).await?;
                    let cfg = tvbox_mod::parse_config(&text)?;
                    source.site_count = cfg.sites.len() as u32;
                    // 顺带解析 lives 中的 txt/m3u 直播列表（异步顺序拉取）
                    let mut groups: Vec<LiveGroup> = Vec::new();
                    for live in &cfg.lives {
                        let lt = live.live_type.unwrap_or(0);
                        if lt != 0 {
                            continue; // v1 只处理 txt 型直播
                        }
                        for url in live.url.split(';') {
                            let url = url.trim();
                            if url.is_empty() {
                                continue;
                            }
                            if let Ok(text) = self.fetch_text(url, None).await {
                                if text.contains("#EXTINF") {
                                    if let Ok(mut gs) = live_parse::parse_m3u(&text) {
                                        prefix_group_names(&mut gs, &live.name);
                                        groups.append(&mut gs);
                                    }
                                } else if let Ok(mut gs) = live_parse::parse_txt_live(&text) {
                                    prefix_group_names(&mut gs, &live.name);
                                    groups.append(&mut gs);
                                }
                            }
                        }
                    }
                    let count = groups.iter().map(|g| g.channels.len() as u32).sum();
                    source.channel_count = count;
                    self.live_cache
                        .lock()
                        .unwrap()
                        .insert(source.id.clone(), groups);
                    self.tvbox_cache
                        .lock()
                        .unwrap()
                        .insert(source.id.clone(), cfg);
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = &result {
            source.last_error = Some(e.to_string());
        }
        result
    }

    /// 获取直播分组（缓存为空时自动刷新）
    pub async fn get_live(&self, source_id: &str) -> Result<Vec<LiveGroup>> {
        if let Some(g) = self.live_cache.lock().unwrap().get(source_id) {
            if !g.is_empty() {
                return Ok(g.clone());
            }
        }
        self.refresh_source(source_id).await?;
        self.live_cache
            .lock()
            .unwrap()
            .get(source_id)
            .cloned()
            .ok_or_else(|| anyhow!("直播数据为空，请检查源地址"))
    }

    pub fn get_tvbox(&self, source_id: &str) -> Result<TvBoxConfig> {
        self.tvbox_cache
            .lock()
            .unwrap()
            .get(source_id)
            .cloned()
            .ok_or_else(|| anyhow!("TVBox 配置未加载，请先刷新源"))
    }

    fn find_site(&self, source_id: &str, site_key: &str) -> Result<TvBoxSite> {
        let cfg = self.get_tvbox(source_id)?;
        cfg.sites
            .into_iter()
            .find(|s| s.key == site_key)
            .ok_or_else(|| anyhow!("站点不存在: {site_key}"))
    }

    fn cms_client<'a>(&'a self, site: &'a TvBoxSite) -> Result<tvbox_mod::CmsClient<'a>> {
        if site.site_type != 1 {
            return Err(anyhow!(
                "站点类型 {} 暂不支持（当前支持 type=1 JSON CMS；type=3 JS Spider 需脚本引擎，规划中）",
                site.site_type
            ));
        }
        Ok(tvbox_mod::CmsClient {
            api: &site.api,
            http: &self.http,
            headers: site.header.as_ref(),
        })
    }

    pub async fn cms_home(&self, source_id: &str, site_key: &str) -> Result<Vec<VodCategory>> {
        let site = self.find_site(source_id, site_key)?;
        self.cms_client(&site)?.home().await
    }

    pub async fn cms_list(
        &self,
        source_id: &str,
        site_key: &str,
        tid: &str,
        pg: u32,
    ) -> Result<(Vec<VodItem>, u32, u32)> {
        let site = self.find_site(source_id, site_key)?;
        self.cms_client(&site)?.list(tid, pg).await
    }

    pub async fn cms_detail(&self, source_id: &str, site_key: &str, vod_id: &str) -> Result<VodDetail> {
        let site = self.find_site(source_id, site_key)?;
        self.cms_client(&site)?.detail(vod_id).await
    }

    pub async fn cms_search(&self, source_id: &str, site_key: &str, wd: &str) -> Result<Vec<VodItem>> {
        let site = self.find_site(source_id, site_key)?;
        self.cms_client(&site)?.search(wd).await
    }

    /// 解析最终播放地址：必要时走 jx 网页解析，再把结果包上本地代理
    pub async fn resolve_play_url(
        &self,
        source_id: &str,
        site_key: &str,
        raw_url: &str,
    ) -> Result<PlayUrl> {
        let site = self.find_site(source_id, site_key)?;
        let headers = site.header.clone();

        // 已是直链：直接代理
        if jx::is_media_url(raw_url) {
            return self.build_play_url(raw_url, headers.as_ref());
        }

        if let Some(cached) = self.jx_cache_get(raw_url) {
            return self.build_play_url(&cached, headers.as_ref());
        }

        let cfg = self.get_tvbox(source_id)?;
        let resolved = self
            .jx_resolve(&site, &cfg, raw_url)
            .await
            .with_context(|| format!("解析播放页失败: {raw_url}"))?;
        self.jx_cache_put(raw_url, &resolved);
        self.build_play_url(&resolved, headers.as_ref())
    }

    /// 直播频道播放地址
    pub fn resolve_live_url(&self, raw_url: &str) -> Result<PlayUrl> {
        self.build_play_url(raw_url, None)
    }

    fn jx_cache_get(&self, raw: &str) -> Option<String> {
        let now = now_secs();
        let mut cache = self.jx_cache.lock().unwrap();
        if let Some((url, ts)) = cache.get(raw) {
            if now.saturating_sub(*ts) < JX_CACHE_TTL {
                return Some(url.clone());
            }
            cache.remove(raw);
        }
        None
    }

    fn jx_cache_put(&self, raw: &str, resolved: &str) {
        let mut cache = self.jx_cache.lock().unwrap();
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(raw.to_string(), (resolved.to_string(), now_secs()));
    }

    /// jx 三级降级：站点 playUrl 模板 → 配置 parses 接口 → 页面通用抽取
    async fn jx_resolve(
        &self,
        site: &TvBoxSite,
        cfg: &TvBoxConfig,
        raw_url: &str,
    ) -> Result<String> {
        let mut tried: Vec<String> = Vec::new();

        // 1) 站点自定义解析模板（含 {playUrl} / {url} 占位符）
        if let Some(tpl) = &site.play_url {
            let target = tpl
                .replace("{playUrl}", &urlencode(raw_url))
                .replace("{url}", &urlencode(raw_url));
            if target != *tpl {
                match self.fetch_with_timeout(&target, site.header.as_ref()).await {
                    Ok(body) => {
                        if let Some(u) = jx::parse_jx_response(&body, &target) {
                            return Ok(u);
                        }
                        tried.push("站点 playUrl 未取到直链".to_string());
                    }
                    Err(e) => tried.push(format!("站点 playUrl 失败: {e}")),
                }
            }
        }

        // 2) 配置里的解析接口，按顺序试；type=3 需 WebView 跳过
        for p in &cfg.parses {
            let t = p.parse_type.unwrap_or(0);
            if p.url.is_empty() || t == 0 {
                continue;
            }
            if t == 3 {
                tried.push(format!("{}: 需 WebView，已跳过", p.name));
                continue;
            }
            let result = match t {
                2 => self.parse_api_post(&p.url, raw_url).await,
                _ => self.parse_api_get(&p.url, raw_url).await,
            };
            match result {
                Ok(body) => {
                    if let Some(u) = jx::parse_jx_response(&body, &p.url) {
                        return Ok(u);
                    }
                    tried.push(format!("{}: 无直链", p.name));
                }
                Err(e) => tried.push(format!("{}: {e}", p.name)),
            }
        }

        // 3) 抓播放页自身，通用抽取
        match self.fetch_with_timeout(raw_url, site.header.as_ref()).await {
            Ok(body) => {
                if let Some(u) = jx::extract_media(&body, raw_url) {
                    return Ok(u);
                }
                tried.push("页面内未找到媒体地址".to_string());
            }
            Err(e) => tried.push(format!("页面抓取失败: {e}")),
        }

        if tried.is_empty() {
            tried.push("配置未提供解析接口".to_string());
        }
        Err(anyhow!("{}", tried.join("；")))
    }

    async fn fetch_with_timeout(
        &self,
        url: &str,
        headers: Option<&serde_json::Value>,
    ) -> Result<String> {
        match tokio::time::timeout(
            std::time::Duration::from_secs(JX_STEP_TIMEOUT),
            self.fetch_text(url, headers),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(anyhow!("超时 {}s", JX_STEP_TIMEOUT)),
        }
    }

    /// type=1：GET 拼接（`?url=` 前缀式或普通参数式）
    async fn parse_api_get(&self, api: &str, raw_url: &str) -> Result<String> {
        let sep = if api.contains("url=") {
            ""
        } else if api.contains('?') {
            "&url="
        } else {
            "?url="
        };
        let target = format!("{api}{sep}{}", urlencode(raw_url));
        self.fetch_with_timeout(&target, None).await
    }

    /// type=2：POST JSON `{"url": ...}`
    async fn parse_api_post(&self, api: &str, raw_url: &str) -> Result<String> {
        let body = serde_json::json!({ "url": raw_url }).to_string();
        let fut = async {
            let resp = self
                .http
                .post(api)
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
                .context("请求解析接口失败")?;
            if !resp.status().is_success() {
                return Err(anyhow!("解析接口返回 {}", resp.status()));
            }
            Ok(resp.text().await?)
        };
        match tokio::time::timeout(std::time::Duration::from_secs(JX_STEP_TIMEOUT), fut).await {
            Ok(r) => r,
            Err(_) => Err(anyhow!("超时 {}s", JX_STEP_TIMEOUT)),
        }
    }

    fn build_play_url(&self, raw_url: &str, headers: Option<&serde_json::Value>) -> Result<PlayUrl> {
        if !raw_url.contains("://") {
            return Err(anyhow!("无效的播放地址: {raw_url}"));
        }
        let kind = detect_kind(raw_url);
        let url = crate::proxy::build_proxy_url(&self.proxy_base, raw_url, headers);
        Ok(PlayUrl {
            url,
            kind: kind.to_string(),
            raw: raw_url.to_string(),
        })
    }
}

fn detect_kind(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".m3u8") {
        return "hls";
    }
    if path.ends_with(".mp4") {
        return "mp4";
    }
    if path.ends_with(".flv") {
        return "flv";
    }
    "other"
}

fn prefix_group_names(groups: &mut [LiveGroup], prefix: &str) {
    for g in groups {
        if !prefix.is_empty() && !g.name.starts_with(prefix) {
            g.name = format!("{}·{}", prefix, g.name);
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn urlencode(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}
