//! 文件名清洗与集号识别。
//!
//! 目标不是覆盖所有命名法，而是让「下载站产物 + 短剧批量转存」这两类真实
//! 文件名能落到正确的剧集上：先剥噪声标签，再按优先级抽季/集号，最后把
//! 剩下的部分当剧名。剩下判错的，交给详情页的手动合并/拆分兜底。

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

fn re(pattern: &'static str) -> Regex {
    Regex::new(pattern).expect("内置正则应当合法")
}

macro_rules! lazy_re {
    ($name:ident, $pat:literal) => {
        fn $name() -> &'static Regex {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| re($pat))
        }
    };
}

lazy_re!(
    quality,
    r"(?ix)
    \b(
      2160p|1080p|1080i|720p|576p|480p|4k|8k|uhd|hdr10\+?|hdr|dolby[ ._-]?vision|dv
      |web[ ._-]?dl|webrip|web-?dl|blu[ ._-]?ray|bdrip|brrip|bd|hdtv|hdrip|dvdrip|dvd
      |x264|x265|h\.?264|h\.?265|hevc|avc|av1|10bit|8bit|hi10p
      |aac|ac3|eac3|dts(-hd)?|truehd|atmos|ddp?[ .]?[257]\.[01]|flac|opus
      |remux|proper|repack|extended|imax|uncut|hq|web|hdtc|tc
    )\b"
);
lazy_re!(site, r"(?i)(www\.[a-z0-9.\-]+|https?://\S+|magnet:\S+)");
lazy_re!(brackets, r"[\[【（(][^\]】）)]{1,40}[\]】）)]");
lazy_re!(year, r"\b(19[2-9]\d|20[0-4]\d)\b");
lazy_re!(ep_s, r"(?i)s(\d{1,2})[ ._-]?e(\d{1,4})");
lazy_re!(ep_cn, r"第\s*(\d{1,4}|[零一二三四五六七八九十百两]{1,5})\s*[集话話期]");
lazy_re!(ep_en, r"(?i)\bep?[ ._-]?(\d{1,4})\b");
lazy_re!(ep_tail, r"[-_\s.·](\d{1,4})\s*$");
lazy_re!(ep_paren, r"[\[(（](\d{1,4})[\])）]");
lazy_re!(season_cn, r"第\s*(\d{1,2}|[一二三四五六七八九十]{1,3})\s*季");
lazy_re!(season_en, r"(?i)\bseason\s*(\d{1,2})\b");
lazy_re!(group, r"[-–—]\s*[A-Za-z0-9@._]*[A-Za-z][A-Za-z0-9@._]*$");
lazy_re!(cleanup, r"[._]+");
lazy_re!(spaces, r"\s{2,}");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedName {
    /// 清洗后的剧名（可能为空，调用方回退到父目录名）
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub year: Option<u32>,
    /// 被剥掉的噪声标签，仅用于调试/展示
    pub tags: Vec<String>,
}

fn chinese_number(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let digit = |c: char| -> Option<u32> {
        Some(match c {
            '零' | '〇' => 0,
            '一' => 1,
            '二' | '两' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            _ => return None,
        })
    };
    let chars: Vec<char> = s.chars().collect();
    let mut total = 0u32;
    let mut current = 0u32;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '百' => {
                total += current.max(1) * 100;
                current = 0;
            }
            '十' => {
                total += current.max(1) * 10;
                current = 0;
            }
            _ => {
                let d = digit(c)?;
                // 末尾连续数字（如 一三 → 13）按拼接处理
                if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                    current = current * 10 + d;
                } else {
                    current = if current == 0 { d } else { current * 10 + d };
                }
            }
        }
    }
    Some(total + current)
}

/// 把 `Some.Title.2023.1080p.WEB-DL.x264-GRP` 这类名字洗成 `Some Title`。
pub fn clean_stem(stem: &str) -> (String, Vec<String>) {
    let mut tags = Vec::new();
    let mut s = stem.to_string();

    for m in site().find_iter(&s) {
        tags.push(m.as_str().to_string());
    }
    s = site().replace_all(&s, " ").into_owned();

    // 括号内容：年份保留，其余当标签丢掉
    let mut kept_year: Option<String> = None;
    let bracket_hits: Vec<String> = brackets()
        .find_iter(&s)
        .map(|m| m.as_str().to_string())
        .collect();
    for hit in bracket_hits {
        let inner = hit.trim_matches(|c| "[]【】（）()".contains(c)).to_string();
        if let Some(y) = year().find(&inner) {
            kept_year = Some(y.as_str().to_string());
        } else if !inner.is_empty() {
            tags.push(hit.clone());
        }
        s = s.replace(&hit, " ");
    }

    for m in quality().find_iter(&s) {
        tags.push(m.as_str().to_string());
    }
    s = quality().replace_all(&s, " ").into_owned();
    // 压制组名：-OurTV / -GRP
    s = group().replace(&s.trim_end(), "").into_owned();

    s = cleanup().replace_all(&s, " ").into_owned();
    s = s
        .replace(|c: char| matches!(c, '[' | ']' | '【' | '】' | '(' | ')' | '（' | '）'), " ");
    s = spaces().replace_all(&s, " ").into_owned();
    let mut s = s.trim().to_string();

    if let Some(y) = kept_year {
        if !s.contains(&y) {
            s = format!("{s} {y}");
        }
    }
    let s = s.trim_matches(|c: char| c == '-' || c == '_' || c == ' ' || c == '·');
    (s.to_string(), tags)
}

