//! 目录扫描：文件系统 → 媒体库索引。
//!
//! 只做「快探」：路径、大小、mtime、文件名解析。真正的编码/分辨率/时长
//! 由后台探测队列补齐（需要 libmpv），避免扫描阶段把磁盘和 CPU 打满。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use walkdir::WalkDir;

use xv_media::{classify, parse_stem, FileKind};

use crate::Library;

#[derive(Debug, Default, Clone, Serialize)]
pub struct ScanReport {
    pub scanned: u64,
    pub added: u64,
    pub updated: u64,
    pub skipped: u64,
    pub errors: u64,
    pub removed: u64,
    pub elapsed_ms: u64,
}

const SKIP_DIRS: &[&str] = &[
    "$RECYCLE.BIN",
    "System Volume Information",
    "@eaDir",
    "#recycle",
    ".git",
    "node_modules",
    "__MACOSX",
];

const POSTER_NAMES: &[&str] = &[
    "poster", "folder", "cover", "front", "fanart", "thumb", "海报", "封面",
];

fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn poster_in_dir(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<PathBuf> = None;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let ext = p
            .extension()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp")
            && POSTER_NAMES.iter().any(|n| stem.contains(n))
        {
            best = Some(p);
            break;
        }
    }
    best.map(|p| p.to_string_lossy().into_owned())
}

fn dir_title(dir: &Path) -> String {
    let raw = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (clean, _) = xv_media::clean_stem(&raw);
    if clean.is_empty() {
        raw
    } else {
        clean
    }
}

struct SeriesKey {
    title: String,
    kind: &'static str,
    year: Option<i64>,
    poster: Option<String>,
}

fn upsert_series(conn: &Connection, key: &SeriesKey, now: i64) -> Result<i64> {
    let id = conn.query_row(
        r#"
        INSERT INTO series(title, sort_title, kind, year, poster, added_at, updated_at)
        VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6)
        ON CONFLICT(title, kind) DO UPDATE SET
          updated_at = excluded.updated_at,
          year = COALESCE(series.year, excluded.year),
          poster = COALESCE(series.poster, excluded.poster)
        RETURNING id
        "#,
        params![
            key.title,
            key.title.to_lowercase(),
            key.kind,
            key.year,
            key.poster,
            now
        ],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// 扫描一个目录（递归）并写入索引。
pub fn scan_path(lib: &Library, root: &Path) -> Result<ScanReport> {
    let started = Instant::now();
    let mut report = ScanReport::default();
    let now = unix_secs(SystemTime::now());
    let mut poster_cache: HashMap<PathBuf, Option<String>> = HashMap::new();

    let mut guard = lib.conn();
    let tx = guard.transaction()?;

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !(name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()));
            }
            true
        })
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let kind = classify(&ext);
        if !matches!(kind, FileKind::Video | FileKind::Audio) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        if meta.len() < 64 * 1024 {
            report.skipped += 1;
            continue;
        }

        let path_str = path.to_string_lossy().into_owned();
        let size = meta.len() as i64;
        let mtime = meta.modified().map(unix_secs).unwrap_or(0);

        // 未变化就跳过，避免每次全量重写
        let existing: Option<(i64, i64, i64)> = tx
            .query_row(
                "SELECT id, size, mtime FROM item WHERE path = ?1",
                params![path_str],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        if let Some((_, s, m)) = existing {
            if s == size && m == mtime {
                report.skipped += 1;
                continue;
            }
        }

        report.scanned += 1;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = path.parent().unwrap_or(root);
        let parsed = parse_stem(&stem);

        let key = if kind == FileKind::Audio {
            let tags = xv_media::tags::read(path).unwrap_or_default();
            let album = if !tags.album.is_empty() {
                tags.album.clone()
            } else {
                dir_title(dir)
            };
            let artist = if !tags.album_artist.is_empty() {
                tags.album_artist.clone()
            } else {
                tags.artist.clone()
            };
            let title = if artist.is_empty() {
                album
            } else {
                format!("{artist} · {album}")
            };
            SeriesKey {
                title,
                kind: "music",
                year: tags.year.map(|y| y as i64),
                poster: poster_cache
                    .entry(dir.to_path_buf())
                    .or_insert_with(|| poster_in_dir(dir))
                    .clone(),
            }
        } else {
            let title = if parsed.title.is_empty() {
                dir_title(dir)
            } else {
                parsed.title.clone()
            };
            let episodic = parsed.episode.is_some();
            SeriesKey {
                title,
                kind: if episodic { "tv" } else { "movie" },
                year: parsed.year.map(|y| y as i64),
                poster: poster_cache
                    .entry(dir.to_path_buf())
                    .or_insert_with(|| poster_in_dir(dir))
                    .clone(),
            }
        };

        let series_id = match upsert_series(&tx, &key, now) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("[scan] 归集失败 {}：{e}", path.display());
                report.errors += 1;
                continue;
            }
        };

        let result = tx.execute(
            r#"
            INSERT INTO item(path, name, ext, kind, size, mtime, series_id, season, episode, added_at)
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(path) DO UPDATE SET
              size = excluded.size, mtime = excluded.mtime, series_id = excluded.series_id,
              season = excluded.season, episode = excluded.episode
            "#,
            params![
                path_str,
                stem,
                ext,
                if kind == FileKind::Audio { "audio" } else { "video" },
                size,
                mtime,
                series_id,
                parsed.season.map(|v| v as i64),
                parsed.episode.map(|v| v as i64),
                now
            ],
        );

        match result {
            Ok(_) => {
                if existing.is_some() {
                    report.updated += 1;
                } else {
                    report.added += 1;
                }
            }
            Err(e) => {
                eprintln!("[scan] 写入失败 {}：{e}", path.display());
                report.errors += 1;
            }
        }
    }

    // 清理已不存在的条目
    let root_str = root.to_string_lossy().into_owned();
    let mut stmt = tx.prepare("SELECT id, path FROM item WHERE path LIKE ?1")?;
    let stale: Vec<(i64, String)> = stmt
        .query_map(params![format!("{}%", root_str)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .filter_map(Result::ok)
        .filter(|(_, p)| !Path::new(p).exists())
        .collect();
    drop(stmt);
    for (id, _) in &stale {
        tx.execute("DELETE FROM progress WHERE item_id = ?1", params![id])?;
        tx.execute("DELETE FROM item WHERE id = ?1", params![id])?;
        report.removed += 1;
    }

    tx.execute(
        "UPDATE series SET total = (SELECT COUNT(*) FROM item WHERE series_id = series.id)",
        [],
    )?;
    tx.execute(
        "DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM item WHERE series_id IS NOT NULL)",
        [],
    )?;
    // 集数很多又没有任何横屏证据的，基本是竖屏短剧；探测定出竖屏后会再确认一次
    tx.execute(
        r#"UPDATE series SET kind = 'short'
           WHERE kind = 'tv' AND total >= 60
             AND NOT EXISTS (
               SELECT 1 FROM item WHERE series_id = series.id AND probed = 1 AND vertical = 0
             )"#,
        [],
    )?;
    tx.commit()?;

    report.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}
