use crate::model::*;
use crate::sources::{jx, live_parse, spider, tvbox as tvbox_mod};
use crate::store;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// jx 解析结果缓存有效期（秒）
const JX_CACHE_TTL: u64 = 600;
/// 单个解析接口的超时（秒）
const JX_STEP_TIMEOUT: u64 = 15;
/// 单次脚本调用超时（秒）
const SPIDER_TIMEOUT: u64 = 45;
/// 聚合搜索中单个站点的超时（秒）
const SEARCH_TIMEOUT: u64 = 15;
/// 聚合搜索默认参与站点数
pub const DEFAULT_SEARCH_SITES: usize = 40;

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
    /// spider 脚本地址 -> 源码
    pub spider_js_cache: Mutex<HashMap<String, String>>,
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
            spider_js_cache: Mutex::new(HashMap::new()),
            proxy_base,
        }
    }

    async fn fetch_text(&self, url: &str, headers: Option<&serde_json::Value>) -> Result<String> {
        // 本地文件：file:///E:/x.json 或直接给路径
        if let Some(path) = local_path(url) {
            return std::fs::read_to_string(&path)
                .with_context(|| format!("读取本地文件失败: {path}"));
        }
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

    /// 取 TVBox 配置：冷启动（进程重启后缓存为空）时自动拉取一次
    pub async fn ensure_tvbox(&self, source_id: &str) -> Result<TvBoxConfig> {
        if let Ok(cfg) = self.get_tvbox(source_id) {
            return Ok(cfg);
        }
        self.refresh_source(source_id).await?;
        self.get_tvbox(source_id)
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

    // ---------- Spider（type=3）----------

    fn source_url(&self, source_id: &str) -> String {
        self.sources
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == source_id)
            .map(|s| s.url.clone())
            .unwrap_or_default()
    }

    /// 取站点脚本，规则与 TVBox 一致：
    /// - `api` 以 `.js` 结尾 → api 本身就是 Spider 脚本（`ext` 作为脚本配置）
    /// - 否则 `api` 是 csp 类名，脚本优先取站点 `ext`（.js），再退到配置 `spider`（.js）
    async fn site_js(
        &self,
        site: &TvBoxSite,
        cfg: &TvBoxConfig,
        source_url: &str,
    ) -> Result<String> {
        let api = site.api.trim();
        let lower_api = api.to_ascii_lowercase();
        let ext = site
            .ext
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let js_url = if lower_api.ends_with(".js") || lower_api.contains(".js?") {
            if api.starts_with("http://")
                || api.starts_with("https://")
                || api.starts_with("file://")
            {
                api.to_string()
            } else if api.starts_with("assets://") {
                return Err(anyhow!("站点脚本使用 assets:// 打包资源（{api}），当前版本不支持"));
            } else {
                spider::join_url(source_url, api)
            }
        } else if ext.to_ascii_lowercase().ends_with(".js") {
            spider::join_url(source_url, &ext)
        } else if let Some(spider_path) = cfg
            .spider
            .as_ref()
            .filter(|s| s.to_ascii_lowercase().ends_with(".js"))
        {
            spider::join_url(source_url, spider_path)
        } else {
            return Err(anyhow!(
                "该站点走 Jar Spider（Java 实现），桌面端无法执行；需要 api 为 .js 或 ext 为 .js 的站点"
            ));
        };

        if let Some(js) = self.spider_js_cache.lock().unwrap().get(&js_url).cloned() {
            return Ok(js);
        }
        let js = self.fetch_text(&js_url, None).await?;
        if js.trim().is_empty() {
            return Err(anyhow!("站点脚本为空: {js_url}"));
        }
        if is_esm_script(&js) {
            return Err(anyhow!(
                "脚本是 ES Module（drpy2 等引擎），当前沙箱只支持传统脚本"
            ));
        }
        self.spider_js_cache
            .lock()
            .unwrap()
            .insert(js_url, js.clone());
        Ok(js)
    }

    /// 站点脚本运行前的环境变量（TVBox 里脚本可读 `ext` / `key`）
    fn site_env(site: &TvBoxSite) -> String {
        let ext = match site.ext.as_ref() {
            Some(serde_json::Value::String(s)) => serde_json::to_string(s).unwrap_or_default(),
            Some(other) => other.to_string(),
            None => "null".to_string(),
        };
        format!(
            "var ext = {ext}; var KEY = {}; var key = KEY;\n",
            json_q(&site.key)
        )
    }

    /// 在沙箱里跑一个站点方法，返回 JSON
    async fn spider_call(
        &self,
        js: &str,
        base: &str,
        env: &str,
        method: &str,
        args: &str,
    ) -> Result<serde_json::Value> {
        let (js, base, env, method, args) = (
            js.to_string(),
            base.to_string(),
            env.to_string(),
            method.to_string(),
            args.to_string(),
        );
        let handle = tokio::task::spawn_blocking(move || {
            spider::call_with_env(&js, &base, &env, &method, &args)
        });
        match tokio::time::timeout(std::time::Duration::from_secs(SPIDER_TIMEOUT), handle).await {
            Ok(Ok(Ok(text))) => {
                serde_json::from_str(&text).map_err(|e| anyhow!("脚本返回非 JSON: {e}"))
            }
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(join_err)) => Err(anyhow!("脚本线程异常: {join_err}")),
            Err(_) => Err(anyhow!("脚本执行超时（{SPIDER_TIMEOUT}s）")),
        }
    }

    async fn spider_ready(&self, source_id: &str, site_key: &str) -> Result<(TvBoxSite, String, String)> {
        let cfg = self.ensure_tvbox(source_id).await?;
        let site = cfg
            .sites
            .iter()
            .find(|s| s.key == site_key)
            .cloned()
            .ok_or_else(|| anyhow!("站点不存在: {site_key}"))?;
        let base = self.source_url(source_id);
        let js = self.site_js(&site, &cfg, &base).await?;
        Ok((site, js, base))
    }

    pub async fn cms_home(&self, source_id: &str, site_key: &str) -> Result<Vec<VodCategory>> {
        self.ensure_tvbox(source_id).await?;
        let site = self.find_site(source_id, site_key)?;
        if site.site_type == 3 {
            let (_site, js, base) = self.spider_ready(source_id, site_key).await?;
            let v = self.spider_call(&js, &base, &Self::site_env(&site), "homeContent", "false").await?;
            return Ok(parse_spider_classes(&v));
        }
        self.cms_client(&site)?.home().await
    }

    pub async fn cms_list(
        &self,
        source_id: &str,
        site_key: &str,
        tid: &str,
        pg: u32,
    ) -> Result<(Vec<VodItem>, u32, u32)> {
        self.ensure_tvbox(source_id).await?;
        let site = self.find_site(source_id, site_key)?;
        if site.site_type == 3 {
            let (_site, js, base) = self.spider_ready(source_id, site_key).await?;
            let args = format!("{}, {}, false, {{}}", json_q(tid), pg);
            let v = self
                .spider_call(&js, &base, &Self::site_env(&site), "categoryContent", &args)
                .await?;
            let items = parse_spider_list(&v);
            let page_count = v.get("pagecount").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
            let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            return Ok((items, page_count.max(1), total));
        }
        self.cms_client(&site)?.list(tid, pg).await
    }

    pub async fn cms_detail(
        &self,
        source_id: &str,
        site_key: &str,
        vod_id: &str,
    ) -> Result<VodDetail> {
        self.ensure_tvbox(source_id).await?;
        let site = self.find_site(source_id, site_key)?;
        if site.site_type == 3 {
            let (_site, js, base) = self.spider_ready(source_id, site_key).await?;
            let args = format!("[{}]", json_q(vod_id));
            let v = self
                .spider_call(&js, &base, &Self::site_env(&site), "detailContent", &args)
                .await?;
            let first = v
                .get("list")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .cloned()
                .ok_or_else(|| anyhow!("站点未返回详情: {vod_id}"))?;
            let mut detail = tvbox_mod::parse_detail(&first)?;
            // 蜘蛛站点的选集是「集 id」，播放时要用 play_from 作为 flag 回调 playerContent
            for ps in detail.play_sources.iter_mut() {
                let flag = ps.from.clone();
                for ep in ps.episodes.iter_mut() {
                    if !ep.1.contains("://") {
                        ep.1 = format!("{flag}|||{}", ep.1);
                    }
                }
            }
            return Ok(detail);
        }
        self.cms_client(&site)?.detail(vod_id).await
    }

    pub async fn cms_search(
        &self,
        source_id: &str,
        site_key: &str,
        wd: &str,
    ) -> Result<Vec<VodItem>> {
        self.ensure_tvbox(source_id).await?;
        let site = self.find_site(source_id, site_key)?;
        if site.site_type == 3 {
            let (_site, js, base) = self.spider_ready(source_id, site_key).await?;
            let args = format!("{}, false", json_q(wd));
            let v = self
                .spider_call(&js, &base, &Self::site_env(&site), "searchContent", &args)
                .await?;
            return Ok(parse_spider_list(&v));
        }
        self.cms_client(&site)?.search(wd).await
    }

    /// 聚合搜索：把关键词并发丢给所有可用站点，合并结果并记录失败原因
    pub async fn aggregate_search(
        self: &Arc<Self>,
        wd: &str,
        max_sites: usize,
    ) -> Result<AggResult> {
        let wd = wd.trim().to_string();
        if wd.is_empty() {
            return Err(anyhow!("搜索关键词为空"));
        }

        // 规划：遍历已启用的 TVBox 源，挑出可搜索的 type=1 / type=3 站点
        let mut plan: Vec<(u8, String, String, String)> = Vec::new();
        let mut skipped = 0usize;
        for s in self
            .list_sources()
            .into_iter()
            .filter(|s| s.enabled && s.kind == SourceKind::Tvbox)
        {
            let cfg = match self.ensure_tvbox(&s.id).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            for site in cfg.sites.iter() {
                if plan.len() >= max_sites {
                    break;
                }
                if site.searchable == Some(0) {
                    continue;
                }
                if site.site_type != 1 && site.site_type != 3 {
                    skipped += 1;
                    continue;
                }
                if site.site_type == 3 && !site_runnable(site, &cfg) {
                    skipped += 1;
                    continue;
                }
                let prio = if site.site_type == 1 { 0u8 } else { 1u8 };
                plan.push((prio, s.id.clone(), site.key.clone(), site.name.clone()));
            }
        }

        // type=1 JSON 接口最稳，优先占位

        plan.sort_by_key(|(prio, _, _, _)| *prio);
        let site_total = plan.len();
        let mut set = tokio::task::JoinSet::new();
        for (_prio, sid, skey, sname) in plan {
            let me = self.clone();
            let kw = wd.clone();
            let key = skey.clone();
            set.spawn(async move {
                let fut = me.cms_search(&sid, &skey, &kw);
                let res = tokio::time::timeout(std::time::Duration::from_secs(SEARCH_TIMEOUT), fut).await;
                (sid, key, sname, res)
            });
        }

        let mut items: Vec<AggItem> = Vec::new();
        let mut failures: Vec<AggFailure> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        let mut site_hit = 0usize;

        while let Some(joined) = set.join_next().await {
            let Ok((sid, skey, sname, res)) = joined else {
                continue;
            };
            match res {
                Ok(Ok(list)) => {
                    if list.is_empty() {
                        continue;
                    }
                    site_hit += 1;
                    for v in list {
                        if !seen.insert((sid.clone(), format!("{skey}:{}", v.vod_id))) {
                            continue;
                        }
                        items.push(AggItem {
                            source_id: sid.clone(),
                            site_key: skey.clone(),
                            site_name: sname.clone(),
                            vod_id: v.vod_id,
                            vod_name: v.vod_name,
                            vod_pic: v.vod_pic,
                            vod_remarks: v.vod_remarks,
                        });
                    }
                }
                Ok(Err(e)) => failures.push(AggFailure {
                    site_name: sname,
                    site_key: skey,
                    reason: format!("{e:#}"),
                }),
                Err(_) => failures.push(AggFailure {
                    site_name: sname,
                    site_key: skey,
                    reason: format!("超时（{SEARCH_TIMEOUT}s）"),
                }),
            }
        }

        Ok(AggResult {
            items,
            failures,
            site_total,
            site_skipped: skipped,
            site_hit,
        })
    }

    /// 解析最终播放地址：必要时走 jx 网页解析，再把结果包上本地代理
    pub async fn resolve_play_url(
        &self,
        source_id: &str,
        site_key: &str,
        raw_url: &str,
    ) -> Result<PlayUrl> {
        self.ensure_tvbox(source_id).await?;
        let site = self.find_site(source_id, site_key)?;
        let headers = site.header.clone();

        // 蜘蛛站点：选集地址形如 `flag|||集id`，需回调 playerContent
        if site.site_type == 3 && raw_url.contains("|||") {
            let (flag, ep_id) = raw_url.split_once("|||").unwrap_or((raw_url, ""));
            let cfg = self.ensure_tvbox(source_id).await?;
            let base = self.source_url(source_id);
            let js = self.site_js(&site, &cfg, &base).await?;
            let args = format!("{}, {}, []", json_q(flag), json_q(ep_id));
            let v = self
                .spider_call(&js, &base, &Self::site_env(&site), "playerContent", &args)
                .await?;
            let url = v
                .get("url")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if url.is_empty() {
                return Err(anyhow!("站点未返回播放地址"));
            }
            let parse = v.get("parse").and_then(|x| x.as_u64()).unwrap_or(0);
            let hdr = v
                .get("header")
                .filter(|h| h.is_object())
                .cloned()
                .or(headers.clone());
            let final_url = if parse == 1 {
                self.jx_resolve(&site, &cfg, &url).await?
            } else {
                url
            };
            return self.build_play_url(&final_url, hdr.as_ref());
        }

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

fn json_q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn parse_spider_classes(v: &serde_json::Value) -> Vec<VodCategory> {
    let mut out = Vec::new();
    if let Some(class) = v.get("class").and_then(|x| x.as_array()) {
        for c in class {
            let type_id = c
                .get("type_id")
                .map(vstr_any)
                .unwrap_or_default();
            let type_name = c.get("type_name").map(vstr_any).unwrap_or_default();
            if type_id.is_empty() {
                continue;
            }
            out.push(VodCategory { type_id, type_name });
        }
    }
    out
}

fn parse_spider_list(v: &serde_json::Value) -> Vec<VodItem> {
    let mut out = Vec::new();
    if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
        for it in list {
            let vod_id = it.get("vod_id").map(vstr_any).unwrap_or_default();
            if vod_id.is_empty() {
                continue;
            }
            out.push(VodItem {
                vod_id,
                vod_name: it.get("vod_name").map(vstr_any).unwrap_or_default(),
                vod_pic: it
                    .get("vod_pic")
                    .map(vstr_any)
                    .filter(|s| !s.is_empty()),
                vod_remarks: it
                    .get("vod_remarks")
                    .map(vstr_any)
                    .filter(|s| !s.is_empty()),
            });
        }
    }
    out
}

