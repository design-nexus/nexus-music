//! Incremental library scan on a worker thread: only new or changed files are
//! read, and files that disappeared are dropped (unless their whole folder is
//! missing, which usually means an unplugged drive).

use super::{Track, art, db, is_audio, tags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum Progress {
    /// Files found; `total` of them need reading.
    Reading {
        done: usize,
        total: usize,
    },
    /// `stamp` is the newest folder time seen, for change watching.
    Finished {
        added: usize,
        updated: usize,
        removed: usize,
        stamp: i64,
    },
    Failed(String),
}

pub type Stamp = (i64, i64);

/// What changed between the files on disk and the database.
#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub new: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

pub fn diff(on_disk: &[(PathBuf, Stamp)], known: &HashMap<PathBuf, Stamp>, roots: &[PathBuf]) -> Diff {
    let mut d = Diff::default();
    let mut seen = HashSet::new();
    for (path, stamp) in on_disk {
        seen.insert(path);
        match known.get(path) {
            None => d.new.push(path.clone()),
            Some(k) if k != stamp => d.changed.push(path.clone()),
            _ => {}
        }
    }
    let present: Vec<&PathBuf> = roots.iter().filter(|r| r.is_dir()).collect();
    for path in known.keys() {
        if seen.contains(path) {
            continue;
        }
        // Only forget files under a root that's still there, and drop files whose
        // root was removed from the library altogether.
        let under_present = present.iter().any(|r| path.starts_with(r));
        let under_any = roots.iter().any(|r| path.starts_with(r));
        if under_present || !under_any {
            d.removed.push(path.clone());
        }
    }
    d.new.sort();
    d.changed.sort();
    d.removed.sort();
    d
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some((mtime, meta.len() as i64))
}

pub fn walk(roots: &[PathBuf]) -> Vec<(PathBuf, Stamp)> {
    let mut out = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(root).follow_links(true).into_iter().filter_entry(|e| {
            // Skip hidden folders (.Trash, .cache…), but never the root itself.
            e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
        }) {
            let Ok(entry) = entry else { continue };
            if entry.file_type().is_file()
                && is_audio(entry.path())
                && let Some(s) = stamp(entry.path())
            {
                out.push((entry.into_path(), s));
            }
        }
    }
    out
}

/// The newest folder modification time under the roots: a cheap way to notice
/// files being added, removed or renamed.
pub fn folder_stamp(roots: &[PathBuf]) -> i64 {
    let mut newest = 0;
    for root in roots {
        for entry in walkdir::WalkDir::new(root).follow_links(true).into_iter().filter_entry(|e| e.file_type().is_dir()).flatten()
        {
            if let Ok(m) = entry.metadata()
                && let Ok(t) = m.modified()
                && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
            {
                newest = newest.max(d.as_secs() as i64);
            }
        }
    }
    newest
}

pub fn run(roots: Vec<PathBuf>, send: impl Fn(Progress)) {
    if let Err(e) = run_inner(&roots, &send) {
        send(Progress::Failed(e.to_string()));
    }
}

fn run_inner(roots: &[PathBuf], send: &impl Fn(Progress)) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    let stamp = folder_stamp(roots);
    let known = db::stamps(&conn)?;
    let on_disk = walk(roots);
    let d = diff(&on_disk, &known, roots);
    let todo: Vec<&PathBuf> = d.new.iter().chain(d.changed.iter()).collect();
    let total = todo.len();
    send(Progress::Reading { done: 0, total });

    let mut covers: HashMap<String, String> = HashMap::new();
    for (i, chunk) in todo.chunks(200).enumerate() {
        let tracks: Vec<Track> = chunk
            .iter()
            .map(|p| {
                let mut t = tags::read(p);
                let album = t.album_key();
                t.art = covers.entry(album).or_insert_with(|| art::ensure(&t)).clone();
                t
            })
            .collect();
        let tx = conn.transaction()?;
        for t in &tracks {
            db::upsert(&tx, t)?;
        }
        tx.commit()?;
        send(Progress::Reading { done: (i * 200 + chunk.len()).min(total), total });
    }
    if !d.removed.is_empty() {
        let tx = conn.transaction()?;
        for p in &d.removed {
            db::delete(&tx, p)?;
        }
        tx.commit()?;
    }
    send(Progress::Finished { added: d.new.len(), updated: d.changed.len(), removed: d.removed.len(), stamp });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_new_changed_and_removed() {
        let dir = std::env::temp_dir();
        let root = dir.clone();
        let a = root.join("a.mp3");
        let b = root.join("b.mp3");
        let c = root.join("c.mp3");
        let on_disk = vec![(a.clone(), (1, 10)), (b.clone(), (2, 20))];
        let known: HashMap<PathBuf, Stamp> = [(b.clone(), (1, 20)), (c.clone(), (1, 1))].into_iter().collect();
        let d = diff(&on_disk, &known, &[root]);
        assert_eq!(d.new, vec![a]);
        assert_eq!(d.changed, vec![b]);
        assert_eq!(d.removed, vec![c]);
    }

    #[test]
    fn keeps_files_from_a_missing_root() {
        let gone = PathBuf::from("/nonexistent-music-drive");
        let known: HashMap<PathBuf, Stamp> = [(gone.join("x.flac"), (1, 1))].into_iter().collect();
        let d = diff(&[], &known, std::slice::from_ref(&gone));
        assert!(d.removed.is_empty());
    }

    #[test]
    fn drops_files_from_a_removed_root() {
        let known: HashMap<PathBuf, Stamp> = [(PathBuf::from("/old/x.flac"), (1, 1))].into_iter().collect();
        let d = diff(&[], &known, &[std::env::temp_dir()]);
        assert_eq!(d.removed, vec![PathBuf::from("/old/x.flac")]);
    }
}
