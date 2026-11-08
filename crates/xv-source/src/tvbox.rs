//! TVBox 配置：结构解析、CMS(JSON) 接口客户端、播放地址解析。
//!
//! 只覆盖 `type=1` 的 CMS 接口站（JSON 直出），以及配置里的 `lives` 直播。
//! `type=3` 的 spider 站需要执行 JS / 加载 jar，当前版本直接跳过并在界面上标注。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TvConfig {
    #[serde(default)]
    pub sites: Vec<Site>,
    #[serde(default)]
    pub lives: Vec<LiveEntry>,
    #[serde(default)]
    pub parses: Vec<Parse>,
    #[serde(default)]
    pub spider: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Site {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: i32,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub searchable: Option<i32>,
    #[serde(default)]
    pub ext: Value,
    #[serde(default)]
    pub header: Value,
    #[serde(default)]
    pub jar: Option<String>,
}

impl Site {
    /// TVBox 里 `searchable` 缺省视为可搜索。
    pub fn searchable(&self) -> bool {
        self.searchable.unwrap_or(1) != 0
    }

    /// 规则脚本地址：优先 `ext`（drpy 的写法），其次 `api` 本身是 .js。
    pub fn script_url(&self) -> Option<String> {
        if self.kind != 3 {
            return None;
        }
        if let Some(s) = self.ext.as_str() {
            let s = s.trim();
            if s.ends_with(".js") || (s.contains(".js?") && s.starts_with("http")) {
                return Some(s.to_string());
            }
        }
        let api = self.api.trim();
        if api.ends_with(".js") || api.contains(".js?") {
            return Some(api.to_string());
        }
        None
    }

    /// spider 站，但不是 JS 规则（多半是 `csp_xxx`，需要 Java 内核）。
    pub fn needs_java_spider(&self) -> bool {
        self.kind == 3 && self.script_url().is_none()
    }

    /// 能否由本程序直接调用：type=1 的 CMS JSON 接口，或 type=3 的 JS spider。
    pub fn supported(&self) -> bool {
        (self.kind == 1 && !self.api.is_empty()) || self.script_url().is_some()
    }

