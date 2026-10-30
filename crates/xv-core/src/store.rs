use crate::model::Source;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XVIDEO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from("./xvideo-data")
}

fn sources_file() -> PathBuf {
    data_dir().join("sources.json")
}

pub fn load_sources() -> Vec<Source> {
    let path = sources_file();
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_sources(sources: &[Source]) -> Result<()> {
    let dir = data_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    let json = serde_json::to_string_pretty(sources)?;
    std::fs::write(sources_file(), json)?;
    Ok(())
}

pub fn new_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("src_{}_{:04}", now.as_secs(), now.subsec_micros() % 10000)
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}
