use crate::model::{PlaySource, TvBoxConfig, VodCategory, VodDetail, VodItem};
use anyhow::{anyhow, Context, Result};

/// 解析 TVBox 配置 JSON
/// - `spider` 字段可能带 `;md5;xxx` 后缀，剥离之
pub fn parse_config(text: &str) -> Result<TvBoxConfig> {
    let mut cfg: TvBoxConfig = serde_json::from_str(text).map_err(|e| anyhow!("配置 JSON 解析失败: {e}"))?;
    if let Some(spider) = &cfg.spider {
        // 形如 "xxx.jar;md5;hash" → 只保留第一段
        let first = spider.split(';').next().unwrap_or("").to_string();
        cfg.spider = Some(first);
    }
    Ok(cfg)
}

/// MAC CMS JSON (TVBox type=1) 客户端
pub struct CmsClient<'a> {
    pub api: &'a str,
    pub http: &'a reqwest::Client,
    pub headers: Option<&'a serde_json::Value>,
}

impl<'a> CmsClient<'a> {
    fn req(&self, params: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.api.trim_end_matches('?'), params);
        let mut rb = self.http.get(&url).header("User-Agent", DEFAULT_UA);
        if let Some(h) = self.headers {
            if let Some(obj) = h.as_object() {
                for (k, v) in obj {
                    if let Some(vs) = v.as_str() {
                        rb = rb.header(k.as_str(), vs);
                    }
                }
            }
        }
        rb
    }

    async fn get_json(&self, params: &str) -> Result<serde_json::Value> {
        let resp = self.req(params).send().await.context("请求 CMS 接口失败")?;
        if !resp.status().is_success() {
            return Err(anyhow!("CMS 接口返回 {}", resp.status()));
        }
        let text = resp.text().await?;
        // 兼容部分站点返回带 BOM / 前置垃圾字符
        let json_start = text.find('{').or_else(|| text.find('[')).unwrap_or(0);
        serde_json::from_str(&text[json_start..]).map_err(|e| anyhow!("CMS 响应解析失败: {e}"))
    }

    /// 首页：分类列表
    pub async fn home(&self) -> Result<Vec<VodCategory>> {
        let json = self.get_json("?ac=list").await?;
        let mut out = Vec::new();
        if let Some(class) = json.get("class").and_then(|v| v.as_array()) {
            for c in class {
                let type_id = c.get("type_id").map(vstr).unwrap_or_default();
                let type_name = c.get("type_name").map(vstr).unwrap_or_default();
                if type_id.is_empty() {
                    continue;
                }
                out.push(VodCategory { type_id, type_name });
            }
        }
        Ok(out)
    }

    /// 分类影片列表
    pub async fn list(&self, tid: &str, pg: u32) -> Result<(Vec<VodItem>, u32, u32)> {
        let json = self
            .get_json(&format!("?ac=videolist&t={}&pg={}", urlencode(tid), pg))
            .await?;
        let items = parse_vod_list(&json);
        let page_count = json
            .get("pagecount")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let total = json.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Ok((items, page_count, total))
    }

    /// 影片详情（含播放列表）
    pub async fn detail(&self, vod_id: &str) -> Result<VodDetail> {
        let json = self
            .get_json(&format!("?ac=videolist&ids={}", urlencode(vod_id)))
            .await?;
        let first = json
            .get("list")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| anyhow!("未找到影片 {}", vod_id))?;
        parse_detail(&first)
    }

    /// 搜索
    pub async fn search(&self, wd: &str) -> Result<Vec<VodItem>> {
        let json = self
            .get_json(&format!("?ac=videolist&wd={}", urlencode(wd)))
            .await?;
        Ok(parse_vod_list(&json))
    }
}