    pub fn headers(&self) -> Vec<(String, String)> {
        http::header_pairs(&self.header)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LiveEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub ua: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Parse {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: i32,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub ext: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Category {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Hit {
    pub id: String,
    pub name: String,
    pub pic: Option<String>,
    pub remarks: Option<String>,
    pub year: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Episode {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlaySource {
    pub from: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VodDetail {
    pub id: String,
    pub name: String,
    pub pic: Option<String>,
    pub year: Option<String>,
    pub area: Option<String>,
    pub kind: Option<String>,
    pub actor: Option<String>,
    pub director: Option<String>,
    pub content: Option<String>,
    pub sources: Vec<PlaySource>,
}

/// 直接可播的地址（已经过解析）。
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

pub fn parse_config(text: &str) -> Result<TvConfig> {
    let value = http::parse_json(text)
        .ok_or_else(|| anyhow!("不是合法的 JSON 配置：{}", text.chars().take(120).collect::<String>()))?;
    let mut cfg: TvConfig =
        serde_json::from_value(value).map_err(|e| anyhow!("配置结构无法识别：{e}"))?;
    cfg.sites.retain(|s| !s.name.is_empty());
    if cfg.sites.is_empty() && cfg.lives.is_empty() {
        return Err(anyhow!("配置里既没有站点也没有直播源"));
    }
    Ok(cfg)
}

fn endpoint(api: &str, params: &str) -> String {
    let base = api.trim_end();
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{params}")
}

pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn get(client: &reqwest::Client, site: &Site, params: &str) -> Result<Value> {
    let url = endpoint(&site.api, params);
    http::fetch_json(client, &url, &site.headers()).await
}

pub async fn categories(client: &reqwest::Client, site: &Site) -> Result<Vec<Category>> {
    let json = get(client, site, "ac=list").await?;
    let mut out = Vec::new();
    for c in json.get("class").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let id = s(&c, "type_id");
        let name = s(&c, "type_name");
        if !id.is_empty() {
            out.push(Category { id, name });
        }
    }
    Ok(out)
}

pub async fn list(
    client: &reqwest::Client,
    site: &Site,
    tid: &str,
    page: u32,
) -> Result<(Vec<Hit>, u32)> {
    let json = get(
        client,
        site,
        &format!("ac=videolist&t={}&pg={page}", enc(tid)),
    )
    .await?;
    let pages = json
        .get("pagecount")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .min(9999) as u32;
    Ok((hits(&json), pages))
}

pub async fn search(client: &reqwest::Client, site: &Site, wd: &str) -> Result<Vec<Hit>> {
    let json = get(client, site, &format!("ac=videolist&wd={}", enc(wd))).await?;
    let mut out = hits(&json);
    if out.is_empty() {
        // 部分 MacCMS 只认 ac=detail
        if let Ok(json) = get(client, site, &format!("ac=detail&wd={}", enc(wd))).await {
            out = hits(&json);
        }
    }
    Ok(out)
}

pub async fn detail(client: &reqwest::Client, site: &Site, id: &str) -> Result<VodDetail> {
    let json = get(client, site, &format!("ac=videolist&ids={}", enc(id))).await?;
    let first = json
        .get("list")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned();
    let first = match first {
        Some(v) => v,
        None => {
            let json = get(client, site, &format!("ac=detail&ids={}", enc(id))).await?;
            json.get("list")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .cloned()
                .ok_or_else(|| anyhow!("站点没有返回该影片"))?
        }
    };
    Ok(to_detail(&first))
}

fn hits(json: &Value) -> Vec<Hit> {
    let mut out = Vec::new();
    for v in json.get("list").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let id = s(&v, "vod_id");
        let name = s(&v, "vod_name");
        if id.is_empty() || name.is_empty() {
            continue;
        }
        out.push(Hit {
            id,
            name,
            pic: opt(&v, "vod_pic"),
            remarks: opt(&v, "vod_remarks"),
            year: opt(&v, "vod_year"),
        });
    }
    out
}

/// 兼容按 CMS 字段返回的 spider 结果；没有 `vod_play_url` 时返回 None。
pub fn detail_from_value(v: &Value) -> Option<VodDetail> {
    if s(v, "vod_play_url").is_empty() {
        return None;
    }
    Some(to_detail(v))
}

fn to_detail(v: &Value) -> VodDetail {
    let froms: Vec<String> = s(v, "vod_play_from")
        .split("$$$")
        .map(|x| x.trim().to_string())
        .collect();

    let mut sources = Vec::new();
    for (i, part) in s(v, "vod_play_url").split("$$$").enumerate() {
        let mut episodes = Vec::new();
        for line in part.split('#') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // 形如「第01集$https://...」；没有 $ 的整段当作地址，名字用序号
            let (name, url) = match line.split_once('$') {
                Some((n, u)) => (n.trim().to_string(), u.trim().to_string()),
                None => (format!("第{}集", episodes.len() + 1), line.to_string()),
            };
            if url.is_empty() {
                continue;
            }
            episodes.push(Episode { name, url });
        }
        if episodes.is_empty() {
            continue;
        }
        let from = froms
            .get(i)
            .filter(|x| !x.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("线路{}", sources.len() + 1));
        sources.push(PlaySource { from, episodes });
    }

    // 采集站的第 1 条线路经常是分享页（`/share/xxx`、`/play/xxx`），直接播会失败；
    // 把「首集就是真实流地址」的线路排前面，播放页默认就能选中能播的那条。
    sources.sort_by_key(|p| {
        let streamish = p
            .episodes
            .first()
            .map(|e| looks_like_stream(&e.url))
            .unwrap_or(false);
        std::cmp::Reverse(streamish)
    });

    VodDetail {
        id: s(v, "vod_id"),
        name: s(v, "vod_name"),
        pic: opt(v, "vod_pic"),
        year: opt(v, "vod_year"),
        area: opt(v, "vod_area"),
        kind: opt(v, "vod_type"),
        actor: opt(v, "vod_actor"),
        director: opt(v, "vod_director"),
        content: opt(v, "vod_content"),
        sources,
    }
}

/// 粗判一个播放地址是不是「能直接喂给播放器」的流，而不是需要解析的分享页。
fn looks_like_stream(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    if !u.starts_with("http://") && !u.starts_with("https://") {
        return false;
    }
    let path = u.split('?').next().unwrap_or(&u);
    path.ends_with(".m3u8")
        || path.ends_with(".mp4")
        || path.ends_with(".flv")
        || path.ends_with(".ts")
        || path.ends_with(".mpd")
        || path.ends_with(".mkv")
}

fn s(v: &Value, key: &str) -> String {    match v.get(key) {
        Some(Value::String(x)) => x.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn opt(v: &Value, key: &str) -> Option<String> {
    let x = s(v, key);
    (!x.is_empty()).then_some(x)
}

// ---------- 播放地址解析 ----------

pub fn is_media_url(url: &str) -> bool {
    crate::extract::is_media(url)
        || url
            .split(['?', '#'])
            .next()
            .unwrap_or(url)
            .to_ascii_lowercase()
            .ends_with(".mp3")
}

fn flags_of(parse: &Parse) -> Vec<String> {
    parse
        .ext
        .get("flag")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|x| x.trim().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// 把 CMS 给的播放地址变成 mpv 能直接打开的真实地址。
pub async fn resolve(
    client: &reqwest::Client,
    site: &Site,
    parses: &[Parse],
    flag: &str,
    url: &str,
) -> Result<Resolved> {
    let url = url.trim();
    if url.is_empty() {
        return Err(anyhow!("播放地址为空"));
    }
    if is_media_url(url) {
        return Ok(Resolved {
            url: url.to_string(),
            headers: site.headers(),
        });
    }

    // 优先匹配 `flag` 声明了支持的解析器
    let flag_lc = flag.trim().to_ascii_lowercase();
    let mut ordered: Vec<&Parse> = Vec::new();
    for p in parses {
        if flags_of(p).iter().any(|f| f == &flag_lc) {
            ordered.push(p);
        }
    }
    for p in parses {
        if !ordered.iter().any(|x| std::ptr::eq(*x, p)) {
            ordered.push(p);
        }
    }

    let mut last_err = None;
    for p in ordered {
        if p.url.is_empty() {
            continue;
        }
        match try_parse(client, p, url).await {
            Ok(found) => {
                return Ok(Resolved {
                    url: found,
                    headers: site.headers(),
                })
            }
            Err(e) => last_err = Some(e),
        }
    }
    // 解析器都不行：把地址当网页抓下来，从 HTML/JS 里捞直链（分享页最常见）
    if let Ok(found) = page_extract(client, site, url).await {
        return Ok(found);
    }

    Err(anyhow!(
        "无法解析播放地址（{}），站点可能需要专用解析器{}",
        url.chars().take(80).collect::<String>(),
        last_err
            .map(|e| format!("：{e}"))
            .unwrap_or_default()
    ))
}

/// 直接抓播放页/分享页，按页面里的变量还原直链。
async fn page_extract(client: &reqwest::Client, site: &Site, url: &str) -> Result<Resolved> {
    if !url.starts_with("http") {
        return Err(anyhow!("不是网页地址"));
    }
    let fetched = http::fetch_text(client, url, Some(url)).await?;
    if fetched.text.trim_start().starts_with("#EXTM3U") {
        return Ok(Resolved {
            url: fetched.final_url,
            headers: page_headers(site, url),
        });
    }
    let found = crate::extract::find_media(&fetched.text, &fetched.final_url)
        .ok_or_else(|| anyhow!("页面里没有找到媒体地址"))?;
    Ok(Resolved {
        url: found,
        headers: page_headers(site, &fetched.final_url),
    })
}

fn page_headers(site: &Site, page: &str) -> Vec<(String, String)> {
    let mut pairs = site.headers();
    if !pairs.iter().any(|(k, _)| k.eq_ignore_ascii_case("referer")) {
        pairs.push(("Referer".to_string(), page.to_string()));
    }
    pairs
}

async fn try_parse(client: &reqwest::Client, parse: &Parse, target: &str) -> Result<String> {
    let resp = if parse.kind == 1 {
        // 官方解析接口多半是 `https://host/parse?url=`，直接把目标地址接在后面
        let joiner = if parse.url.ends_with('=') || parse.url.ends_with('/') {
            ""
        } else if parse.url.contains('?') {
            "&url="
        } else {
            "?url="
        };
        let full = format!("{}{joiner}{}", parse.url, enc(target));
        client
            .get(full)
            .timeout(std::time::Duration::from_secs(12))
            .send()
            .await?
    } else {
        client
            .post(&parse.url)
            .json(&serde_json::json!({ "url": target }))
            .timeout(std::time::Duration::from_secs(12))
            .send()
            .await?
    };
    let text = resp.text().await.unwrap_or_default();
    if let Some(v) = http::parse_json(&text) {
        if let Some(u) = v.get("url").and_then(|x| x.as_str()) {
            if is_media_url(u) {
                return Ok(u.to_string());
            }
        }
    }
    extract_media(&text).ok_or_else(|| anyhow!("解析器 {} 没有返回可播地址", parse.name))
}

/// 从任意文本里捞第一个媒体地址（解析接口常返回 JSONP / JS 片段）。
fn extract_media(text: &str) -> Option<String> {
    let unescaped = text.replace("\\/", "/");
    let mut idx = 0;
    let bytes = unescaped.as_bytes();
    while let Some(pos) = unescaped[idx..].find("http") {
        let start = idx + pos;
        let rest = &unescaped[start..];
        let end = rest
            .find(|c: char| c == '"' || c == '\'' || c == '<' || c == '>' || c.is_whitespace() || c == '\\')
            .unwrap_or(rest.len());
        let cand = &rest[..end];
        if is_media_url(cand) {
            return Some(cand.to_string());
        }
        idx = start + 4.max(end);
        if idx >= bytes.len() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let text = r#"{
          "spider": "https://x/spider.jar;md5;abc",
          "sites": [
            {"key":"a","name":"A站","type":1,"api":"https://a/api.php/provide/vod/","searchable":1},
            {"key":"b","name":"B站","type":3,"api":"csp_B","jar":"https://x/b.jar"}
          ],
          "lives":[{"name":"央视","url":"https://x/live.txt"}],
          "parses":[{"name":"官方","type":1,"url":"https://p/?url=","ext":{"flag":["qq"]}}]
        }"#;
        let cfg = parse_config(text).unwrap();
        assert_eq!(cfg.sites.len(), 2);
        assert!(cfg.sites[0].searchable());
        assert!(!cfg.sites[1].supported());
        assert_eq!(cfg.lives[0].name, "央视");
        assert_eq!(cfg.parses.len(), 1);
    }

    #[test]
    fn splits_play_sources() {
        let v = serde_json::json!({
            "vod_id": "7",
            "vod_name": "测试",
            "vod_play_from": "量子$$$备用",
            "vod_play_url": "第01集$https://a/1.m3u8#第02集$https://a/2.m3u8$$$HD$https://b/1.mp4"
        });
        let d = to_detail(&v);
        assert_eq!(d.sources.len(), 2);
        assert_eq!(d.sources[0].from, "量子");
        assert_eq!(d.sources[0].episodes[1].name, "第02集");
        assert_eq!(d.sources[1].episodes[0].url, "https://b/1.mp4");
    }

    #[test]
    fn endpoint_builds_query() {
        assert_eq!(endpoint("https://a/vod", "ac=list"), "https://a/vod?ac=list");
        assert_eq!(
            endpoint("https://a/vod?x=1", "ac=list"),
            "https://a/vod?x=1&ac=list"
        );
    }

    #[test]
    fn media_url_detection() {
        assert!(is_media_url("https://a/1.m3u8?t=1"));
        assert!(is_media_url("https://a/1.MP4"));
        assert!(!is_media_url("https://a/play/1.html"));
    }

    #[test]
    fn extracts_media_from_js() {
        let text = r#"var x = {"url":"https:\/\/a\/1.m3u8"}"#;
        assert_eq!(extract_media(text).as_deref(), Some("https://a/1.m3u8"));
    }
}
