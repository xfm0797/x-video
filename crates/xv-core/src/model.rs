use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// M3U/M3U8 直播列表（IPTV）
    M3u,
    /// TXT 格式直播列表（含 TVBox lives 内置 txt）
    Txt,
    /// TVBox 配置（点播 sites + 直播 lives）
    Tvbox,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::M3u => "m3u",
            SourceKind::Txt => "txt",
            SourceKind::Tvbox => "tvbox",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: SourceKind,
    pub url: String,
    pub enabled: bool,
    pub added_at: u64,
    /// 上次刷新解析出的直播频道数（m3u/txt）
    #[serde(default)]
    pub channel_count: u32,
    /// 上次刷新解析出的点播站点数（tvbox）
    #[serde(default)]
    pub site_count: u32,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveChannel {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvg_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveGroup {
    pub name: String,
    pub channels: Vec<LiveChannel>,
}

/// TVBox 配置中的点播站点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TvBoxSite {
    pub key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub site_type: i32,
    pub api: String,
    #[serde(default)]
    pub searchable: Option<i32>,
    #[serde(default)]
    pub ext: Option<serde_json::Value>,
    #[serde(default)]
    pub header: Option<serde_json::Value>,
    #[serde(default)]
    pub jar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TvBoxLive {
    pub name: String,
    #[serde(rename = "type")]
    pub live_type: Option<i32>,
    pub url: String,
    #[serde(default)]
    pub epg: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TvBoxConfig {
    #[serde(default)]
    pub spider: Option<String>,
    #[serde(default)]
    pub sites: Vec<TvBoxSite>,
    #[serde(default)]
    pub lives: Vec<TvBoxLive>,
}

/// 点播分类
#[derive(Debug, Clone, Serialize)]
pub struct VodCategory {
    pub type_id: String,
    pub type_name: String,
}

/// 点播影片条目
#[derive(Debug, Clone, Serialize)]
pub struct VodItem {
    pub vod_id: String,
    pub vod_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_pic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_remarks: Option<String>,
}

/// 影片详情
#[derive(Debug, Clone, Serialize)]
pub struct VodDetail {
    pub vod_id: String,
    pub vod_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_pic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_year: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_area: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_director: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_content: Option<String>,
    /// 播放源：from 名 -> (集名, url) 列表
    pub play_sources: Vec<PlaySource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaySource {
    pub from: String,
    pub episodes: Vec<(String, String)>,
}

/// 播放地址（已包代理）
#[derive(Debug, Clone, Serialize)]
pub struct PlayUrl {
    pub url: String,
    pub kind: String, // hls | mp4 | other
    pub raw: String,
}