fn vstr_any(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// 把 `file://` / 裸路径转成本地路径；非本地返回 None
fn local_path(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("file://") {
        let p = rest.trim_start_matches('/');
        // Windows: file:///E:/a.js -> E:/a.js
        if p.len() > 1 && p.as_bytes()[1] == b':' {
            return Some(p.to_string());
        }
        return Some(format!("/{p}"));
    }
    if url.starts_with('/') || (url.len() > 2 && url.as_bytes()[1] == b':') {
        return Some(url.to_string());
    }
    None
}

/// type=3 站点是否能在本地 JS 沙箱里跑（jar 依赖一律判否）
fn site_runnable(site: &TvBoxSite, cfg: &TvBoxConfig) -> bool {
    let api = site.api.trim().to_ascii_lowercase();
    if api.contains("drpy") {
        // drpy2 是 ESM 引擎，Boa 无法加载
        return false;
    }
    if api.ends_with(".js") || api.contains(".js?") {
        return !api.starts_with("assets://");
    }
    let ext_js = site
        .ext
        .as_ref()
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase().ends_with(".js"))
        .unwrap_or(false);
    if ext_js {
        return true;
    }
    cfg.spider
        .as_ref()
        .map(|s| s.to_ascii_lowercase().ends_with(".js"))
        .unwrap_or(false)
}

/// 判断脚本是不是 ES Module（drpy2 等，Boa 不支持）
fn is_esm_script(js: &str) -> bool {
    let head = js.trim_start();
    head.starts_with("import ")
        || head.starts_with("export ")
        || head[..head.len().min(400)].contains("import.meta")
}
