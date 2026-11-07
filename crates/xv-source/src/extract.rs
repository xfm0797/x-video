//! 网页抽取：把「分享页 / 播放页」还原成媒体直链。
//!
//! 站点给的 `vod_play_url` 经常不是直链而是 `/share/xxxx` 这种页面，
//! 页面里再用 `var main = "/2025.../index.m3u8?sign=..."` 之类的变量引用真实地址。
//! 这里按「变量赋值 → 引号内绝对地址 → 引号内相对地址 → 裸地址 → base64」逐级降级。

use base64::Engine;
use regex::Regex;
use std::sync::OnceLock;

const MEDIA_EXTS: [&str; 8] = ["m3u8", "mp4", "flv", "mkv", "mov", "ts", "mpd", "webm"];

pub fn is_media(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    MEDIA_EXTS.iter().any(|e| path.ends_with(&format!(".{e}")))
}

fn re_assign() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(?:var|let|const)?\s*["']?\b(main|url|playurl|play_url|video|videourl|vurl|source|src|file|m3u8)\b["']?\s*[:=]\s*["']([^"']{4,})["']"#,
        )
        .unwrap()
    })
}

fn re_abs() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)["'](https?://[^"'\s\\]+?\.(?:m3u8|mp4|flv|mkv|mov|mpd)(?:\?[^"'\s\\]*)?)["']"#,
        )
        .unwrap()
    })
}

fn re_rel() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)["'](/?[^"'\s\\]{2,}?\.(?:m3u8|mp4|flv|mkv|mov|mpd)(?:\?[^"'\s\\]*)?)["']"#)
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

fn re_b64() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"["']([A-Za-z0-9+/]{24,}={0,2})["']"#).unwrap())
}

/// JS/JSON 转义还原：`\/`、`\u0026`、`&amp;`
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '/' => {
                    out.push('/');
                    i += 2;
                    continue;
                }
                'u' if i + 5 < chars.len() => {
                    let hex: String = chars[i + 2..i + 6].iter().collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                            i += 6;
                            continue;
                        }
                    }
                }
                c => {
                    out.push(c);
                    i += 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.replace("&amp;", "&")
}

pub fn absolutize(base: &str, rel: &str) -> String {
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    reqwest::Url::parse(base)
        .and_then(|b| b.join(rel))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| rel.to_string())
}

/// 从页面/JS 文本里找媒体直链。
pub fn find_media(text: &str, base: &str) -> Option<String> {
    let text = unescape(text);

    for cap in re_assign().captures_iter(&text) {
        let val = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        if val.len() < 5 {
            continue;
        }
        let cand = absolutize(base, val);
        if is_media(&cand) {
            return Some(cand);
        }
    }
    if let Some(m) = re_abs().captures(&text) {
        return Some(m.get(1)?.as_str().to_string());
    }
    for cap in re_rel().captures_iter(&text) {
        let val = cap.get(1)?.as_str();
        if val.starts_with("//") {
            continue;
        }
        let cand = absolutize(base, val);
        if is_media(&cand) {
            return Some(cand);
        }
    }
    if let Some(m) = re_bare().captures(&text) {
        return Some(m.get(1)?.as_str().to_string());
    }
    for cap in re_b64().captures_iter(&text) {
        let raw = cap.get(1)?.as_str();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())?;
        if let Some(found) = find_media(&decoded, base) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_relative_main_var() {
        let html = r#"<script>var main = "/20250923/2204_417d8bba/index.m3u8?sign=8e7d";</script>"#;
        assert_eq!(
            find_media(html, "https://v.lzcdn28.com/share/abc").as_deref(),
            Some("https://v.lzcdn28.com/20250923/2204_417d8bba/index.m3u8?sign=8e7d")
        );
    }

    #[test]
    fn finds_escaped_absolute() {
        let html = r#"player_aaaa={"url":"https:\/\/cdn.x\/a\/index.m3u8"}"#;
        assert_eq!(find_media(html, "https://x/p/1").as_deref(), Some("https://cdn.x/a/index.m3u8"));
    }

    #[test]
    fn finds_base64_payload() {
        let src = "https://cdn.y/b/index.m3u8?x=1";
        let encoded = base64::engine::general_purpose::STANDARD.encode(src);
        let html = format!(r#"var cfg = "{encoded}";"#);
        assert_eq!(find_media(&html, "https://x/").as_deref(), Some(src));
    }

    #[test]
    fn ignores_plain_pages() {
        assert!(find_media("<html>hello</html>", "https://x/").is_none());
    }
}
