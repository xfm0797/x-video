//! 直播清单解析：M3U / M3U8 与「频道名,地址」TXT 两种常见格式。

use serde::{Deserialize, Serialize};

use crate::http;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveChannel {
    pub name: String,
    pub url: String,
    pub logo: Option<String>,
    pub tvg_id: Option<String>,
    /// 频道自带的请求头（#EXTVLCOPT 的 referrer / user-agent）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveGroup {
    pub name: String,
    pub channels: Vec<LiveChannel>,
}

fn ensure_group<'a>(groups: &'a mut Vec<LiveGroup>, name: &str) -> &'a mut LiveGroup {
    let key = if name.trim().is_empty() { "未分组" } else { name.trim() };
    if let Some(i) = groups.iter().position(|g| g.name == key) {
        return &mut groups[i];
    }
    groups.push(LiveGroup {
        name: key.to_string(),
        channels: Vec::new(),
    });
    groups.last_mut().unwrap()
}

/// `#EXTINF:-1 tvg-logo=".." group-title="..",频道名`
pub fn parse_m3u(text: &str) -> Vec<LiveGroup> {
    let mut groups: Vec<LiveGroup> = Vec::new();
    let mut pending: Option<(LiveChannel, String)> = None;
    let mut pending_opts: Vec<String> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (attrs, title) = split_extinf(rest);
            let group = attr(&attrs, "group-title").unwrap_or_default();
            let ch = LiveChannel {
                name: title.trim().to_string(),
                url: String::new(),
                logo: attr(&attrs, "tvg-logo"),
                tvg_id: attr(&attrs, "tvg-id").or_else(|| attr(&attrs, "tvg-name")),
                headers: None,
            };
            pending = Some((ch, group));
            pending_opts.clear();
            continue;
        }
        if let Some(opt) = line.strip_prefix("#EXTVLCOPT:") {
            pending_opts.push(opt.trim().to_string());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some((mut ch, group)) = pending.take() else {
            continue; // 没有 EXTINF 的裸地址，跳过
        };
        if !line.contains("://") {
            continue;
        }
        ch.url = line.to_string();
        if ch.name.is_empty() {
            ch.name = fallback_name(&ch.url);
        }
        if !pending_opts.is_empty() {
            let fields: Vec<String> = pending_opts
                .iter()
                .filter_map(|o| {
                    let (k, v) = o.split_once('=')?;
                    let k = k.trim().to_ascii_lowercase();
                    match k.as_str() {
                        "http-referrer" | "http-referer" => Some(format!("Referer: {}", v.trim())),
                        "http-user-agent" => Some(format!("User-Agent: {}", v.trim())),
                        _ => None,
                    }
                })
                .collect();
            if !fields.is_empty() {
                ch.headers = Some(fields.join(","));
            }
            pending_opts.clear();
        }
        ensure_group(&mut groups, &group).channels.push(ch);
    }
    groups.retain(|g| !g.channels.is_empty());
    groups
}

/// `分组名,#genre#` 切分组，`频道名,url1#url2` 是频道行。
pub fn parse_txt(text: &str) -> Vec<LiveGroup> {
    let mut groups: Vec<LiveGroup> = Vec::new();
    let mut current = String::from("未分组");

    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_suffix(",#genre#") {
            if !name.trim().is_empty() {
                current = name.trim().to_string();
            }
            continue;
        }
        let Some((name, urls)) = line.split_once(',') else {
            continue;
        };
        let name = name.trim();
        for url in urls.split('#') {
            let url = url.trim();
            if !url.contains("://") {
                continue;
            }
            ensure_group(&mut groups, &current).channels.push(LiveChannel {
                name: if name.is_empty() { fallback_name(url) } else { name.to_string() },
                url: url.to_string(),
                logo: None,
                tvg_id: None,
                headers: None,
            });
        }
    }
    groups.retain(|g| !g.channels.is_empty());
    groups
}

pub fn parse_auto(text: &str) -> Vec<LiveGroup> {
    if text.contains("#EXTINF") {
        parse_m3u(text)
    } else {
        parse_txt(text)
    }
}

