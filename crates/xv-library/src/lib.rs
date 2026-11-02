//! 媒体库：SQLite 索引、扫描归集、查询与播放进度。
//!
//! 归集策略（宁可少归、不可错归）：
//! - 有集号的文件 → 按「剧名 + 类型」归到同一部剧，重复文件按集号覆盖；
//! - 没有集号 → 单文件即一部电影；
//! - 音频 → 按「专辑艺人 / 专辑」归成专辑，标签缺失时回退到父目录名。

pub mod scan;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, Row};
use serde::Serialize;

pub use scan::{scan_path, ScanReport};

#[derive(Debug, Clone, Serialize)]
pub struct SeriesCard {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub year: Option<i32>,
    pub total: i64,
    pub added_at: i64,
    pub updated_at: i64,
    pub watched: i64,
    pub last_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemRow {
    pub id: i64,
    pub series_id: Option<i64>,
    pub name: String,
    pub path: String,
    pub episode: Option<i32>,
    pub season: Option<i32>,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub vertical: bool,
    pub size: i64,
    pub added_at: i64,
    pub position: f64,
    pub finished: bool,
    pub vcodec: String,
    pub acodec: String,
    pub container: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinueRow {
    pub item_id: i64,
    pub series_id: Option<i64>,
    pub series_title: String,
    pub name: String,
    pub episode: Option<i32>,
    pub position: f64,
    pub duration: f64,
    pub updated_at: i64,
    pub vertical: bool,
}

pub struct Library {
    conn: Mutex<Connection>,
    pub data_dir: PathBuf,
    pub thumbs_dir: PathBuf,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Library {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let thumbs_dir = data_dir.join("thumbs");
        std::fs::create_dir_all(&thumbs_dir)?;
        let conn = Connection::open(data_dir.join("library.db"))
            .with_context(|| format!("打开数据库失败：{}", data_dir.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS series(
              id INTEGER PRIMARY KEY,
              title TEXT NOT NULL,
              sort_title TEXT NOT NULL DEFAULT '',
              kind TEXT NOT NULL,
              year INTEGER,
              poster TEXT,
              total INTEGER NOT NULL DEFAULT 0,
              added_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              UNIQUE(title, kind)
            );
            CREATE TABLE IF NOT EXISTS item(
              id INTEGER PRIMARY KEY,
              path TEXT NOT NULL UNIQUE,
              name TEXT NOT NULL,
              ext TEXT NOT NULL,
              kind TEXT NOT NULL,
              size INTEGER NOT NULL,
              mtime INTEGER NOT NULL,
              series_id INTEGER,
              season INTEGER,
              episode INTEGER,
              duration REAL NOT NULL DEFAULT 0,
              width INTEGER NOT NULL DEFAULT 0,
              height INTEGER NOT NULL DEFAULT 0,
              vertical INTEGER NOT NULL DEFAULT 0,
              vcodec TEXT NOT NULL DEFAULT '',
              acodec TEXT NOT NULL DEFAULT '',
              container TEXT NOT NULL DEFAULT '',
              probed INTEGER NOT NULL DEFAULT 0,
              added_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS item_series ON item(series_id, season, episode);
            CREATE INDEX IF NOT EXISTS item_kind ON item(kind);
            CREATE TABLE IF NOT EXISTS progress(
              item_id INTEGER PRIMARY KEY,
              position REAL NOT NULL,
              duration REAL NOT NULL,
              finished INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS root(
              id INTEGER PRIMARY KEY,
              path TEXT NOT NULL UNIQUE,
              added_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS setting(
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            data_dir: data_dir.to_path_buf(),
            thumbs_dir,
        })
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- 根目录 ----------

    pub fn roots(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT path FROM root ORDER BY added_at")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn add_root(&self, path: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO root(path, added_at) VALUES(?1, ?2)",
            params![path, now()],
        )?;
        Ok(())
    }

    pub fn remove_root(&self, path: &str) -> Result<i64> {
        let conn = self.conn();
        conn.execute("DELETE FROM root WHERE path = ?1", params![path])?;
        let n = conn.execute(
            "DELETE FROM item WHERE path LIKE ?1",
            params![format!("{}%", path)],
        )?;
        Ok(n as i64)
    }

    // ---------- 剧集列表 ----------

    pub fn list_series(
        &self,
        kind: Option<&str>,
        query: Option<&str>,
        sort: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SeriesCard>> {
        let order = match sort {
            "title" => "s.title COLLATE NOCASE ASC",
            "added" => "s.added_at DESC",
            _ => "s.updated_at DESC",
        };
        let sql = format!(
            r#"
            SELECT s.id, s.title, s.kind, s.year, s.total, s.added_at, s.updated_at,
              (SELECT COUNT(*) FROM item i JOIN progress p ON p.item_id = i.id
                 WHERE i.series_id = s.id AND p.finished = 1) AS watched,
              COALESCE((SELECT MAX(p.updated_at) FROM item i JOIN progress p ON p.item_id = i.id
                 WHERE i.series_id = s.id), 0) AS last_at
            FROM series s
            WHERE (?1 IS NULL OR s.kind = ?1)
              AND (?2 IS NULL OR s.title LIKE ?2)
            ORDER BY {order}
            LIMIT ?3 OFFSET ?4
            "#
        );
        let like = query.map(|q| format!("%{}%", q.trim()));
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![kind, like, limit, offset], map_series)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn series_count(&self, kind: Option<&str>) -> Result<i64> {
        let conn = self.conn();
        let n = conn.query_row(
            "SELECT COUNT(*) FROM series WHERE (?1 IS NULL OR kind = ?1)",
            params![kind],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn series(&self, id: i64) -> Result<Option<(SeriesCard, Vec<ItemRow>)>> {
        let conn = self.conn();
        let card = conn
            .query_row(
                r#"
                SELECT s.id, s.title, s.kind, s.year, s.total, s.added_at, s.updated_at,
                  (SELECT COUNT(*) FROM item i JOIN progress p ON p.item_id = i.id
                     WHERE i.series_id = s.id AND p.finished = 1) AS watched,
                  COALESCE((SELECT MAX(p.updated_at) FROM item i JOIN progress p ON p.item_id = i.id
                     WHERE i.series_id = s.id), 0) AS last_at
                FROM series s WHERE s.id = ?1
                "#,
                params![id],
                map_series,
            )
            .ok();
        let Some(card) = card else {
            return Ok(None);
        };
        drop(conn);
        let items = self.items_of_series(id)?;
        Ok(Some((card, items)))
    }

    /// 首页「继续观看」：未看完且进度超过 5 秒的记录。
    pub fn continue_watching(&self, limit: i64) -> Result<Vec<ContinueRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            r#"
            SELECT p.item_id, i.series_id, COALESCE(s.title, i.name), i.name, i.episode,
                   p.position, p.duration, p.updated_at, i.vertical
            FROM progress p
            JOIN item i ON i.id = p.item_id
            LEFT JOIN series s ON s.id = i.series_id
            WHERE p.finished = 0 AND p.position > 5
              AND (p.duration <= 0 OR p.position < p.duration * 0.97)
            ORDER BY p.updated_at DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(ContinueRow {
                item_id: r.get(0)?,
                series_id: r.get(1)?,
                series_title: r.get(2)?,
                name: r.get(3)?,
                episode: r.get(4)?,
                position: r.get(5)?,
                duration: r.get(6)?,
                updated_at: r.get(7)?,
                vertical: r.get::<_, i64>(8)? != 0,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn recent_items(&self, limit: i64) -> Result<Vec<ItemRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            r#"
            SELECT i.id, i.series_id, i.name, i.path, i.episode, i.season, i.duration,
                   i.width, i.height, i.vertical, i.size, i.added_at,
                   COALESCE(p.position, 0), COALESCE(p.finished, 0),
                   i.vcodec, i.acodec, i.container
            FROM item i LEFT JOIN progress p ON p.item_id = i.id
            ORDER BY i.added_at DESC, i.id DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit], map_item)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn item(&self, id: i64) -> Result<Option<ItemRow>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                r#"
                SELECT i.id, i.series_id, i.name, i.path, i.episode, i.season, i.duration,
                       i.width, i.height, i.vertical, i.size, i.added_at,
                       COALESCE(p.position, 0), COALESCE(p.finished, 0),
                       i.vcodec, i.acodec, i.container
                FROM item i LEFT JOIN progress p ON p.item_id = i.id
                WHERE i.id = ?1
                "#,
                params![id],
                map_item,
            )
            .ok();
        Ok(row)
    }

    pub fn item_by_path(&self, path: &str) -> Result<Option<ItemRow>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                r#"
                SELECT i.id, i.series_id, i.name, i.path, i.episode, i.season, i.duration,
                       i.width, i.height, i.vertical, i.size, i.added_at,
                       COALESCE(p.position, 0), COALESCE(p.finished, 0),
                       i.vcodec, i.acodec, i.container
                FROM item i LEFT JOIN progress p ON p.item_id = i.id
                WHERE i.path = ?1
                "#,
                params![path],
                map_item,
            )
            .ok();
        Ok(row)
    }

    pub fn items_of_series(&self, series_id: i64) -> Result<Vec<ItemRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            r#"
            SELECT i.id, i.series_id, i.name, i.path, i.episode, i.season, i.duration,
                   i.width, i.height, i.vertical, i.size, i.added_at,
                   COALESCE(p.position, 0), COALESCE(p.finished, 0),
                   i.vcodec, i.acodec, i.container
            FROM item i LEFT JOIN progress p ON p.item_id = i.id
            WHERE i.series_id = ?1
            ORDER BY i.season IS NULL, i.season, i.episode IS NULL, i.episode, i.name
            "#,
        )?;
        let rows = stmt.query_map(params![series_id], map_item)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn series_of_item(&self, item_id: i64) -> Result<Vec<ItemRow>> {
        let sid: Option<i64> = {
            let conn = self.conn();
            conn.query_row("SELECT series_id FROM item WHERE id = ?1", params![item_id], |r| {
                r.get(0)
            })
            .ok()
            .flatten()
        };
        match sid {
            Some(s) => self.items_of_series(s),
            None => Ok(Vec::new()),
        }
    }

    // ---------- 进度 ----------

    pub fn save_progress(&self, item_id: i64, position: f64, duration: f64) -> Result<()> {
        if position < 1.0 {
            return Ok(());
        }
        let finished = duration > 0.0 && position >= duration * 0.97;
        let conn = self.conn();
        conn.execute(
            r#"
            INSERT INTO progress(item_id, position, duration, finished, updated_at)
            VALUES(?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(item_id) DO UPDATE SET
              position = excluded.position,
              duration = CASE WHEN excluded.duration > 0 THEN excluded.duration ELSE progress.duration END,
              finished = excluded.finished,
              updated_at = excluded.updated_at
            "#,
            params![item_id, position, duration, i64::from(finished), now()],
        )?;
        Ok(())
    }

    // ---------- 设置 ----------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let v = conn
            .query_row("SELECT value FROM setting WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .ok();
        Ok(v)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO setting(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// 需要补探测的条目（时长未知），后台队列用。
    pub fn pending_probe(&self, limit: i64) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, path FROM item WHERE probed = 0 AND kind = 'video' ORDER BY added_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn save_probe(&self, item_id: i64, width: i64, height: i64, duration: f64, vcodec: &str, acodec: &str, container: &str) -> Result<()> {
        let vertical = i64::from(is_vertical(width as u32, height as u32));
        let conn = self.conn();
        conn.execute(
            r#"UPDATE item SET duration = ?2, width = ?3, height = ?4, vertical = ?5,
                   vcodec = ?6, acodec = ?7, container = ?8, probed = 1 WHERE id = ?1"#,
            params![item_id, duration, width, height, vertical, vcodec, acodec, container],
        )?;
        if vertical == 1 {
            conn.execute(
                "UPDATE series SET kind = 'short'
                 WHERE id = (SELECT series_id FROM item WHERE id = ?1)
                   AND kind = 'tv'",
                params![item_id],
            )?;
        }
        Ok(())
    }

    pub fn thumb_path(&self, item_id: i64) -> PathBuf {
        self.thumbs_dir.join(format!("{item_id}.png"))
    }

    /// 给某个剧集找一张可用的封面（优先同目录 poster，其次首集缩略图）。
    pub fn poster_of(&self, series_id: i64) -> Option<PathBuf> {
        let conn = self.conn();
        let poster: Option<String> = conn
            .query_row("SELECT poster FROM series WHERE id = ?1", params![series_id], |r| r.get(0))
            .ok()
            .flatten();
        if let Some(p) = poster {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some(path);
            }
        }
        let first: Option<i64> = conn
            .query_row(
                "SELECT id FROM item WHERE series_id = ?1
                 ORDER BY season IS NULL, season, episode IS NULL, episode LIMIT 1",
                params![series_id],
                |r| r.get(0),
            )
            .ok();
        drop(conn);
        first.map(|id| self.thumb_path(id))
    }
}

pub fn is_vertical(width: u32, height: u32) -> bool {
    xv_media::is_vertical(width, height)
}

fn map_series(r: &Row) -> rusqlite::Result<SeriesCard> {
    Ok(SeriesCard {
        id: r.get(0)?,
        title: r.get(1)?,
        kind: r.get(2)?,
        year: r.get(3)?,
        total: r.get(4)?,
        added_at: r.get(5)?,
        updated_at: r.get(6)?,
        watched: r.get(7)?,
        last_at: r.get(8)?,
    })
}

fn map_item(r: &Row) -> rusqlite::Result<ItemRow> {
    Ok(ItemRow {
        id: r.get(0)?,
        series_id: r.get(1)?,
        name: r.get(2)?,
        path: r.get(3)?,
        episode: r.get(4)?,
        season: r.get(5)?,
        duration: r.get(6)?,
        width: r.get(7)?,
        height: r.get(8)?,
        vertical: r.get::<_, i64>(9)? != 0,
        size: r.get(10)?,
        added_at: r.get(11)?,
        position: r.get(12)?,
        finished: r.get::<_, i64>(13)? != 0,
        vcodec: r.get(14)?,
        acodec: r.get(15)?,
        container: r.get(16)?,
    })
}