/// 从文件名（不含扩展名）里抽剧名、季号、集号、年份。
pub fn parse_stem(stem: &str) -> ParsedName {
    let (cleaned, tags) = clean_stem(stem);
    let mut out = ParsedName {
        title: cleaned.clone(),
        tags,
        ..Default::default()
    };

    if let Some(m) = year().find(&cleaned) {
        out.year = m.as_str().parse().ok();
    }

    // 年份不参与集号判定，否则「某某 2023」会被当成第 2023 集
    let work = squash(&year().replace_all(&cleaned, " "));
    let mut title = work.clone();

    if let Some(c) = ep_s().captures(&work) {
        out.season = c.get(1).and_then(|x| x.as_str().parse().ok());
        out.episode = c.get(2).and_then(|x| x.as_str().parse().ok());
        title = work[..c.get(0).unwrap().start()].to_string();
    } else if let Some(c) = ep_cn().captures(&work) {
        out.episode = c.get(1).and_then(|x| chinese_number(x.as_str()));
        title = work[..c.get(0).unwrap().start()].to_string();
    } else if let Some(c) = ep_en().captures(&work) {
        out.episode = c.get(1).and_then(|x| x.as_str().parse().ok());
        title = work[..c.get(0).unwrap().start()].to_string();
    } else if let Some(c) = ep_paren().captures(&work) {
        out.episode = c.get(1).and_then(|x| x.as_str().parse().ok());
        title = work[..c.get(0).unwrap().start()].to_string();
    } else if let Some(c) = ep_tail().captures(&work) {
        out.episode = c.get(1).and_then(|x| x.as_str().parse().ok());
        title = work[..c.get(0).unwrap().start()].to_string();
    }

    if out.season.is_none() {
        if let Some(c) = season_cn().captures(&work) {
            out.season = c.get(1).and_then(|x| chinese_number(x.as_str()));
        } else if let Some(c) = season_en().captures(&work) {
            out.season = c.get(1).and_then(|x| x.as_str().parse().ok());
        }
    }

    // 剧名里去掉季号残留与多余分隔符
    title = season_cn().replace_all(&title, " ").into_owned();
    title = season_en().replace_all(&title, " ").into_owned();
    let title = squash(&title);
    out.title = if title.is_empty() { squash(&work) } else { title };
    out
}

fn squash(s: &str) -> String {
    let s = s.replace(|c: char| c.is_whitespace(), " ");
    let s = spaces().replace_all(s.trim(), " ").into_owned();
    s.trim_matches(|c: char| c == '-' || c == '_' || c == '·' || c == ' ')
        .to_string()
}

/// 仅供测试/调试：判断文件名是否像一个「单集」。
pub fn looks_like_episode(stem: &str) -> bool {
    parse_stem(stem).episode.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_with_quality_tags() {
        let p = parse_stem("流浪地球2.2023.2160p.HDR.WEB-DL.H265.10bit-OurTV");
        assert_eq!(p.title, "流浪地球2");
        assert_eq!(p.year, Some(2023));
        assert_eq!(p.episode, None);
    }

    #[test]
    fn tv_episode_sxxexx() {
        let p = parse_stem("The.Last.of.Us.S01E03.1080p.WEB-DL");
        assert_eq!(p.season, Some(1));
        assert_eq!(p.episode, Some(3));
        assert!(p.title.starts_with("The Last of Us"));
    }

    #[test]
    fn short_cn_episode() {
        let p = parse_stem("霸道总裁爱上我 第12集 1080p");
        assert_eq!(p.title, "霸道总裁爱上我");
        assert_eq!(p.episode, Some(12));
    }

    #[test]
    fn short_tail_number() {
        let p = parse_stem("重生之我在古代当首富-087");
        assert_eq!(p.title, "重生之我在古代当首富");
        assert_eq!(p.episode, Some(87));
    }

    #[test]
    fn chinese_numerals() {
        assert_eq!(chinese_number("十二"), Some(12));
        assert_eq!(chinese_number("一百零三"), Some(103));
        assert_eq!(chinese_number("九"), Some(9));
    }

    #[test]
    fn no_false_episode_on_title_digit() {
        let p = parse_stem("阿凡达2");
        assert_eq!(p.episode, None);
    }

    #[test]
    fn site_watermark_removed() {
        let p = parse_stem("【www.abc123.com】庆余年 第二季 第05集");
        assert_eq!(p.episode, Some(5));
        assert_eq!(p.season, Some(2));
        assert_eq!(p.title, "庆余年");
    }
}