fn parse_vod_list(json: &serde_json::Value) -> Vec<VodItem> {
    let mut out = Vec::new();
    if let Some(list) = json.get("list").and_then(|v| v.as_array()) {
        for v in list {
            let vod_id = v.get("vod_id").map(vstr).unwrap_or_default();
            let vod_name = v.get("vod_name").map(vstr).unwrap_or_default();
            if vod_id.is_empty() {
                continue;
            }
            out.push(VodItem {
                vod_id,
                vod_name,
                vod_pic: v.get("vod_pic").map(vstr).filter(|s| !s.is_empty()),
                vod_remarks: v
                    .get("vod_remarks")
                    .map(vstr)
                    .filter(|s| !s.is_empty()),
            });
        }
    }
    out
}

fn parse_detail(v: &serde_json::Value) -> Result<VodDetail> {
    let play_from = v.get("vod_play_from").map(vstr).unwrap_or_default();
    let play_url = v.get("vod_play_url").map(vstr).unwrap_or_default();

    let froms: Vec<String> = play_from
        .split(['$', ',', '#'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut play_sources = Vec::new();
    for (i, part) in play_url.split("$$$").enumerate() {
        let mut episodes = Vec::new();
        for ep in part.split('#') {
            if let Some((name, url)) = ep.split_once('$') {
                let url = url.trim();
                if !url.is_empty() {
                    episodes.push((name.trim().to_string(), url.to_string()));
                }
            }
        }
        if episodes.is_empty() {
            continue;
        }
        let from = froms
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("播放源{}", play_sources.len() + 1));
        play_sources.push(PlaySource { from, episodes });
    }

    Ok(VodDetail {
        vod_id: v.get("vod_id").map(vstr).unwrap_or_default(),
        vod_name: v.get("vod_name").map(vstr).unwrap_or_default(),
        vod_pic: v.get("vod_pic").map(vstr).filter(|s| !s.is_empty()),
        vod_year: v.get("vod_year").map(vstr).filter(|s| !s.is_empty()),
        vod_area: v.get("vod_area").map(vstr).filter(|s| !s.is_empty()),
        vod_type: v.get("vod_type").map(vstr).filter(|s| !s.is_empty()),
        vod_actor: v.get("vod_actor").map(vstr).filter(|s| !s.is_empty()),
        vod_director: v.get("vod_director").map(vstr).filter(|s| !s.is_empty()),
        vod_content: v.get("vod_content").map(vstr).filter(|s| !s.is_empty()),
        play_sources,
    })
}

fn vstr(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => v.to_string().trim_matches('"').to_string(),
    }
}

fn urlencode(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

pub const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let text = r#"{
            "spider": "https://x/spider.jar;md5;abcdef",
            "sites": [
                {"key": "pear", "name": "皮皮", "type": 1, "api": "https://x/api.php/provide/vod", "searchable": 1},
                {"key": "js1", "name": "JS源", "type": 3, "api": "csp_XXX", "ext": "https://x/xx.js"}
            ],
            "lives": [{"name": "直播", "type": 0, "url": "https://x/live.txt"}]
        }"#;
        let cfg = parse_config(text).unwrap();
        assert_eq!(cfg.sites.len(), 2);
        assert_eq!(cfg.spider.as_deref(), Some("https://x/spider.jar"));
        assert_eq!(cfg.sites[0].name, "皮皮");
        assert_eq!(cfg.lives[0].name, "直播");
    }

    #[test]
    fn test_parse_detail_play_url() {
        let v = serde_json::json!({
            "vod_id": 1, "vod_name": "测试片",
            "vod_play_from": "量子$$$资源二",
            "vod_play_url": "第01集$https://a/1.m3u8#第02集$https://a/2.m3u8$$$HD$https://b/1.mp4"
        });
        let d = parse_detail(&v).unwrap();
        assert_eq!(d.play_sources.len(), 2);
        assert_eq!(d.play_sources[0].from, "量子");
        assert_eq!(d.play_sources[0].episodes.len(), 2);
        assert_eq!(d.play_sources[1].episodes[0].0, "HD");
        assert_eq!(d.play_sources[1].episodes[0].1, "https://b/1.mp4");
    }
}