/// 拉取一组直播源地址（TVBox 的 `lives[].url` 可能是多个地址用 `#` 连接）。
pub async fn load(
    client: &reqwest::Client,
    sources: &[(String, String)],
    referer: Option<&str>,
) -> Vec<LiveGroup> {
    let mut out: Vec<LiveGroup> = Vec::new();
    for (label, url) in sources {
        let real = unwrap_proxy(url);
        if real.is_empty() {
            continue;
        }
        match http::fetch_text(client, &real, referer).await {
            Ok(f) => {
                let mut groups = parse_auto(&f.text);
                // 只有一组且没有分组名时，用 lives 里的名字兜底
                if groups.len() == 1 && (groups[0].name == "未分组" || groups[0].name.is_empty()) {
                    if !label.trim().is_empty() {
                        groups[0].name = label.trim().to_string();
                    }
                }
                out.extend(groups);
            }
            Err(e) => eprintln!("[live] {label} 拉取失败：{e:#}"),
        }
    }
    out
}

/// TVBox 的 `proxy://do=live&type=txt&ext=<真实地址>`
pub fn unwrap_proxy(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("proxy://") {
        for part in rest.split('&') {
            if let Some(ext) = part.strip_prefix("ext=") {
                return percent_decode(ext);
            }
        }
        return String::new();
    }
    url.trim().to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn fallback_name(url: &str) -> String {
    url.rsplit('/')
        .find(|s| !s.is_empty() && !s.contains('?'))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "未命名".to_string())
}

/// 属性段与标题的分界：最后一个不在引号内的逗号。
fn split_extinf(s: &str) -> (String, String) {
    let mut in_quotes = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => return (s[..i].to_string(), s[i + 1..].to_string()),
            _ => {}
        }
    }
    (s.to_string(), String::new())
}

fn attr(attrs: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = attrs.find(&needle)? + needle.len();
    let rest = &attrs[start..];
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        let v = stripped[..end].trim();
        return (!v.is_empty()).then(|| v.to_string());
    }
    let end = rest.find([' ', ',']).unwrap_or(rest.len());
    let v = rest[..end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_groups_and_opts() {
        let text = "\u{feff}#EXTM3U\n\
            #EXTINF:-1 tvg-logo=\"http://a/1.png\" group-title=\"央视\",CCTV-1\n\
            #EXTVLCOPT:http-referrer=http://a/\n\
            http://a/1.m3u8\n\
            #EXTINF:-1 group-title=\"卫视\",湖南卫视\n\
            http://b/2.m3u8\n";
        let g = parse_m3u(text);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].name, "央视");
        assert_eq!(g[0].channels[0].name, "CCTV-1");
        assert_eq!(g[0].channels[0].logo.as_deref(), Some("http://a/1.png"));
        assert_eq!(g[0].channels[0].headers.as_deref(), Some("Referer: http://a/"));
        assert_eq!(g[1].channels[0].name, "湖南卫视");
    }

    #[test]
    fn txt_groups() {
        let text = "央视频道,#genre#\nCCTV-1,http://a/1.m3u8\nCCTV-2,http://a/2.m3u8#http://a/2b.m3u8\n\
                    地方,#genre#\n北京卫视,http://b/1.m3u8\n";
        let g = parse_txt(text);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].channels.len(), 3);
        assert_eq!(g[1].name, "地方");
    }

    #[test]
    fn proxy_url_unwrapped() {
        let u = "proxy://do=live&type=txt&ext=http%3A%2F%2Fa%2Flive.txt";
        assert_eq!(unwrap_proxy(u), "http://a/live.txt");
    }

    #[test]
    fn attrs_without_quotes() {
        let (attrs, title) = split_extinf("-1 tvg-id=abc group-title=卫视,北京卫视");
        assert_eq!(attr(&attrs, "group-title").as_deref(), Some("卫视"));
        assert_eq!(title, "北京卫视");
    }
}
