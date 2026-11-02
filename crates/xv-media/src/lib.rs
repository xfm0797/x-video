//! 媒体识别：文件类型、文件名解析、音频标签。
//!
//! 这里是纯逻辑层，不碰数据库也不碰播放器，便于单测与复用。

pub mod name;
pub mod tags;

use serde::{Deserialize, Serialize};

pub use name::{clean_stem, looks_like_episode, parse_stem, ParsedName};
pub use tags::AudioTags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Video,
    Audio,
    Subtitle,
    Other,
}

const VIDEO: &[&str] = &[
    "mp4", "mkv", "avi", "flv", "wmv", "mov", "ts", "m2ts", "mts", "mpg", "mpeg", "rmvb", "rm",
    "webm", "ogv", "3gp", "asf", "vob", "m4v", "f4v", "divx", "m2v", "tp", "trp", "dat",
];

const AUDIO: &[&str] = &[
    "mp3", "flac", "ape", "wav", "m4a", "aac", "ogg", "opus", "wma", "dff", "dsf", "mka", "aiff",
    "aif", "alac", "wv", "tta",
];

const SUBTITLE: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup"];

pub fn classify(ext: &str) -> FileKind {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    if VIDEO.contains(&e.as_str()) {
        FileKind::Video
    } else if AUDIO.contains(&e.as_str()) {
        FileKind::Audio
    } else if SUBTITLE.contains(&e.as_str()) {
        FileKind::Subtitle
    } else {
        FileKind::Other
    }
}

pub fn is_video(path: &str) -> bool {
    classify(ext_of(path)) == FileKind::Video
}

pub fn is_audio(path: &str) -> bool {
    classify(ext_of(path)) == FileKind::Audio
}

pub fn ext_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, e)| e).unwrap_or("")
}

/// 竖屏判定：短剧几乎都是 9:16，宽高比小于 0.8 就按短剧排版。
pub fn is_vertical(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && (width as f64 / height as f64) < 0.8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_common() {
        assert_eq!(classify("MKV"), FileKind::Video);
        assert_eq!(classify("flac"), FileKind::Audio);
        assert_eq!(classify("ass"), FileKind::Subtitle);
        assert_eq!(classify("txt"), FileKind::Other);
    }

    #[test]
    fn vertical_ratio() {
        assert!(is_vertical(1080, 1920));
        assert!(!is_vertical(1920, 1080));
        assert!(!is_vertical(0, 0));
    }
}
