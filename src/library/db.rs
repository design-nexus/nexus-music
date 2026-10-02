//! SQLite storage. Every call opens its own connection, so it can run on any
//! thread; WAL mode keeps readers and the scanner out of each other's way.

use super::Track;
use crate::paths;
use anyhow::Result;
use rusqlite::{Connection, Transaction, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tracks (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    album_artist TEXT NOT NULL,
    genre TEXT NOT NULL,
    year INTEGER,
    track_no INTEGER,
    disc_no INTEGER,
    duration REAL NOT NULL,
    rg_track_gain REAL,
    rg_track_peak REAL,
    rg_album_gain REAL,
    rg_album_peak REAL,
    art TEXT NOT NULL DEFAULT '',
    plays INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER NOT NULL DEFAULT 0,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    created INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    pos INTEGER NOT NULL,
    path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS playlist_items_by_list ON playlist_items(playlist_id, pos);
";

pub fn open() -> Result<Connection> {
    open_at(&paths::library_db())
}

pub fn open_at(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn load_all(conn: &Connection) -> Result<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, title, artist, album, album_artist, genre, year, track_no, disc_no, duration,
                rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak, art, plays, last_played, mtime, size
         FROM tracks",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Track {
            id: r.get(0)?,
            path: PathBuf::from(r.get::<_, String>(1)?),
            title: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            album_artist: r.get(5)?,
            genre: r.get(6)?,
            year: r.get(7)?,
            track_no: r.get(8)?,
            disc_no: r.get(9)?,
            duration: r.get(10)?,
            rg_track_gain: r.get(11)?,
            rg_track_peak: r.get(12)?,
            rg_album_gain: r.get(13)?,
            rg_album_peak: r.get(14)?,
            art: r.get(15)?,
            plays: std::cell::Cell::new(r.get(16)?),
            last_played: r.get(17)?,
            mtime: r.get(18)?,
            size: r.get(19)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Path → (mtime, size) for every known file: what the scanner diffs against.
pub fn stamps(conn: &Connection) -> Result<HashMap<PathBuf, (i64, i64)>> {
    let mut stmt = conn.prepare("SELECT path, mtime, size FROM tracks")?;
    let rows = stmt.query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), (r.get(1)?, r.get(2)?))))?;
    Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

/// Insert or refresh a file's tags, keeping its play history.
pub fn upsert(tx: &Transaction, t: &Track) -> Result<()> {
    tx.execute(
        "INSERT INTO tracks (path, title, artist, album, album_artist, genre, year, track_no, disc_no, duration,
                             rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak, art, mtime, size)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(path) DO UPDATE SET
            title = excluded.title, artist = excluded.artist, album = excluded.album,
            album_artist = excluded.album_artist, genre = excluded.genre, year = excluded.year,
            track_no = excluded.track_no, disc_no = excluded.disc_no, duration = excluded.duration,
            rg_track_gain = excluded.rg_track_gain, rg_track_peak = excluded.rg_track_peak,
            rg_album_gain = excluded.rg_album_gain, rg_album_peak = excluded.rg_album_peak,
            art = excluded.art, mtime = excluded.mtime, size = excluded.size",
        params![
            path_text(&t.path),
            t.title,
            t.artist,
            t.album,
            t.album_artist,
            t.genre,
            t.year,
            t.track_no,
            t.disc_no,
            t.duration,
            t.rg_track_gain,
            t.rg_track_peak,
            t.rg_album_gain,
            t.rg_album_peak,
            t.art,
            t.mtime,
            t.size
        ],
    )?;
    Ok(())
}

pub fn delete(tx: &Transaction, path: &Path) -> Result<()> {
    tx.execute("DELETE FROM tracks WHERE path = ?1", [path_text(path)])?;
    Ok(())
}

pub fn count_play(conn: &Connection, path: &Path, when: i64) -> Result<()> {
    conn.execute("UPDATE tracks SET plays = plays + 1, last_played = ?2 WHERE path = ?1", params![path_text(path), when])?;
    Ok(())
}

// ---------- Playlists ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
}

pub fn playlists(conn: &Connection) -> Result<Vec<Playlist>> {
    let mut stmt = conn.prepare("SELECT id, name FROM playlists ORDER BY name COLLATE NOCASE, id")?;
    let rows = stmt.query_map([], |r| Ok(Playlist { id: r.get(0)?, name: r.get(1)? }))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn playlist_paths(conn: &Connection, id: i64) -> Result<Vec<PathBuf>> {
    let mut stmt = conn.prepare("SELECT path FROM playlist_items WHERE playlist_id = ?1 ORDER BY pos")?;
    let rows = stmt.query_map([id], |r| Ok(PathBuf::from(r.get::<_, String>(0)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn create_playlist(conn: &Connection, name: &str, items: &[PathBuf]) -> Result<i64> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    conn.execute("INSERT INTO playlists (name, created) VALUES (?1, ?2)", params![name, now])?;
    let id = conn.last_insert_rowid();
    set_playlist(conn, id, items)?;
    Ok(id)
}

pub fn rename_playlist(conn: &Connection, id: i64, name: &str) -> Result<()> {
    conn.execute("UPDATE playlists SET name = ?2 WHERE id = ?1", params![id, name])?;
    Ok(())
}

pub fn delete_playlist(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
    Ok(())
}

/// Replace a playlist's contents.
pub fn set_playlist(conn: &Connection, id: i64, items: &[PathBuf]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])?;
    {
        let mut stmt = tx.prepare("INSERT INTO playlist_items (playlist_id, pos, path) VALUES (?1, ?2, ?3)")?;
        for (i, p) in items.iter().enumerate() {
            stmt.execute(params![id, i as i64, path_text(p)])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn append_playlist(conn: &Connection, id: i64, items: &[PathBuf]) -> Result<()> {
    let mut all = playlist_paths(conn, id)?;
    all.extend(items.iter().cloned());
    set_playlist(conn, id, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Connection, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nexus-music-test-{}-{}", std::process::id(), rand_suffix()));
        let path = dir.join("t.db");
        (open_at(&path).unwrap(), dir)
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn upsert_keeps_play_counts() {
        let (mut conn, dir) = temp_db();
        let t = Track { path: "/m/a.flac".into(), title: "A".into(), mtime: 1, size: 2, ..Default::default() };
        let tx = conn.transaction().unwrap();
        upsert(&tx, &t).unwrap();
        tx.commit().unwrap();
        count_play(&conn, &t.path, 99).unwrap();
        let tx = conn.transaction().unwrap();
        upsert(&tx, &Track { title: "A2".into(), mtime: 5, ..t.clone() }).unwrap();
        tx.commit().unwrap();
        let all = load_all(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "A2");
        assert_eq!(all[0].plays.get(), 1);
        assert_eq!(all[0].last_played, 99);
        assert_eq!(stamps(&conn).unwrap()[&t.path], (5, 2));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn playlists_round_trip() {
        let (conn, dir) = temp_db();
        let id = create_playlist(&conn, "Road trip", &["/a.mp3".into(), "/b.mp3".into()]).unwrap();
        append_playlist(&conn, id, &["/c.mp3".into()]).unwrap();
        assert_eq!(playlist_paths(&conn, id).unwrap(), vec![PathBuf::from("/a.mp3"), "/b.mp3".into(), "/c.mp3".into()]);
        rename_playlist(&conn, id, "Drive").unwrap();
        assert_eq!(playlists(&conn).unwrap(), vec![Playlist { id, name: "Drive".into() }]);
        delete_playlist(&conn, id).unwrap();
        assert!(playlists(&conn).unwrap().is_empty());
        assert!(playlist_paths(&conn, id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
