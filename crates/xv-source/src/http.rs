//! 统一的网络抓取：UA、超时、字符集回退。
//!
//! 国内 IPTV/TVBox 源大量使用 GBK 编码的 txt 直播列表，且服务器经常不回
//! `charset`，所以解码顺序是：BOM → 响应头 charset → UTF-8 严格 → GB18030。

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

pub const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 共享客户端。视频站（尤其 CMS 接口）经常挂 5xx 或拖到十几秒，所以超时给足。
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub struct Fetched {
    pub text: String,
    pub final_url: String,
}

/// 抓一个文本地址，自动处理编码。
pub async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
) -> Result<Fetched> {
    let mut req = client.get(url);
    if let Some(r) = referer.filter(|r| !r.is_empty()) {
        req = req.header(reqwest::header::REFERER, r);
    }
    let resp = req.send().await.with_context(|| format!("请求失败：{url}"))?;
    let status = resp.status();
    let final_url = resp.url().to_string();
    let charset = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(charset_of);
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("读取响应失败：{url}"))?
        .to_vec();
    if !status.is_success() {
        return Err(anyhow!("{} 返回 {status}", url));
    }
    if bytes.is_empty() {
        return Err(anyhow!("{} 返回空内容", url));
    }
    Ok(Fetched {
        text: decode(&bytes, charset.as_deref()),
        final_url,
    })
}

/// 抓 JSON。部分站点会在正文前塞 BOM 或广告字符，这里从第一个 `{` / `[` 起解析。
pub async fn fetch_json(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
) -> Result<Value> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.with_context(|| format!("请求失败：{url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("{url} 返回 {status}"));
    }
    parse_json(&text).ok_or_else(|| anyhow!("{url} 不是合法 JSON：{}", head(&text)))
}

pub fn parse_json(text: &str) -> Option<Value> {
    let start = text
        .char_indices()
        .find(|(_, c)| *c == '{' || *c == '[')
        .map(|(i, _)| i)
        .unwrap_or(0);
    serde_json::from_str(text[start..].trim()).ok()
}

fn head(s: &str) -> String {
    s.chars().take(120).collect()
}

fn charset_of(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("charset="))
        .map(|c| c.trim_matches('"').to_ascii_lowercase())
}

/// 字节 → 文本。UTF-8 解码失败就按 GB18030（GBK 超集）再来一次。
pub fn decode(bytes: &[u8], charset: Option<&str>) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if let Some(cs) = charset {
        if cs.contains("gb") || cs.contains("gbk") || cs.contains("gb2312") {
            let (text, _, _) = encoding_rs::GB18030.decode(bytes);
            return text.into_owned();
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (text, _, had_errors) = encoding_rs::GB18030.decode(bytes);
            if had_errors {
                String::from_utf8_lossy(bytes).into_owned()
            } else {
                text.into_owned()
            }
        }
    }
}

/// 把配置里的相对地址（`./list.txt`）按订阅地址补全。
pub fn join(base: &str, rel: &str) -> String {
    let rel = rel.trim();
    if rel.is_empty() {
        return String::new();
    }
    if rel.contains("://") {
        return rel.to_string();
    }
    match reqwest::Url::parse(base).and_then(|b| b.join(rel)) {
        Ok(u) => u.to_string(),
        Err(_) => rel.to_string(),
    }
}

/// 把 `header` 字段（对象或空）摊平成请求头列表。
pub fn header_pairs(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(obj) = v.as_object() {
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    out.push((k.clone(), s.to_string()));
                }
            }
        }
    }
    out
}

/// 拼成 mpv 的 `http-header-fields`（逗号分隔）。
pub fn mpv_header_fields(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("host"))
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_gbk() {
        let (gbk, _, _) = encoding_rs::GB18030.encode("CCTV-1 综合");
        let text = decode(&gbk, None);
        assert_eq!(text, "CCTV-1 综合");
    }

    #[test]
    fn decodes_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("频道".as_bytes());
        assert_eq!(decode(&bytes, None), "频道");
    }

    #[test]
    fn parses_json_with_prefix_garbage() {
        let v = parse_json("oops {\"a\":1}").unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn joins_header_fields() {
        let pairs = vec![("Referer".to_string(), "https://a/".to_string())];
        assert_eq!(mpv_header_fields(&pairs), "Referer: https://a/");
    }
}
