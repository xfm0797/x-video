//! jx 网页解析（TVBox 的「网页解析」能力）
//!
//! 目标：把「播放页/分享页/解析接口返回的中间结果」还原成真正的媒体直链。
//! 三层策略，逐层降级：
//! 1. 解析接口（TVBox 配置里的 `parses`，type=1 GET / type=2 POST）
//! 2. 站点自定义 `playUrl` 模板（含 `{playUrl}` 占位符）
//! 3. 通用页面抽取：直接抓 HTML/JS，从 `var main = "..."`、JSON、base64 等形态里捞直链

use base64::Engine;
use regex::Regex;
use std::sync::OnceLock;

/// 视为「已经是直链」的扩展名
const MEDIA_EXTS: [&str; 8] = ["m3u8", "mp4", "flv", "mkv", "mov", "ts", "mpd", "webm"];

/// URL 是否已经是媒体直链（无需解析）
pub fn is_media_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    MEDIA_EXTS.iter().any(|e| path.ends_with(&format!(".{e}")))
}

/// 页面是否需要走解析：非媒体直链一律视为需要
pub fn needs_parse(url: &str) -> bool {
    !is_media_url(url)
}

fn re_assign() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // var main = "/a/b/index.m3u8?sign=xx" / url:'xxx' / playUrl = "xxx"
        Regex::new(
            r#"(?i)(?:var|let|const)?\s*["']?\b(main|url|playurl|play_url|video|videourl|vurl|source|src|file|m3u8)\b["']?\s*[:=]\s*["']([^"']{4,})["']"#,
        )
        .unwrap()
    })
}

fn re_abs() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)["'](https?://[^"'\s\\]+?\.(?:m3u8|mp4|flv|mkv|mov|mpd)(?:\?[^"'\s\\]*)?)["']"#)
            .unwrap()
    })
}

fn re_rel() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)["']([^"'\s\\]{2,}?\.(?:m3u8|mp4|flv|mkv|mov|mpd)(?:\?[^"'\s\\]*)?)["']"#)
            .unwrap()
    })
}

fn re_bare() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(https?://[^\s"'<>\\]+?\.(?:m3u8|mp4|flv|mkv|mov|mpd)(?:\?[^\s"'<>\\]*)?)"#)
            .unwrap()
    })
}

/// 把 JS/JSON 里的转义还原：`\/` → `/`、`\u0026` → `&`、`&amp;` → `&`
pub fn unescape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\\' && i + 1 < bytes.len() {
            let n = bytes[i + 1];
            if n == '/' {
                out.push('/');
                i += 2;
                continue;
            }
            if n == 'u' && i + 5 < bytes.len() {
                let hex: String = bytes[i + 2..i + 6].iter().collect();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                        i += 6;
                        continue;
                    }
                }
            }
            out.push(n);
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.replace("&amp;", "&")
}

/// 相对路径转绝对（不引入 url crate，手写够用）
pub fn absolutize(base: &str, rel: &str) -> String {
    let rel = rel.trim();
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    if let Ok(b) = reqwest::Url::parse(base) {
        if let Ok(u) = b.join(rel) {
            return u.to_string();
        }
    }
    rel.to_string()
}

/// 从解析接口响应里取直链：支持 JSON（url/data/playUrl）、纯文本直链、HTML
pub fn parse_jx_response(body: &str, api_url: &str) -> Option<String> {
    let text = unescape_js(body);
    let trimmed = text.trim();

    // 1) 直接就是 m3u8 清单
    if trimmed.starts_with("#EXTM3U") {
        return Some(api_url.to_string());
    }

    // 2) JSON 结构
    let json_start = trimmed.find('{').or_else(|| trimmed.find('['));
    if let Some(start) = json_start {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&trimmed[start..]) {
            if let Some(u) = json_pick_url(&v, 0) {
                return Some(absolutize(api_url, &u));
            }
        }
    }

    // 3) 普通网页 / JS
    extract_media(&text, api_url)
}

