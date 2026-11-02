//! 音频标签：优先读内嵌标签，读不到再由调用方回退到文件名。

use std::path::Path;

use lofty::file::AudioFile;
use lofty::prelude::{Accessor, TaggedFileExt};
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct AudioTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<u32>,
    pub duration: f64,
    pub cover: Option<Vec<u8>>,
    pub cover_mime: String,
}

impl AudioTags {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.artist.is_empty() && self.album.is_empty()
    }
}

pub fn read(path: &Path) -> Option<AudioTags> {
    let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
    let duration = tagged.properties().duration().as_secs_f64();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let mut out = AudioTags {
        duration,
        ..Default::default()
    };

    if let Some(tag) = tag {
        out.title = tag.title().map(|s| s.to_string()).unwrap_or_default();
        out.artist = tag.artist().map(|s| s.to_string()).unwrap_or_default();
        out.album = tag.album().map(|s| s.to_string()).unwrap_or_default();
        out.album_artist = tag
            .get_string(&lofty::tag::ItemKey::AlbumArtist)
            .unwrap_or_default()
            .to_string();
        out.genre = tag.genre().map(|s| s.to_string()).unwrap_or_default();
        out.track = tag.track();
        out.disc = tag.disk();
        out.year = tag.year();
        if let Some(pic) = tag.pictures().first() {
            let mime = pic
                .mime_type()
                .map(|m| m.to_string())
                .unwrap_or_else(|| "image/jpeg".to_string());
            out.cover = Some(pic.data().to_vec());
            out.cover_mime = mime;
        }
    }

    Some(out)
}
