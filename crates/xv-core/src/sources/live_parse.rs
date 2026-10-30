use crate::model::{LiveChannel, LiveGroup};
use anyhow::Result;

/// 解析 M3U/M3U8 直播列表（IPTV 格式）
/// 支持 #EXTINF 属性：tvg-name / tvg-logo / group-title / tvg-id
pub fn parse_m3u(text: &str) -> Result<Vec<LiveGroup>> {
    let mut groups: Vec<LiveGroup> = Vec::new();
    let mut pending: Option<LiveChannel> = None;
    let mut pending_group = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(attr_part) = line.strip_prefix("#EXTINF:") {
            // #EXTINF:-1 tvg-name="x" tvg-logo="y" group-title="g",标题
            let (attrs, title) = split_extinf(attr_part);
            pending_group = get_attr(&attrs, "group-title").unwrap_or_default();
            pending = Some(LiveChannel {
                name: title.trim().to_string(),
                url: String::new(),
                logo: get_attr(&attrs, "tvg-logo"),
                tvg_id: get_attr(&attrs, "tvg-id").or_else(|| get_attr(&attrs, "tvg-name")),
            });
        } else if line.starts_with('#') {
            // 其他指令（#EXTVLCOPT、#EXT-X-STREAM-INF 等），忽略
            continue;
        } else if let Some(mut ch) = pending.take() {
            ch.url = line.to_string();
            if ch.name.is_empty() {
                ch.name = "未命名频道".into();
            }
            if !ch.url.contains("://") {
                continue;
            }
            let group_name = if pending_group.trim().is_empty() {
                "未分组"
            } else {
                pending_group.trim()
            };
            let group = ensure_group(&mut groups, group_name);
            group.channels.push(ch);
        }
    }
    Ok(groups)
}

/// 解析 TVBox TXT 直播格式：
///   分组名,#genre#
///   频道名,url1#url2
pub fn parse_txt_live(text: &str) -> Result<Vec<LiveGroup>> {
    let mut groups: Vec<LiveGroup> = Vec::new();
    let mut current: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.contains(",#genre#") {
            let name = line.trim_end_matches(",#genre#").trim().to_string();
            if !name.is_empty() {
                current = Some(name);
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        // 频道行：名称,url1#url2
        if let Some((name, urls)) = line.split_once(',') {
            let group_name = current.clone().unwrap_or_else(|| "未分组".into());
            let group = ensure_group(&mut groups, &group_name);
            for url in urls.split('#') {
                let url = url.trim();
                if url.is_empty() || !url.contains("://") {
                    continue;
                }
                group.channels.push(LiveChannel {
                    name: name.trim().to_string(),
                    url: url.to_string(),
                    logo: None,
                    tvg_id: None,
                });
            }
        }
    }
    Ok(groups)
}

pub fn ensure_group<'a>(groups: &'a mut Vec<LiveGroup>, name: &str) -> &'a mut LiveGroup {
    if let Some(idx) = groups.iter().position(|g| g.name == name) {
        return &mut groups[idx];
    }
    groups.push(LiveGroup {
        name: name.to_string(),
        channels: Vec::new(),
    });
    groups.last_mut().unwrap()
}

/// 提取 EXTINF 内的属性段与标题（标题在最后一个未被引号包裹的逗号之后）
fn split_extinf(s: &str) -> (String, String) {
    let mut in_quotes = false;
    let mut split_idx = None;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                split_idx = Some(i);
                break;
            }
            _ => {}
        }
    }
    match split_idx {
        Some(i) => (s[..i].to_string(), s[i + 1..].to_string()),
        None => (s.to_string(), String::new()),
    }
}

fn get_attr(attrs: &str, key: &str) -> Option<String> {
    let pattern = format!("{}=\"", key);
    let start = attrs.find(&pattern)? + pattern.len();
    let rest = &attrs[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_m3u() {
        let text = "#EXTM3U\n#EXTINF:-1 tvg-id=\"cctv1\" tvg-logo=\"a.png\" group-title=\"央视\",CCTV-1\nhttp://a/1.m3u8\n#EXTINF:-1 group-title=\"卫视\",湖南卫视\nhttp://a/2.m3u8\n";
        let groups = parse_m3u(text).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "央视");
        assert_eq!(groups[0].channels[0].name, "CCTV-1");
        assert_eq!(groups[0].channels[0].tvg_id.as_deref(), Some("cctv1"));
        assert_eq!(groups[1].channels[0].name, "湖南卫视");
    }

    #[test]
    fn test_txt_live() {
        let text = "央视,#genre#\nCCTV-1,http://a/1.m3u8#http://b/1.m3u8\n卫视,#genre#\n湖南卫视,http://a/2.m3u8\n";
        let groups = parse_txt_live(text).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].channels.len(), 2);
        assert_eq!(groups[0].channels[0].url, "http://a/1.m3u8");
        assert_eq!(groups[1].channels[0].name, "湖南卫视");
    }
}