fn json_pick_url(v: &serde_json::Value, depth: u32) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match v {
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.starts_with("http://") || s.starts_with("https://") || s.starts_with('#') {
                if s.starts_with("#EXTM3U") {
                    return None;
                }
                if is_media_url(s) || s.contains(".m3u8") || s.contains(".mp4") {
                    return Some(s.to_string());
                }
            }
            None
        }
        serde_json::Value::Array(a) => a.iter().find_map(|x| json_pick_url(x, depth + 1)),
        serde_json::Value::Object(o) => {
            for k in ["url", "playUrl", "play_url", "data", "src", "video", "m3u8", "result"] {
                if let Some(x) = o.get(k) {
                    if let Some(u) = json_pick_url(x, depth + 1) {
                        return Some(u);
                    }
                }
            }
            o.values().find_map(|x| json_pick_url(x, depth + 1))
        }
        _ => None,
    }
}

/// 通用抽取：从任意 HTML/JS 文本里找媒体直链
pub fn extract_media(text: &str, base_url: &str) -> Option<String> {
    let text = unescape_js(text);

    // a) 变量赋值形态（最贴近播放页）
    for cap in re_assign().captures_iter(&text) {
        let val = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        if val.len() > 4 && (val.contains('/') || val.contains('.')) {
            let cand = absolutize(base_url, val);
            if is_media_url(&cand) {
                return Some(cand);
            }
        }
    }

    // b) 引号内的绝对地址
    if let Some(m) = re_abs().captures(&text) {
        return Some(absolutize(base_url, m.get(1)?.as_str()));
    }

    // c) 引号内的相对地址
    for cap in re_rel().captures_iter(&text) {
        let cand = absolutize(base_url, cap.get(1)?.as_str());
        if is_media_url(&cand) {
            return Some(cand);
        }
    }

    // d) 裸地址
    if let Some(m) = re_bare().captures(&text) {
        return Some(m.get(1)?.as_str().to_string());
    }

    // e) base64 片段（部分站点把地址编码后塞进变量）
    extract_base64_media(&text, base_url)
}

fn extract_base64_media(text: &str, base_url: &str) -> Option<String> {
    static RE_B64: OnceLock<Regex> = OnceLock::new();
    let re = RE_B64.get_or_init(|| Regex::new(r#""([A-Za-z0-9+/]{24,}={0,2})""#).unwrap());
    for cap in re.captures_iter(text).take(20) {
        let raw = cap.get(1)?.as_str();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw) {
            if let Ok(s) = String::from_utf8(bytes) {
                if is_media_url(&s) || s.contains(".m3u8") {
                    return Some(absolutize(base_url, s.trim()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_media_url() {
        assert!(is_media_url("https://a/b/index.m3u8?sign=1"));
        assert!(is_media_url("https://a/b/1.mp4"));
        assert!(!is_media_url("https://v.lzcdn28.com/share/abc"));
        assert!(!is_media_url("https://a/b/index.html"));
    }

    #[test]
    fn test_extract_var_main_relative() {
        // 量子分享页的真实结构
        let html = r#"<script>
            var videoid = "0a0a0c8aaa00ade50f74a3f0ca981ed7";
            var main = "/20250918/1173_8785e8d6/index.m3u8?sign=7fc463fd";
            var xml = "/20250918/1173_8785e8d6/index.xml?sign=7fc463fd";
        </script>"#;
        let got = extract_media(html, "https://v.lzcdn28.com/share/0a0a0c8aaa00ade50f74a3f0ca981ed7");
        assert_eq!(
            got.as_deref(),
            Some("https://v.lzcdn28.com/20250918/1173_8785e8d6/index.m3u8?sign=7fc463fd")
        );
    }

    #[test]
    fn test_extract_json_escaped() {
        let body = r#"{"code":200,"url":"https:\/\/cdn.x.com\/a\/index.m3u8?t=1"}"#;
        let got = parse_jx_response(body, "https://api.x.com/jx/");
        assert_eq!(got.as_deref(), Some("https://cdn.x.com/a/index.m3u8?t=1"));
    }

    #[test]
    fn test_extract_abs_quoted() {
        let html = r#"<video src="https://cdn.x.com/play/abc.mp4"></video>"#;
        let got = extract_media(html, "https://page.x.com/1");
        assert_eq!(got.as_deref(), Some("https://cdn.x.com/play/abc.mp4"));
    }

    #[test]
    fn test_unescape() {
        assert_eq!(unescape_js(r"a\/b\u0026c&amp;d"), "a/b&c&d");
    }
}
