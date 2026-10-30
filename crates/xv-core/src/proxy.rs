use base64::Engine;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

pub const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// 构造经过本地代理的播放地址
/// `headers`: 额外请求头（Referer/UA/Cookie 等），base64(JSON) 后通过 h 参数传递
pub fn build_proxy_url(base: &str, raw_url: &str, headers: Option<&serde_json::Value>) -> String {
    let u = utf8_percent_encode(raw_url, NON_ALPHANUMERIC).to_string();
    let mut url = format!("{}/p?u={}", base, u);
    if let Some(h) = headers {
        if let Ok(json) = serde_json::to_string(h) {
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());
            url.push_str(&format!("&h={}", b64));
        }
    }
    url
}

/// 解析 h 参数（base64 JSON → HeaderMap）
pub fn decode_headers(h_b64: &str) -> reqwest::header::HeaderMap {
    let mut map = reqwest::header::HeaderMap::new();
    if h_b64.is_empty() {
        return map;
    }
    if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(h_b64) {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(obj) = val.as_object() {
                for (k, v) in obj {
                    if let Some(vs) = v.as_str() {
                        if let (Ok(name), Ok(value)) = (
                            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                            reqwest::header::HeaderValue::from_str(vs),
                        ) {
                            map.insert(name, value);
                        }
                    }
                }
            }
        }
    }
    map
}

/// 判断 URL 是否是 HLS 播放列表
pub fn is_m3u8(url: &str, content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("mpegurl") || ct.contains("vnd.apple") {
        return true;
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".m3u8")
}

/// 重写 M3U8 播放列表：把相对地址改写为绝对地址并全部包上代理，
/// 这样 hls.js 拉取分片与解密 key 时都会经过本地代理（保持防盗链请求头）。
pub fn rewrite_m3u8(text: &str, base_url: &str, proxy_base: &str, headers_json: Option<&serde_json::Value>) -> String {
    let mut out = String::with_capacity(text.len() + 512);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('#') {
            // 指令行：重写其中 URI="..." 引用（EXT-X-KEY / EXT-X-MEDIA / EXT-X-MAP 等）
            if let Some(rewritten) = rewrite_uri_attr(trimmed, base_url, proxy_base, headers_json) {
                out.push_str(&rewritten);
            } else {
                out.push_str(trimmed);
            }
        } else {
            // 裸行：分片或子播放列表地址
            let abs = resolve_url(base_url, trimmed);
            out.push_str(&build_proxy_url(proxy_base, &abs, headers_json));
        }
        out.push('\n');
    }
    out
}

fn rewrite_uri_attr(line: &str, base_url: &str, proxy_base: &str, headers_json: Option<&serde_json::Value>) -> Option<String> {
    let key = "URI=\"";
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    let uri = &rest[..end];
    let abs = resolve_url(base_url, uri);
    let proxied = build_proxy_url(proxy_base, &abs, headers_json);
    Some(format!("{}{}{}", &line[..start], proxied, &line[start + end..]))
}

/// 相对地址解析为绝对地址
pub fn resolve_url(base: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_string();
    }
    if let Ok(parsed) = reqwest::Url::parse(base) {
        if let Ok(resolved) = parsed.join(url) {
            return resolved.to_string();
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_m3u8() {
        let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x1\nseg0.ts\nhttps://cdn.other.com/seg1.ts\n";
        let out = rewrite_m3u8(playlist, "http://host/path/play.m3u8", "http://127.0.0.1:21600", None);
        eprintln!("rewritten:\n{}", out);
        assert!(out.contains("/p?u="));
        assert!(out.contains("127.0.0.1"));
        assert!(
            out.contains("host%2Fpath%2Fkey%2Ebin"),
            "key 地址应解析为绝对地址并包代理"
        );
        // 相对分片行必须已被改写为代理地址（不再以裸路径单独成行）
        assert!(
            !out.lines().any(|l| l.trim() == "seg0.ts"),
            "相对分片应已被重写"
        );
        assert!(
            out.contains("cdn%2Eother%2Ecom"),
            "绝对分片应包代理（域名经编码保留）"
        );
    }

    #[test]
    fn test_proxy_url() {
        let url = build_proxy_url("http://127.0.0.1:21600", "http://a/b.m3u8?x=1", None);
        eprintln!("proxy url = {}", url);
        assert!(url.starts_with("http://127.0.0.1:21600/p?u="));
        assert!(url.contains("m3u8"));
    }
}
